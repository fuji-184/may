use std::io;
use std::net::{self, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
#[cfg(feature = "io_timeout")]
use std::time::Duration;

use crate::io as io_impl;
use crate::io::net as net_impl;
#[cfg(feature = "io_timeout")]
use crate::sync::atomic_dur::AtomicDuration;
use crate::yield_now::yield_with_io;

#[derive(Debug)]
pub struct UdpSocket {
    _io: io_impl::IoData,
    sys: net::UdpSocket,
    #[cfg(feature = "io_timeout")]
    read_timeout: AtomicDuration,
    #[cfg(feature = "io_timeout")]
    write_timeout: AtomicDuration,
}

impl UdpSocket {
    fn new(s: net::UdpSocket) -> io::Result<UdpSocket> {
        // only set non blocking in coroutine context
        // we would first call nonblocking io in the coroutine
        // to avoid unnecessary context switch
        s.set_nonblocking(true)?;

        io_impl::add_socket(&s).map(|io| UdpSocket {
            _io: io,
            sys: s,
            #[cfg(feature = "io_timeout")]
            read_timeout: AtomicDuration::new(None),
            #[cfg(feature = "io_timeout")]
            write_timeout: AtomicDuration::new(None),
        })
    }

    pub fn inner(&self) -> &net::UdpSocket {
        &self.sys
    }

    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<UdpSocket> {
        net::UdpSocket::bind(addr).and_then(UdpSocket::new)
    }

    pub fn connect<A: ToSocketAddrs>(&self, addr: A) -> io::Result<()> {
        // for udp connect it's a nonblocking operation
        // so we just use the system call
        self.sys.connect(addr)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sys.local_addr()
    }

    #[cfg(not(windows))]
    pub fn try_clone(&self) -> io::Result<UdpSocket> {
        let s = self.sys.try_clone().and_then(UdpSocket::new)?;
        #[cfg(feature = "io_timeout")]
        s.set_read_timeout(self.read_timeout.get()).unwrap();
        #[cfg(feature = "io_timeout")]
        s.set_write_timeout(self.write_timeout.get()).unwrap();
        Ok(s)
    }

    // windows doesn't support add dup handler to IOCP
    #[cfg(windows)]
    pub fn try_clone(&self) -> io::Result<UdpSocket> {
        let s = self.sys.try_clone()?;
        s.set_nonblocking(true)?;
        io_impl::add_socket(&s).ok();
        Ok(UdpSocket {
            _io: io_impl::IoData::new(0),
            sys: s,
            #[cfg(feature = "io_timeout")]
            read_timeout: AtomicDuration::new(self.read_timeout.get()),
            #[cfg(feature = "io_timeout")]
            write_timeout: AtomicDuration::new(self.write_timeout.get()),
        })
    }

    pub fn send_to<A: ToSocketAddrs>(&self, buf: &[u8], addr: A) -> io::Result<usize> {
        #[cfg(unix)]
        {
            self._io.reset();
            // this is an earlier return try for nonblocking read
            match self.sys.send_to(buf, &addr) {
                Ok(n) => return Ok(n),
                Err(e) => {
                    // raw_os_error is faster than kind
                    let raw_err = e.raw_os_error();
                    if raw_err == Some(libc::EAGAIN) || raw_err == Some(libc::EWOULDBLOCK) {
                        // do nothing here
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        let mut writer = net_impl::UdpSendTo::new(self, buf, addr)?;
        yield_with_io(&writer, writer.is_coroutine);
        writer.done()
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        #[cfg(unix)]
        {
            self._io.reset();
            // this is an earlier return try for nonblocking read
            match self.sys.recv_from(buf) {
                Ok(n) => return Ok(n),
                Err(e) => {
                    // raw_os_error is faster than kind
                    let raw_err = e.raw_os_error();
                    if raw_err == Some(libc::EAGAIN) || raw_err == Some(libc::EWOULDBLOCK) {
                        // do nothing here
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        let mut reader = net_impl::UdpRecvFrom::new(self, buf);
        yield_with_io(&reader, reader.is_coroutine);
        reader.done()
    }

    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            self._io.reset();
            // this is an earlier return try for nonblocking write
            match self.sys.send(buf) {
                Ok(n) => return Ok(n),
                Err(e) => {
                    // raw_os_error is faster than kind
                    let raw_err = e.raw_os_error();
                    if raw_err == Some(libc::EAGAIN) || raw_err == Some(libc::EWOULDBLOCK) {
                        // do nothing here
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        let mut writer = net_impl::SocketWrite::new(
            self,
            buf,
            #[cfg(feature = "io_timeout")]
            self.write_timeout.get(),
        );
        yield_with_io(&writer, writer.is_coroutine);
        writer.done()
    }

    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            self._io.reset();
            // this is an earlier return try for nonblocking read
            match self.sys.recv(buf) {
                Ok(n) => return Ok(n),
                Err(e) => {
                    // raw_os_error is faster than kind
                    let raw_err = e.raw_os_error();
                    if raw_err == Some(libc::EAGAIN) || raw_err == Some(libc::EWOULDBLOCK) {
                        // do nothing here
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        let mut reader = net_impl::SocketRead::new(
            self,
            buf,
            #[cfg(feature = "io_timeout")]
            self.read_timeout.get(),
        );
        yield_with_io(&reader, reader.is_coroutine);
        reader.done()
    }

    #[cfg(feature = "io_timeout")]
    pub fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.sys.set_read_timeout(dur)?;
        self.read_timeout.store(dur);
        Ok(())
    }

    #[cfg(feature = "io_timeout")]
    pub fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.sys.set_write_timeout(dur)?;
        self.write_timeout.store(dur);
        Ok(())
    }

    #[cfg(feature = "io_timeout")]
    pub fn read_timeout(&self) -> io::Result<Option<Duration>> {
        Ok(self.read_timeout.get())
    }

    #[cfg(feature = "io_timeout")]
    pub fn write_timeout(&self) -> io::Result<Option<Duration>> {
        Ok(self.write_timeout.get())
    }

    pub fn broadcast(&self) -> io::Result<bool> {
        self.sys.broadcast()
    }

    pub fn set_broadcast(&self, on: bool) -> io::Result<()> {
        self.sys.set_broadcast(on)
    }

    pub fn multicast_loop_v4(&self) -> io::Result<bool> {
        self.sys.multicast_loop_v4()
    }

    pub fn set_multicast_loop_v4(&self, on: bool) -> io::Result<()> {
        self.sys.set_multicast_loop_v4(on)
    }

    pub fn multicast_ttl_v4(&self) -> io::Result<u32> {
        self.sys.multicast_ttl_v4()
    }

    pub fn set_multicast_ttl_v4(&self, ttl: u32) -> io::Result<()> {
        self.sys.set_multicast_ttl_v4(ttl)
    }

    pub fn multicast_loop_v6(&self) -> io::Result<bool> {
        self.sys.multicast_loop_v6()
    }

    pub fn set_multicast_loop_v6(&self, on: bool) -> io::Result<()> {
        self.sys.set_multicast_loop_v6(on)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        self.sys.ttl()
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.sys.set_ttl(ttl)
    }

    pub fn join_multicast_v4(&self, multiaddr: &Ipv4Addr, interface: &Ipv4Addr) -> io::Result<()> {
        self.sys.join_multicast_v4(multiaddr, interface)
    }

    pub fn join_multicast_v6(&self, multiaddr: &Ipv6Addr, interface: u32) -> io::Result<()> {
        self.sys.join_multicast_v6(multiaddr, interface)
    }

    pub fn leave_multicast_v4(&self, multiaddr: &Ipv4Addr, interface: &Ipv4Addr) -> io::Result<()> {
        self.sys.leave_multicast_v4(multiaddr, interface)
    }

    pub fn leave_multicast_v6(&self, multiaddr: &Ipv6Addr, interface: u32) -> io::Result<()> {
        self.sys.leave_multicast_v6(multiaddr, interface)
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.sys.take_error()
    }
}

#[cfg(unix)]
impl io_impl::AsIoData for UdpSocket {
    fn as_io_data(&self) -> &io_impl::IoData {
        &self._io
    }
}

// ===== UNIX ext =====
//
//

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

#[cfg(unix)]
impl IntoRawFd for UdpSocket {
    fn into_raw_fd(self) -> RawFd {
        self.sys.into_raw_fd()
        // drop self will dereg from the selector
    }
}

#[cfg(unix)]
impl AsRawFd for UdpSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.sys.as_raw_fd()
    }
}

#[cfg(unix)]
impl FromRawFd for UdpSocket {
    unsafe fn from_raw_fd(fd: RawFd) -> UdpSocket {
        UdpSocket::new(FromRawFd::from_raw_fd(fd))
            .unwrap_or_else(|e| panic!("from_raw_socket for UdpSocket, err = {e:?}"))
    }
}

// ===== Windows ext =====
//
//

#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket, IntoRawSocket, RawSocket};

#[cfg(windows)]
impl IntoRawSocket for UdpSocket {
    fn into_raw_socket(self) -> RawSocket {
        self.sys.into_raw_socket()
    }
}

#[cfg(windows)]
impl AsRawSocket for UdpSocket {
    fn as_raw_socket(&self) -> RawSocket {
        self.sys.as_raw_socket()
    }
}

#[cfg(windows)]
impl FromRawSocket for UdpSocket {
    unsafe fn from_raw_socket(s: RawSocket) -> UdpSocket {
        // TODO: set the time out info here
        // need to set the read/write timeout from sys and sync each other
        UdpSocket::new(FromRawSocket::from_raw_socket(s))
            .unwrap_or_else(|e| panic!("from_raw_socket for UdpSocket, err = {e:?}"))
    }
}

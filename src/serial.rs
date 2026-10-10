//! Caller connections: a telnet socket or a serial line (the COM ports
//! TMPCOM.PAS drove). Serial ports each get a thread that waits for a key,
//! claims a node and runs a session, then reopens the port for the next caller.

use crate::config::SerialPort;
use crate::session::Shared;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub enum Link {
    Tcp(TcpStream),
    /// The port and a hang-up flag shared with its clones.
    Serial(File, Arc<AtomicBool>, u32),
}

impl Link {
    pub fn telnet(&self) -> bool {
        matches!(self, Link::Tcp(_))
    }

    /// Line speed, 0 for telnet.
    pub fn baud(&self) -> u32 {
        match self {
            Link::Serial(_, _, b) => *b,
            Link::Tcp(_) => 0,
        }
    }

    pub fn try_clone(&self) -> io::Result<Link> {
        Ok(match self {
            Link::Tcp(s) => Link::Tcp(s.try_clone()?),
            Link::Serial(f, h, b) => Link::Serial(f.try_clone()?, h.clone(), *b),
        })
    }

    /// Drop the caller (console `K`, shutdown).
    pub fn hangup(&self) {
        match self {
            Link::Tcp(s) => drop(s.shutdown(std::net::Shutdown::Both)),
            Link::Serial(_, h, _) => h.store(true, Ordering::Relaxed),
        }
    }

    fn fd(&self) -> RawFd {
        match self {
            Link::Tcp(s) => s.as_raw_fd(),
            Link::Serial(f, ..) => f.as_raw_fd(),
        }
    }

    /// Wait up to `wait` for input; true when a read won't block (data or EOF).
    pub fn wait(&self, wait: Duration) -> io::Result<bool> {
        let end = Instant::now() + wait;
        loop {
            if let Link::Serial(_, h, _) = self
                && h.load(Ordering::Relaxed)
            {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            // ponytail: serial polls in 250ms slices so a console hang-up is noticed
            let slice = end.saturating_duration_since(Instant::now()).min(Duration::from_millis(250));
            let mut p = libc::pollfd { fd: self.fd(), events: libc::POLLIN, revents: 0 };
            match unsafe { libc::poll(&mut p, 1, slice.as_millis() as libc::c_int) } {
                n if n > 0 => return Ok(true),
                0 if Instant::now() >= end => return Ok(false),
                0 => {}
                _ => {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
            }
        }
    }
}

impl Read for Link {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match self {
            Link::Tcp(s) => s.read(b),
            Link::Serial(f, h, _) if !h.load(Ordering::Relaxed) => f.read(b),
            Link::Serial(..) => Ok(0),
        }
    }
}

impl Write for Link {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Link::Tcp(s) => s.write(b),
            Link::Serial(_, h, _) if h.load(Ordering::Relaxed) => Err(io::ErrorKind::BrokenPipe.into()),
            Link::Serial(f, ..) => f.write(b),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Open `p` raw at its baud rate (8N1, no flow control, ignoring carrier).
pub fn open(p: &SerialPort) -> io::Result<Link> {
    let speed = match p.baud {
        300 => libc::B300,
        1200 => libc::B1200,
        2400 => libc::B2400,
        4800 => libc::B4800,
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        b => return Err(io::Error::other(format!("unsupported baud rate {b}"))),
    };
    // O_NONBLOCK so the open doesn't wait for carrier; cleared below
    let f = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK).open(&p.device)?;
    let fd = f.as_raw_fd();
    let ok = |r: libc::c_int| if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) };
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        ok(libc::tcgetattr(fd, &mut t))?;
        libc::cfmakeraw(&mut t);
        t.c_cflag |= libc::CLOCAL | libc::CREAD | libc::HUPCL;
        t.c_cflag &= !(libc::CRTSCTS | libc::CSTOPB | libc::PARENB);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        ok(libc::cfsetispeed(&mut t, speed))?;
        ok(libc::cfsetospeed(&mut t, speed))?;
        ok(libc::tcsetattr(fd, libc::TCSANOW, &t))?;
        ok(libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK))?;
        libc::tcflush(fd, libc::TCIOFLUSH);
    }
    Ok(Link::Serial(f, Default::default(), p.baud))
}

/// Answer `p` forever: a key wakes it, then the caller gets a node.
pub fn spawn(sh: Arc<Shared>, p: SerialPort) {
    std::thread::spawn(move || {
        loop {
            let mut link = match open(&p) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Serial {}: {e}; retrying in 30s", p.device.display());
                    std::thread::sleep(Duration::from_secs(30));
                    continue;
                }
            };
            // ponytail: direct-connect terminals only; no modem init/ATA or DCD wait
            let _ = link.write_all(b"\r\nPress Enter to connect.\r\n");
            let mut b = [0];
            if link.read(&mut b).map_or(true, |n| n == 0) {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            match crate::claim_node(&sh, &link) {
                Some(node) => crate::session::run(sh.clone(), link, p.device.display().to_string(), node),
                None => {
                    let _ = link.write_all(b"All nodes are busy, please call back later.\r\n");
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_port() {
        // a pty stands in for the serial line
        let (mut m, mut s) = (0, 0);
        assert_eq!(unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) }, 0);
        let name = unsafe { std::ffi::CStr::from_ptr(libc::ttyname(s)) }.to_str().unwrap().to_string();
        let mut master = unsafe { <File as std::os::fd::FromRawFd>::from_raw_fd(m) };
        let mut link = open(&SerialPort { device: name.into(), baud: 9600 }).unwrap();
        assert!(!link.wait(Duration::from_millis(50)).unwrap());
        master.write_all(b"hi").unwrap();
        assert!(link.wait(Duration::from_secs(1)).unwrap());
        let mut b = [0; 2];
        link.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"hi");
        link.write_all(b"\xff").unwrap(); // raw: no telnet escaping, no output processing
        master.read_exact(&mut b[..1]).unwrap();
        assert_eq!(b[0], 0xff);
        link.try_clone().unwrap().hangup();
        assert!(link.wait(Duration::from_secs(1)).is_err());
        assert!(open(&SerialPort { device: "/dev/null".into(), baud: 1234 }).is_err());
        unsafe { libc::close(s) };
    }
}

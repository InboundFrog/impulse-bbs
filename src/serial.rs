//! Caller connections: a telnet socket or a serial line (the COM ports
//! TMPCOM.PAS drove). Serial ports each get a thread that waits for a caller
//! (a key on a direct line, or RING / answer / CONNECT on a dial-in modem,
//! MMODEM.PAS), claims a node and runs a session, then re-arms the port.

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
    Serial(Line),
}

pub struct Line {
    f: File,
    /// Hang-up flag shared with clones (console `K`).
    hup: Arc<AtomicBool>,
    /// Line speed: the port rate, or the modem's CONNECT rate.
    baud: u32,
    /// A dropped DCD ends the call.
    dcd: bool,
}

impl Line {
    fn clone_as(&self, baud: u32, dcd: bool) -> io::Result<Line> {
        Ok(Line { f: self.f.try_clone()?, hup: self.hup.clone(), baud, dcd })
    }

    fn modem_bits(&self, req: libc::c_ulong, bits: libc::c_int) -> io::Result<libc::c_int> {
        let mut b = bits;
        if unsafe { libc::ioctl(self.f.as_raw_fd(), req, &mut b) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(b)
    }

    /// False once carrier drops. ponytail: a port without modem lines (a pty)
    /// can't tell, so it counts as connected.
    fn carrier(&self) -> bool {
        self.modem_bits(libc::TIOCMGET, 0).map_or(true, |b| b & libc::TIOCM_CAR != 0)
    }

    fn dtr(&self, on: bool) {
        let _ = self.modem_bits(if on { libc::TIOCMBIS } else { libc::TIOCMBIC }, libc::TIOCM_DTR);
    }
}

impl Link {
    pub fn telnet(&self) -> bool {
        matches!(self, Link::Tcp(_))
    }

    /// Line speed, 0 for telnet.
    pub fn baud(&self) -> u32 {
        match self {
            Link::Serial(l) => l.baud,
            Link::Tcp(_) => 0,
        }
    }

    pub fn try_clone(&self) -> io::Result<Link> {
        Ok(match self {
            Link::Tcp(s) => Link::Tcp(s.try_clone()?),
            Link::Serial(l) => Link::Serial(l.clone_as(l.baud, l.dcd)?),
        })
    }

    /// Drop the caller (console `K`, shutdown).
    pub fn hangup(&self) {
        match self {
            Link::Tcp(s) => drop(s.shutdown(std::net::Shutdown::Both)),
            Link::Serial(l) => l.hup.store(true, Ordering::Relaxed),
        }
    }

    fn fd(&self) -> RawFd {
        match self {
            Link::Tcp(s) => s.as_raw_fd(),
            Link::Serial(l) => l.f.as_raw_fd(),
        }
    }

    /// Wait up to `wait` for input; true when a read won't block (data or EOF).
    pub fn wait(&self, wait: Duration) -> io::Result<bool> {
        let end = Instant::now() + wait;
        loop {
            if let Link::Serial(l) = self
                && (l.hup.load(Ordering::Relaxed) || (l.dcd && !l.carrier()))
            {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            // ponytail: serial polls in 250ms slices so a hang-up or lost carrier is noticed
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
            Link::Serial(l) if !l.hup.load(Ordering::Relaxed) => l.f.read(b),
            Link::Serial(_) => Ok(0),
        }
    }
}

impl Write for Link {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Link::Tcp(s) => s.write(b),
            Link::Serial(l) if l.hup.load(Ordering::Relaxed) => Err(io::ErrorKind::BrokenPipe.into()),
            Link::Serial(l) => l.f.write(b),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Open `p` raw at its baud rate, 8N1, ignoring carrier until a call is up.
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
        if p.rts_cts {
            t.c_cflag |= libc::CRTSCTS;
        }
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        ok(libc::cfsetispeed(&mut t, speed))?;
        ok(libc::cfsetospeed(&mut t, speed))?;
        ok(libc::tcsetattr(fd, libc::TCSANOW, &t))?;
        ok(libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK))?;
        libc::tcflush(fd, libc::TCIOFLUSH);
    }
    Ok(Link::Serial(Line { f, hup: Default::default(), baud: p.baud, dcd: false }))
}

/// Send a modem string (outmodemstring000): `~` pauses half a second,
/// `|` is a CR, and a CR ends it.
fn modem_send(l: &mut Link, s: &str) -> io::Result<()> {
    for c in s.chars() {
        match c {
            '~' => std::thread::sleep(Duration::from_millis(500)),
            '|' => l.write_all(b"\r")?,
            c => {
                l.write_all(c.to_string().as_bytes())?;
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    l.write_all(b"\r")
}

/// Next non-empty result line from the modem, None after `wait` of silence.
fn modem_line(l: &mut Link, wait: Duration) -> io::Result<Option<String>> {
    let mut s = Vec::new();
    while s.len() < 256 && l.wait(wait)? {
        let mut b = [0];
        if l.read(&mut b)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        match b[0] {
            b'\r' | b'\n' if s.is_empty() => {}
            b'\r' | b'\n' => return Ok(Some(String::from_utf8_lossy(&s).trim().to_string())),
            c => s.push(c),
        }
    }
    Ok((!s.is_empty()).then(|| String::from_utf8_lossy(&s).trim().to_string()))
}

/// Send `cmd`, then wait for a line starting with one of `want`.
fn modem_cmd(l: &mut Link, cmd: &str, want: &[&str], wait: Duration) -> io::Result<Option<String>> {
    if !cmd.is_empty() {
        modem_send(l, cmd)?;
    }
    let end = Instant::now() + wait;
    while let Some(r) = modem_line(l, end.saturating_duration_since(Instant::now()))? {
        if Instant::now() > end {
            break;
        }
        if want.iter().any(|w| r.starts_with(w)) {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

/// The rate in `CONNECT 14400/ARQ`; bare `CONNECT` means 300 baud.
fn connect_baud(r: &str) -> u32 {
    let n: String = r.trim_start_matches("CONNECT").trim().chars().take_while(char::is_ascii_digit).collect();
    n.parse().unwrap_or(300)
}

/// Console `O` (WFC "take off hook"): send `offhook` and stay busy until cleared.
fn busy(sh: &Shared, p: &SerialPort, l: &mut Link) -> io::Result<()> {
    modem_cmd(l, &p.offhook, &["OK"], Duration::from_secs(3))?;
    while sh.offhook.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

/// Wait for a call on a dial-in modem: init, `rings` RINGs, answer, CONNECT.
/// Caller ID lines (NMBR = / NAME =) ride along with the CONNECT result.
/// Ok(None) asks for a re-init (no call in `init_minutes`, init failed, off hook).
fn answer(sh: &Shared, p: &SerialPort, l: &mut Link) -> io::Result<Option<(Line, String)>> {
    if sh.offhook.load(Ordering::Relaxed) {
        busy(sh, p, l)?;
        return Ok(None);
    }
    if modem_cmd(l, &p.init, &["OK"], Duration::from_secs(5))?.is_none() {
        eprintln!("Serial {}: modem didn't answer init {:?}; retrying in 30s", p.device.display(), p.init);
        std::thread::sleep(Duration::from_secs(30));
        return Ok(None);
    }
    let reinit = (p.init_minutes > 0).then(|| Instant::now() + Duration::from_secs(p.init_minutes * 60));
    let (mut rings, mut last_ring, mut cid) = (0, Instant::now(), Vec::new());
    let r = loop {
        if sh.offhook.load(Ordering::Relaxed) {
            busy(sh, p, l)?;
            return Ok(None);
        }
        if reinit.is_some_and(|t| Instant::now() >= t) {
            return Ok(None);
        }
        if rings > 0 && last_ring.elapsed() > Duration::from_secs(10) {
            (rings, cid) = (0, Vec::new()); // the caller gave up
        }
        let Some(r) = modem_line(l, Duration::from_secs(1))? else { continue };
        if r.starts_with("RING") {
            (rings, last_ring) = (rings + 1, Instant::now());
            if rings >= p.rings.max(1) {
                break modem_cmd(l, &p.answer, &["CONNECT", "NO CARRIER", "BUSY", "ERROR", "NO ANSWER"], Duration::from_secs(60))?;
            }
        } else if r.starts_with("NMBR") || r.starts_with("NAME") {
            cid.push(r);
        } else if r.starts_with("CONNECT") {
            break Some(r); // the modem auto-answered (S0>0)
        }
    };
    match (r, l) {
        (Some(r), Link::Serial(line)) if r.starts_with("CONNECT") => {
            let baud = connect_baud(&r);
            cid.insert(0, r);
            Ok(Some((line.clone_as(baud, true)?, cid.join(", "))))
        }
        _ => Ok(None),
    }
}

/// Answer `p` forever.
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
            let mut peer = p.device.display().to_string();
            let call = if p.modem {
                match answer(&sh, &p, &mut link) {
                    Ok(Some((line, connect))) => {
                        peer = format!("{peer} ({connect})");
                        Link::Serial(line)
                    }
                    _ => continue,
                }
            } else {
                let _ = link.write_all(b"\r\nPress Enter to connect.\r\n");
                let mut b = [0];
                if link.read(&mut b).map_or(true, |n| n == 0) {
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
                match link.try_clone() {
                    Ok(l) => l,
                    Err(_) => continue,
                }
            };
            match crate::claim_node(&sh, &call) {
                Some(node) => crate::session::run(sh.clone(), call, peer, node),
                None => {
                    let mut call = call;
                    let _ = call.write_all(b"All nodes are busy, please call back later.\r\n");
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
            if p.modem
                && let Link::Serial(l) = &link
            {
                // dophonehangup: drop DTR, then the hangup string for modems that ignore it
                l.hup.store(false, Ordering::Relaxed);
                l.dtr(false);
                std::thread::sleep(Duration::from_secs(1));
                l.dtr(true);
                let _ = modem_cmd(&mut link, &p.hangup, &["OK", "0"], Duration::from_secs(3));
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
        let port = SerialPort { device: name.into(), ..Default::default() };
        let mut link = open(&port).unwrap();
        assert!(!link.wait(Duration::from_millis(50)).unwrap());
        master.write_all(b"hi").unwrap();
        assert!(link.wait(Duration::from_secs(1)).unwrap());
        let mut b = [0; 2];
        link.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"hi");
        link.write_all(b"\xff").unwrap(); // raw: no telnet escaping, no output processing
        master.read_exact(&mut b[..1]).unwrap();
        assert_eq!(b[0], 0xff);

        // modem chat: CR-terminated commands, result lines
        master.write_all(b"\r\nOK\r\n").unwrap();
        assert_eq!(modem_cmd(&mut link, "AT|Z", &["OK"], Duration::from_secs(1)).unwrap().as_deref(), Some("OK"));
        let mut b = [0; 5];
        master.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"AT\rZ\r");
        assert_eq!(connect_baud("CONNECT 14400/ARQ/V32"), 14400);
        assert_eq!(connect_baud("CONNECT"), 300);

        link.try_clone().unwrap().hangup();
        assert!(link.wait(Duration::from_secs(1)).is_err());
        assert!(open(&SerialPort { device: "/dev/null".into(), baud: 1234, ..Default::default() }).is_err());
        unsafe { libc::close(s) };
    }
}

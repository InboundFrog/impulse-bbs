//! Telnet transport and ANSI colour, standing in for the FOSSIL/COM layer
//! (TMPCOM.PAS) and `getc`/`setc` in COMMON.PAS.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

const IAC: u8 = 255;

pub struct Term {
    s: TcpStream,
    buf: Vec<u8>,
    last: u8,
    skip_lf: bool,
    /// Keys stuffed by a `-;` macro, returned before real input.
    pub stuffed: std::collections::VecDeque<char>,
    /// Current text attribute (DOS style: low nibble fg, high nibble bg, 0x80 blink).
    pub curco: u8,
    idle: Duration,
}

impl Term {
    pub fn new(s: TcpStream, idle: Duration) -> io::Result<Term> {
        s.set_read_timeout(Some(idle))?;
        s.set_nodelay(true)?;
        let mut t = Term { s, buf: Vec::new(), last: 0, skip_lf: false, stuffed: Default::default(), curco: 7, idle };
        // WILL ECHO + WILL SUPPRESS-GO-AHEAD: character mode, we do the echoing.
        t.buf.extend_from_slice(&[IAC, 251, 1, IAC, 251, 3]);
        Ok(t)
    }

    /// Queue one byte of output; bare LF becomes CRLF, IAC is escaped.
    pub fn put(&mut self, b: u8) {
        if b == b'\n' && self.last != b'\r' {
            self.buf.push(b'\r');
        }
        if b == IAC {
            self.buf.push(IAC);
        }
        self.buf.push(b);
        self.last = b;
    }

    pub fn put_str(&mut self, s: &str) {
        s.bytes().for_each(|b| self.put(b));
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            self.s.write_all(&self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }

    fn byte(&mut self) -> io::Result<u8> {
        let mut b = [0];
        loop {
            match self.s.read(&mut b) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) => return Ok(b[0]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Next key from the caller, with telnet negotiation stripped.
    /// Enter (CR, CR LF, CR NUL or bare LF) always comes back as '\r'.
    pub fn getkey(&mut self) -> io::Result<char> {
        self.flush()?;
        if let Some(c) = self.stuffed.pop_front() {
            return Ok(c);
        }
        loop {
            let b = self.byte()?;
            let skip = std::mem::take(&mut self.skip_lf);
            match b {
                IAC => match self.byte()? {
                    251..=254 => {
                        self.byte()?;
                    }
                    250 => while !(self.byte()? == IAC && self.byte()? == 240) {},
                    IAC => return Ok(IAC as char),
                    _ => {}
                },
                b'\n' | 0 if skip => {}
                b'\r' | b'\n' => {
                    self.skip_lf = b == b'\r';
                    return Ok('\r');
                }
                _ => return Ok(b as char),
            }
        }
    }
}

impl Term {
    /// Run `cmd` with stdin/stdout joined to the caller, telnet-escaped both
    /// ways (transfer protocols and doors; the FOSSIL handoff in EXEC.PAS).
    pub fn run_external(&mut self, cmd: &mut Command) -> io::Result<ExitStatus> {
        // WILL/DO BINARY so 8-bit transfers survive the client.
        self.buf.extend_from_slice(&[IAC, 251, 0, IAC, 253, 0]);
        self.flush()?;
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let mut cin = child.stdin.take();
        let mut cout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
        let mut sock = self.s.try_clone()?;
        let pump = std::thread::spawn(move || {
            let mut b = [0u8; 4096];
            while let Ok(n @ 1..) = cout.read(&mut b) {
                if sock.write_all(&escape_iac(&b[..n])).is_err() {
                    break;
                }
            }
        });
        self.s.set_read_timeout(Some(Duration::from_millis(100)))?;
        let mut state = Unescape::default();
        let mut b = [0u8; 4096];
        let r = loop {
            if let Some(st) = child.try_wait()? {
                break Ok(st);
            }
            match self.s.read(&mut b) {
                Ok(0) => {
                    let _ = child.kill();
                    break Err(io::ErrorKind::UnexpectedEof.into());
                }
                Ok(n) => {
                    let data = state.feed(&b[..n]);
                    if let Some(w) = cin.as_mut()
                        && w.write_all(&data).is_err() {
                            cin = None; // child closed stdin; keep waiting for it
                        }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
                Err(e) => {
                    let _ = child.kill();
                    break Err(e);
                }
            }
        };
        drop(cin);
        let _ = child.wait();
        let _ = pump.join();
        self.s.set_read_timeout(Some(self.idle))?;
        r
    }
}

fn escape_iac(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(b.len());
    for &x in b {
        v.push(x);
        if x == IAC {
            v.push(IAC);
        }
    }
    v
}

/// Strips telnet commands from incoming bytes, across reads.
#[derive(Default)]
struct Unescape {
    /// 0 data, 1 after IAC, 2 option byte to skip, 3 in SB, 4 IAC inside SB.
    st: u8,
}

impl Unescape {
    fn feed(&mut self, b: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        for &x in b {
            self.st = match (self.st, x) {
                (0, IAC) => 1,
                (0, _) => {
                    v.push(x);
                    0
                }
                (1, IAC) => {
                    v.push(IAC);
                    0
                }
                (1, 251..=254) => 2,
                (1, 250) => 3,
                (1 | 2, _) => 0,
                (3, IAC) => 4,
                (4, 240) => 0,
                (3 | 4, _) => 3,
                _ => 0,
            };
        }
        v
    }
}

/// ANSI sequence switching attribute `cur` to `c`; a port of `getc`.
pub fn ansi_color(cur: u8, c: u8) -> String {
    const X: &[u8; 8] = b"04261537";
    let (mut s, cur) = if (cur & !c) & 0x88 != 0 { (vec!["0".to_string()], 0x07) } else { (vec![], cur) };
    let n = s.len();
    if c & 7 != cur & 7 {
        s.push(format!("3{}", X[(c & 7) as usize] as char));
    }
    if c & 0x70 != cur & 0x70 {
        s.push(format!("4{}", X[(c >> 4 & 7) as usize] as char));
    }
    if c & 0x80 != 0 {
        s.push("5".into());
    }
    if c & 8 != 0 {
        s.push("1".into());
    }
    if s.len() == n {
        s.push(format!("3{}", X[(c & 7) as usize] as char));
    }
    format!("\x1b[{}m", s.join(";"))
}

#[cfg(test)]
mod tests {
    use super::ansi_color;

    #[test]
    fn telnet_escaping() {
        assert_eq!(super::escape_iac(&[1, 255, 2]), [1, 255, 255, 2]);
        let mut u = super::Unescape::default();
        // IAC IAC -> 255, IAC WILL x dropped, SB ... SE dropped, split across feeds
        assert_eq!(u.feed(&[1, 255, 255, 2, 255, 251]), [1, 255, 2]);
        assert_eq!(u.feed(&[0, 3, 255, 250, 24, 0, 255, 240, 4]), [3, 4]);
    }

    #[test]
    fn colors() {
        assert_eq!(ansi_color(7, 15), "\x1b[1m");
        assert_eq!(ansi_color(15, 7), "\x1b[0;37m");
        assert_eq!(ansi_color(7, 0x1e), "\x1b[33;44;1m");
        assert_eq!(ansi_color(7, 4), "\x1b[31m");
    }
}

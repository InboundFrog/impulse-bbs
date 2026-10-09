//! Telnet transport and ANSI colour, standing in for the FOSSIL/COM layer
//! (TMPCOM.PAS) and `getc`/`setc` in COMMON.PAS.

use std::io::{self, Read, Write};
use std::net::TcpStream;
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
}

impl Term {
    pub fn new(s: TcpStream, idle: Duration) -> io::Result<Term> {
        s.set_read_timeout(Some(idle))?;
        s.set_nodelay(true)?;
        let mut t = Term { s, buf: Vec::new(), last: 0, skip_lf: false, stuffed: Default::default(), curco: 7 };
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
    fn colors() {
        assert_eq!(ansi_color(7, 15), "\x1b[1m");
        assert_eq!(ansi_color(15, 7), "\x1b[0;37m");
        assert_eq!(ansi_color(7, 0x1e), "\x1b[33;44;1m");
        assert_eq!(ansi_color(7, 4), "\x1b[31m");
    }
}

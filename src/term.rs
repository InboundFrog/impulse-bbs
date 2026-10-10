//! Telnet/serial transport and ANSI colour, standing in for the FOSSIL/COM layer
//! (TMPCOM.PAS) and `getc`/`setc` in COMMON.PAS.

use crate::serial::Link;
use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

const IAC: u8 = 255;

/// What the console sees of a node: the local screen (`V`) and the
/// scrollback (SCRLBK.PAS, `B`).
#[derive(Default)]
pub struct Tap {
    pub back: std::collections::VecDeque<u8>,
    pub live: bool,
}

/// Scrollback kept per node, in bytes.
const BACK: usize = 32 * 1024;

/// CP437 bytes as Unicode, for showing a node on the console.
pub fn cp437(b: &[u8]) -> String {
    const HI: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";
    b.iter().map(|&c| if c < 128 { c as char } else { HI.chars().nth(usize::from(c - 128)).unwrap_or('?') }).collect()
}

pub struct Term {
    s: Link,
    /// Telnet IAC handling; off on a serial line.
    telnet: bool,
    /// Line speed for ACS `B`, 0 = telnet.
    pub baud: u32,
    buf: Vec<u8>,
    last: u8,
    skip_lf: bool,
    /// Keys stuffed by a `-;` macro, returned before real input.
    pub stuffed: std::collections::VecDeque<char>,
    /// Current text attribute (DOS style: low nibble fg, high nibble bg, 0x80 blink).
    pub curco: u8,
    idle: Duration,
    /// Output since the last flush, before telnet escaping, for `tap`.
    raw: Vec<u8>,
    pub tap: std::sync::Arc<std::sync::Mutex<Tap>>,
    /// User audit file (inittrapfile): the caller's keys are appended.
    pub trap: Option<std::fs::File>,
}

impl Term {
    pub fn new(s: Link, idle: Duration) -> io::Result<Term> {
        if let Link::Tcp(s) = &s {
            s.set_nodelay(true)?;
        }
        let (telnet, baud) = (s.telnet(), s.baud());
        let mut t = Term {
            s,
            telnet,
            baud,
            buf: Vec::new(),
            last: 0,
            skip_lf: false,
            stuffed: Default::default(),
            curco: 7,
            idle,
            raw: Vec::new(),
            tap: Default::default(),
            trap: None,
        };
        if telnet {
            // WILL ECHO + WILL SUPPRESS-GO-AHEAD: character mode, we do the echoing.
            t.buf.extend_from_slice(&[IAC, 251, 1, IAC, 251, 3]);
        }
        Ok(t)
    }

    /// Queue one byte of output; bare LF becomes CRLF, IAC is escaped (telnet).
    pub fn put(&mut self, b: u8) {
        if b == b'\n' && self.last != b'\r' {
            self.buf.push(b'\r');
        }
        if b == IAC && self.telnet {
            self.buf.push(IAC);
        }
        self.buf.push(b);
        self.raw.push(b);
        self.last = b;
    }

    pub fn put_str(&mut self, s: &str) {
        s.bytes().for_each(|b| self.put(b));
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if !self.raw.is_empty() {
            let mut tap = self.tap.lock().unwrap_or_else(|e| e.into_inner());
            if tap.live {
                print!("{}", cp437(&self.raw));
                let _ = io::stdout().flush();
            }
            tap.back.extend(self.raw.drain(..));
            let over = tap.back.len().saturating_sub(BACK);
            tap.back.drain(..over);
        }
        if !self.buf.is_empty() {
            self.s.write_all(&self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }

    fn byte(&mut self) -> io::Result<u8> {
        let mut b = [0];
        loop {
            if !self.s.wait(self.idle)? {
                return Err(io::ErrorKind::TimedOut.into());
            }
            match self.s.read(&mut b) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) => return Ok(b[0]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Wait up to `wait` for input; true if a key (or telnet data) is ready.
    pub fn key_ready(&mut self, wait: Duration) -> io::Result<bool> {
        self.flush()?;
        if !self.stuffed.is_empty() {
            return Ok(true);
        }
        // EOF also counts as ready; the next getkey reports it
        self.s.wait(wait)
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
                IAC if self.telnet => match self.byte()? {
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
                    self.trapped(b"\n");
                    return Ok('\r');
                }
                8 | 127 => {
                    self.trapped(b"\x08 \x08");
                    return Ok(b as char);
                }
                _ => {
                    self.trapped(&[b]);
                    return Ok(b as char);
                }
            }
        }
    }
}

impl Term {
    fn trapped(&mut self, b: &[u8]) {
        if let Some(f) = &mut self.trap {
            let _ = f.write_all(b);
        }
    }
}

/// Cursor keys as returned by `getkey_ext` (private-use chars).
pub const UP: char = '\u{E000}';
pub const DOWN: char = '\u{E001}';
pub const RIGHT: char = '\u{E002}';
pub const LEFT: char = '\u{E003}';
pub const HOME: char = '\u{E004}';
pub const END: char = '\u{E005}';
pub const DEL: char = '\u{E006}';
pub const PGUP: char = '\u{E007}';
pub const PGDN: char = '\u{E008}';

/// ANSI cursor sequence (after ESC [ or ESC O) to one of the key consts.
fn ansi_key(seq: &str) -> Option<char> {
    Some(match seq {
        "A" => UP,
        "B" => DOWN,
        "C" => RIGHT,
        "D" => LEFT,
        "H" | "1~" | "7~" => HOME,
        "F" | "K" | "4~" | "8~" => END,
        "3~" => DEL,
        "5~" => PGUP,
        "6~" => PGDN,
        _ => return None,
    })
}

impl Term {
    /// `getkey`, with ANSI cursor sequences folded into UP/DOWN/...;
    /// a lone ESC comes back as '\x1b'.
    pub fn getkey_ext(&mut self) -> io::Result<char> {
        let c = self.getkey()?;
        if c != '\x1b' || !self.key_ready(Duration::from_millis(80))? {
            return Ok(c);
        }
        let b = self.getkey()?;
        if b != '[' && b != 'O' {
            return Ok(b);
        }
        let mut seq = String::new();
        while seq.len() < 4 && self.key_ready(Duration::from_millis(80))? {
            let k = self.getkey()?;
            seq.push(k);
            if k.is_ascii_alphabetic() || k == '~' {
                break;
            }
        }
        Ok(ansi_key(&seq).unwrap_or('\x1b'))
    }

    /// Run `cmd` with stdin/stdout joined to the caller, telnet-escaped both
    /// ways (transfer protocols and doors; the FOSSIL handoff in EXEC.PAS).
    /// `text` turns the caller's Enter (CR, CR LF, CR NUL) into LF for doors.
    pub fn run_external(&mut self, cmd: &mut Command, text: bool) -> io::Result<ExitStatus> {
        if self.telnet {
            // WILL/DO BINARY so 8-bit transfers survive the client.
            self.buf.extend_from_slice(&[IAC, 251, 0, IAC, 253, 0]);
        }
        let telnet = self.telnet;
        self.flush()?;
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let mut cin = child.stdin.take();
        let mut cout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
        let mut sock = self.s.try_clone()?;
        let pump = std::thread::spawn(move || {
            let mut b = [0u8; 4096];
            let mut last = 0;
            while let Ok(n @ 1..) = cout.read(&mut b) {
                if sock.write_all(&escape_out(&b[..n], text, telnet, &mut last)).is_err() {
                    break;
                }
            }
        });
        let mut state = Unescape { text, telnet, ..Unescape::default() };
        let mut b = [0u8; 4096];
        let r = loop {
            if let Some(st) = child.try_wait()? {
                break Ok(st);
            }
            match self.s.wait(Duration::from_millis(100)).and_then(|r| if r { self.s.read(&mut b) } else { Err(io::ErrorKind::TimedOut.into()) }) {
                Ok(0) => {
                    let _ = child.kill();
                    break Err(io::ErrorKind::UnexpectedEof.into());
                }
                Ok(n) => {
                    let data = state.feed(&b[..n]);
                    if let Some(w) = cin.as_mut()
                        && w.write_all(&data).is_err()
                    {
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
        r
    }
}

/// Double IACs (telnet); in text mode also turn bare LF into CR LF.
fn escape_out(b: &[u8], text: bool, telnet: bool, last: &mut u8) -> Vec<u8> {
    let mut v = Vec::with_capacity(b.len());
    for &x in b {
        if text && x == b'\n' && *last != b'\r' {
            v.push(b'\r');
        }
        *last = x;
        v.push(x);
        if x == IAC && telnet {
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
    text: bool,
    telnet: bool,
    after_cr: bool,
}

impl Unescape {
    fn feed(&mut self, b: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        for &x in b {
            self.st = match (self.st, x) {
                (0, IAC) if self.telnet => 1,
                (0, _) if self.text => {
                    match x {
                        b'\n' | 0 if self.after_cr => {}
                        b'\r' => v.push(b'\n'),
                        _ => v.push(x),
                    }
                    self.after_cr = x == b'\r';
                    0
                }
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
    fn cp437_table() {
        assert_eq!(super::cp437(&[b'A', 0x80, 0xB3, 0xDB, 0xFE]), "AÇ│█■");
        assert_eq!(super::cp437(&[0xFF]), "\u{a0}");
    }

    #[test]
    fn telnet_escaping() {
        assert_eq!(super::escape_out(&[1, 255, 2, 10], false, true, &mut 0), [1, 255, 255, 2, 10]);
        assert_eq!(super::escape_out(&[1, 255], false, false, &mut 0), [1, 255]);
        assert_eq!(super::escape_out(b"a\nb\r\n", true, true, &mut 0), b"a\r\nb\r\n");
        let mut raw = super::Unescape::default();
        assert_eq!(raw.feed(&[255, 251, 1]), [255, 251, 1]);
        let mut u = super::Unescape { telnet: true, ..Default::default() };
        // IAC IAC -> 255, IAC WILL x dropped, SB ... SE dropped, split across feeds
        assert_eq!(u.feed(&[1, 255, 255, 2, 255, 251]), [1, 255, 2]);
        assert_eq!(u.feed(&[0, 3, 255, 250, 24, 0, 255, 240, 4]), [3, 4]);
        let mut t = super::Unescape { text: true, telnet: true, ..Default::default() };
        assert_eq!(t.feed(b"a\r\nb\r\0c\r"), b"a\nb\nc\n");
        assert_eq!(t.feed(b"\nd"), b"d");
    }

    #[test]
    fn cursor_keys() {
        assert_eq!(super::ansi_key("A"), Some(super::UP));
        assert_eq!(super::ansi_key("3~"), Some(super::DEL));
        assert_eq!(super::ansi_key("Z"), None);
    }

    #[test]
    fn colors() {
        assert_eq!(ansi_color(7, 15), "\x1b[1m");
        assert_eq!(ansi_color(15, 7), "\x1b[0;37m");
        assert_eq!(ansi_color(7, 0x1e), "\x1b[33;44;1m");
        assert_eq!(ansi_color(7, 4), "\x1b[31m");
    }
}

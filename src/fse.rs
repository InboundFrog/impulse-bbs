//! Full-screen ANSI message editor (ANSIEDIT.PAS): WordStar control keys
//! plus cursor keys, word wrap, and `/S` `/A` `/Q` `/?` typed on a line alone.

use crate::db::Msg;
use crate::session::{Session, strip_pipes};
use crate::term::{DEL, DOWN, END, HOME, LEFT, PGDN, PGUP, RIGHT, UP};
use std::io;

/// Right margin (rightmargin := cols-2).
const WIDTH: usize = 78;

/// What the screen needs after a key.
#[derive(Debug, PartialEq)]
pub enum Ed {
    Cursor,
    Line,
    /// Redraw from this line down.
    From(usize),
    Refresh,
    Save,
    Abort,
    Quote,
    Help,
}

pub struct Buf {
    pub l: Vec<Vec<char>>,
    pub x: usize,
    pub y: usize,
    pub ins: bool,
    max: usize,
    /// Screen rows taken by the header above the text.
    pub head: usize,
}

impl Buf {
    pub fn new(max: usize) -> Buf {
        Buf { l: vec![Vec::new()], x: 0, y: 0, ins: true, max: max.max(1), head: 3 }
    }

    fn rows(&self) -> usize {
        24 - self.head
    }

    fn cur(&mut self) -> &mut Vec<char> {
        &mut self.l[self.y]
    }

    fn clamp(&mut self) {
        self.x = self.x.min(self.l[self.y].len());
    }

    /// Insert `v` as line `at`, if there's room.
    fn insert_line(&mut self, at: usize, v: Vec<char>) -> bool {
        if self.l.len() >= self.max {
            return false;
        }
        self.l.insert(at, v);
        true
    }

    pub fn text(&self) -> Vec<String> {
        let mut v: Vec<String> = self.l.iter().map(|l| l.iter().collect::<String>().trim_end().to_string()).collect();
        while v.last().is_some_and(|l| l.is_empty()) {
            v.pop();
        }
        v
    }

    /// Wrap the current line at the last space before the margin,
    /// pushing the tail onto a new line below (scrollwwrap).
    fn wrap(&mut self) -> Ed {
        let l = &self.l[self.y];
        if l.len() <= WIDTH {
            return Ed::Line;
        }
        let cut = l[..WIDTH].iter().rposition(|&c| c == ' ').map_or(WIDTH, |p| p + 1);
        let tail: Vec<char> = self.l[self.y].split_off(cut);
        if !self.insert_line(self.y + 1, tail.clone()) {
            // no room: drop what didn't fit
            self.x = self.x.min(cut);
            return Ed::Line;
        }
        let from = self.y;
        if self.x >= cut {
            self.y += 1;
            self.x -= cut;
        }
        Ed::From(from)
    }

    fn word_left(&mut self) {
        let l = &self.l[self.y];
        while self.x > 0 && l[self.x - 1] == ' ' {
            self.x -= 1;
        }
        while self.x > 0 && l[self.x - 1] != ' ' {
            self.x -= 1;
        }
    }

    fn word_right(&mut self) {
        let l = &self.l[self.y];
        while self.x < l.len() && l[self.x] != ' ' {
            self.x += 1;
        }
        while self.x < l.len() && l[self.x] == ' ' {
            self.x += 1;
        }
    }

    /// Enter: a lone `/x` is a command, otherwise split the line
    /// (or just move down in overwrite mode).
    fn enter(&mut self) -> Ed {
        let s: String = self.l[self.y].iter().collect::<String>().to_ascii_uppercase();
        let cmd = match s.trim() {
            "/S" => Some(Ed::Save),
            "/A" => Some(Ed::Abort),
            "/Q" => Some(Ed::Quote),
            "/?" => Some(Ed::Help),
            _ => None,
        };
        if let Some(c) = cmd {
            self.cur().clear();
            self.x = 0;
            return c;
        }
        let tail = if self.ins { self.l[self.y].split_off(self.x) } else { Vec::new() };
        if self.y + 1 < self.l.len() && !self.ins {
            self.y += 1;
            self.x = 0;
            return Ed::Cursor;
        }
        if !self.insert_line(self.y + 1, tail.clone()) {
            self.l[self.y].extend(tail);
            return Ed::Cursor;
        }
        self.y += 1;
        self.x = 0;
        Ed::From(self.y - 1)
    }

    pub fn key(&mut self, k: char) -> Ed {
        let last = self.l.len() - 1;
        match k {
            LEFT | '\x13' => {
                if self.x > 0 {
                    self.x -= 1;
                } else if self.y > 0 {
                    self.y -= 1;
                    self.x = self.l[self.y].len();
                }
            }
            RIGHT | '\x04' => {
                if self.x < self.l[self.y].len() {
                    self.x += 1;
                } else if self.y < last {
                    self.y += 1;
                    self.x = 0;
                }
            }
            UP | '\x05' => self.y = self.y.saturating_sub(1),
            DOWN | '\x18' => self.y = (self.y + 1).min(last),
            PGUP | '\x12' => self.y = self.y.saturating_sub(self.rows()),
            PGDN | '\x03' => self.y = (self.y + self.rows()).min(last),
            HOME | '\x11' => self.x = 0,
            END | '\x17' => self.x = self.l[self.y].len(),
            '\x01' => self.word_left(),
            '\x06' => self.word_right(),
            '\x09' => self.ins = !self.ins,
            '\x0c' => return Ed::Refresh,
            '\x15' => return Ed::Help,
            '\x1a' => return Ed::Save,
            '\r' => return self.enter(),
            '\x08' | '\x7f' => {
                if self.x > 0 {
                    self.x -= 1;
                    let x = self.x;
                    self.cur().remove(x);
                    return Ed::Line;
                }
                if self.y == 0 {
                    return Ed::Cursor;
                }
                // join onto the line above
                let l = self.l.remove(self.y);
                self.y -= 1;
                self.x = self.l[self.y].len();
                self.cur().extend(l);
                let from = self.y;
                let e = self.wrap();
                self.y = from;
                self.clamp();
                return if e == Ed::Line { Ed::From(from) } else { e };
            }
            DEL | '\x07' => {
                if self.x < self.l[self.y].len() {
                    let x = self.x;
                    self.cur().remove(x);
                    return Ed::Line;
                }
                return self.key('\x0a');
            }
            '\x0a' => {
                // ^J join the next line onto this one
                if self.y < last {
                    let l = self.l.remove(self.y + 1);
                    let (y, x) = (self.y, self.x);
                    self.cur().extend(l);
                    self.wrap();
                    (self.y, self.x) = (y, x);
                    return Ed::From(y);
                }
            }
            '\x14' => {
                // ^T delete the word right of the cursor
                let x = self.x;
                self.word_right();
                let end = self.x;
                self.x = x;
                self.cur().drain(x..end);
                return Ed::Line;
            }
            '\x19' => {
                // ^Y delete line
                if self.l.len() > 1 {
                    self.l.remove(self.y);
                    self.y = self.y.min(self.l.len() - 1);
                } else {
                    self.l[0].clear();
                }
                self.clamp();
                return Ed::From(self.y);
            }
            '\x0e' => {
                // ^N insert a blank line
                if self.insert_line(self.y, Vec::new()) {
                    self.x = 0;
                    return Ed::From(self.y);
                }
            }
            '\x02' => {
                // ^B break the line at the cursor, staying put
                let tail = self.l[self.y].split_off(self.x);
                if !self.insert_line(self.y + 1, tail.clone()) {
                    self.cur().extend(tail);
                    return Ed::Cursor;
                }
                return Ed::From(self.y);
            }
            '\x10' => {
                let x = self.x;
                self.cur().truncate(x);
                return Ed::Line;
            }
            c if c >= ' ' && (c as u32) < 256 && c != '\x7f' => {
                let x = self.x;
                if self.ins || x >= self.l[self.y].len() {
                    self.cur().insert(x, c);
                } else {
                    self.cur()[x] = c;
                }
                self.x += 1;
                return self.wrap();
            }
            _ => {}
        }
        self.clamp();
        Ed::Cursor
    }
}

impl Session {
    /// The full-screen editor; same contract as `editor`.
    pub fn fse(&mut self, max: usize, to: &str, subject: &str, quote: Option<&Msg>) -> io::Result<Option<Vec<String>>> {
        let mut b = Buf::new(max);
        let board = self.cfg().boards.get(self.board).map(|b| b.name.clone()).unwrap_or_default();
        let vars = [("BN", board), ("DT", crate::session::today()), ("FM", self.user.name.clone()), ("TL", subject.to_string()), ("TO", to.to_string())];
        for (k, v) in vars {
            self.vars.insert(k.into(), v);
        }
        let file = self.text_file("FSEHEAD");
        // header height is where FSEHEAD leaves the cursor (hdrlen:=wherey)
        b.head = file.as_ref().map_or(3, |t| (t.matches('\n').count() + usize::from(!t.ends_with('\n'))).clamp(1, 20));
        let rows = b.rows();
        let head = file.unwrap_or_else(|| {
            format!(
                "|U1To: |U3{:<30}|U1 Subj: |U3{}|NL|U1Ctrl: |U3Z|U1 save  |U3U|U1 help  |U3I|U1 insert  |U3Y|U1 del line   or |U3/S /A{} /?|U1 on a line alone",
                to,
                subject,
                if quote.is_some() { " /Q" } else { "" }
            ) + "|NL"
                + &"\u{c4}".repeat(79)
        }); // CP437 line
        let r = self.fse_loop(&mut b, &head, rows, quote);
        for k in ["BN", "DT", "FM", "TL", "TO"] {
            self.vars.remove(k);
        }
        r
    }

    fn fse_loop(&mut self, b: &mut Buf, head: &str, rows: usize, quote: Option<&Msg>) -> io::Result<Option<Vec<String>>> {
        let mut top = 0;
        let mut redraw = Ed::Refresh;
        loop {
            // keep the cursor in the window
            if b.y < top {
                top = b.y;
                redraw = Ed::Refresh;
            } else if b.y >= top + rows {
                top = b.y + 1 - rows;
                redraw = Ed::Refresh;
            }
            match redraw {
                Ed::Refresh => {
                    self.cls()?;
                    self.show_text(head)?;
                    self.draw_lines(b, top, top, top + rows)?;
                }
                Ed::From(n) => self.draw_lines(b, top, n.max(top), top + rows)?,
                Ed::Line => self.draw_lines(b, top, b.y, b.y + 1)?,
                _ => {}
            }
            self.setc(7)?;
            self.out(&format!("\x1b[{};{}H", b.y - top + b.head + 1, b.x + 1))?;
            let k = self.t.getkey_ext()?;
            redraw = b.key(k);
            match redraw {
                Ed::Save => {
                    let t = b.text();
                    self.out(&format!("\x1b[{};1H", 24))?;
                    self.nl()?;
                    return Ok((!t.is_empty()).then_some(t));
                }
                Ed::Abort => {
                    self.out("\x1b[24;1H")?;
                    self.nl()?;
                    return Ok(None);
                }
                Ed::Help => {
                    self.cls()?;
                    if !self.printf("FSEHELP")? {
                        for l in FSE_HELP {
                            self.sprint(l)?;
                        }
                    }
                    self.pause()?;
                    redraw = Ed::Refresh;
                }
                Ed::Quote => {
                    if let Some(m) = quote {
                        self.cls()?;
                        let q = self.pick_quote(m)?;
                        for l in q {
                            if !b.insert_line(b.y, l.chars().collect()) {
                                break;
                            }
                            b.y += 1;
                        }
                    }
                    redraw = Ed::Refresh;
                }
                _ => {}
            }
        }
    }

    /// Draw text lines `from..to` of the window starting at line `top`.
    fn draw_lines(&mut self, b: &Buf, top: usize, from: usize, to: usize) -> io::Result<()> {
        self.setc(7)?;
        for n in from..to {
            self.out(&format!("\x1b[{};1H\x1b[K", n - top + b.head + 1))?;
            if let Some(l) = b.l.get(n) {
                self.out(&l.iter().collect::<String>())?;
            } else if n > b.l.len() {
                break; // lines below the text were cleared on the last pass
            }
        }
        Ok(())
    }

    /// Ask which lines of `m` to quote; returns them prefixed.
    pub fn pick_quote(&mut self, m: &Msg) -> io::Result<Vec<String>> {
        let pre = crate::msgs::quote_prefix(Self::anon_name(m).unwrap_or(&m.from));
        for (i, l) in m.body.iter().enumerate() {
            self.sprompt(&format!("|U2{:>3}: |U0", i + 1))?;
            self.out(&strip_pipes(l))?;
            self.nl()?;
        }
        self.sprompt(&format!("|U1Quote from line (1-{}): |U3", m.body.len()))?;
        let a = self.input(3)?.parse::<usize>().unwrap_or(0);
        self.sprompt(&format!("|U1To line ({a}-{}): |U3", m.body.len()))?;
        let z = self.input(3)?.parse::<usize>().unwrap_or(a).min(m.body.len());
        if a < 1 {
            return Ok(Vec::new());
        }
        Ok(m.body.iter().take(z).skip(a - 1).map(|l| format!("{pre}{}", strip_pipes(l)).chars().take(WIDTH).collect()).collect())
    }
}

const FSE_HELP: &[&str] = &[
    "|U5Full-screen editor keys",
    "",
    "|U3^S ^D|U1 / arrows  left, right        |U3^E ^X|U1 / arrows  up, down",
    "|U3^A ^F|U1           word left, right   |U3^R ^C|U1 / PgUp PgDn  page up, down",
    "|U3^Q ^W|U1 / Home End  line start, end  |U3^I|U1  insert/overwrite",
    "|U3^H|U1  backspace  |U3^G|U1 / Del  delete   |U3^T|U1  delete word   |U3^P|U1  delete to end",
    "|U3^Y|U1  delete line  |U3^N|U1  insert line  |U3^B|U1  break line  |U3^J|U1  join lines",
    "|U3^L|U1  redraw  |U3^Z|U1  save",
    "",
    "|U1On a line by itself: |U3/S|U1 save  |U3/A|U1 abort  |U3/Q|U1 quote  |U3/?|U1 this help",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(b: &mut Buf, s: &str) {
        s.chars().for_each(|c| {
            b.key(c);
        });
    }

    #[test]
    fn editing() {
        let mut b = Buf::new(10);
        typed(&mut b, "hello world\rsecond");
        assert_eq!(b.text(), ["hello world", "second"]);
        // backspace at line start joins
        b.key(HOME);
        b.key('\x08');
        assert_eq!((b.text(), b.y, b.x), (vec!["hello worldsecond".to_string()], 0, 11));
        // ^B splits, ^J joins back
        b.key('\x02');
        assert_eq!(b.text(), ["hello world", "second"]);
        b.key('\x0a');
        assert_eq!(b.text(), ["hello worldsecond"]);
        // word wrap carries the last word down with the cursor
        let mut w = Buf::new(10);
        typed(&mut w, &"abc ".repeat(19));
        typed(&mut w, "wrapped");
        assert_eq!(w.l.len(), 2);
        assert_eq!(w.text()[1], "wrapped");
        assert_eq!((w.y, w.x), (1, 7));
        // commands on a line alone; line limit holds
        let mut c = Buf::new(2);
        typed(&mut c, "a\rb\rc");
        assert_eq!(c.text(), ["a", "bc"]);
        assert_eq!(c.key('\x19'), Ed::From(0));
        assert_eq!(c.text(), ["a"]);
        typed(&mut c, "\x05\x19/s");
        assert_eq!(c.key('\r'), Ed::Save);
        assert!(c.text().is_empty());
    }
}

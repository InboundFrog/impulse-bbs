//! One caller on one node: output with pipe/MCI codes (OUTPUT.PAS, COMMON.PAS),
//! input, logon (LOGON1/2.PAS, NEWUSERS.PAS) and the menu loop (IMP.PAS, MENUS.PAS).

use crate::config::Config;
use crate::db::{self, Db, LastCaller, User};
use crate::menu::{self, Command, Menu};
use crate::term::{self, Term};
use crate::{acs, events};
use chrono::{Datelike, Local, NaiveDate, Timelike};
use std::collections::HashMap;
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

pub struct Shared {
    /// Swapped whole when the SysOp edits it; sessions hold an `Arc` snapshot.
    pub cfg: RwLock<Arc<Config>>,
    pub cfg_path: String,
    // ponytail: one global lock for all data, per-file locks if node counts ever get large
    pub db: Mutex<Db>,
    /// Chat forum the console has joined (`C node`), 0 = none.
    pub console_forum: std::sync::atomic::AtomicU8,
}

impl Shared {
    pub fn db(&self) -> MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn cfg(&self) -> Arc<Config> {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Apply an edit to the configuration and write it back to disk.
    pub fn edit_cfg(&self, f: impl FnOnce(&mut Config)) -> io::Result<()> {
        let mut g = self.cfg.write().unwrap_or_else(|e| e.into_inner());
        let mut c = (**g).clone();
        f(&mut c);
        c.save(&self.cfg_path)?;
        *g = Arc::new(c);
        Ok(())
    }
}

/// Ends the session; every input/output call propagates it with `?`.
pub fn hangup() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "hangup")
}

pub enum Flow {
    /// Keep running further commands bound to the same keys.
    Next,
    /// Stop running commands for these keys (`-$` password failed).
    Stop,
    /// Menu changed; optionally run this command on the new menu.
    Menu(Option<String>),
}

pub struct Session {
    pub sh: Arc<Shared>,
    pub t: Term,
    pub node: usize,
    pub user: User,
    /// User number, 0 until logged in.
    pub unum: usize,
    pub menu: Menu,
    pub menu_name: String,
    menu_stack: Vec<String>,
    pub board: usize,
    /// Current message and file conference, 0 = all.
    pub mconf: usize,
    pub fconf: usize,
    /// Board files whose password was given this call.
    pub board_pw: Vec<String>,
    pub fbase: usize,
    pub fbase_pw: Vec<String>,
    /// Batch download queue: (file base, file name).
    pub batch: Vec<(usize, String)>,
    /// New file scan date, the last call's date unless `FP` changes it.
    pub new_date: String,
    /// `%XX` template variables (sysMciSet).
    pub vars: HashMap<String, String>,
    /// Header text for the `HDR` display file (`-W`, `%HS`).
    pub hdr: String,
    /// Inside a display file: `%XX` MCI codes are live (showscr).
    pct: bool,
    /// Key handed back by a `@x`/`$x`/`!x` command in a prompt menu (readpmnu).
    pub pkey: Option<char>,
    /// Set by O1-O3 on the matrix menu (tshuttlelogon).
    pub shuttle: u8,
    /// Minutes left are capped so the caller is off before an event.
    chop: Option<Instant>,
    help: bool,
    /// A command just ran from a menu (forcepause, `F` menu flag).
    ran_cmd: bool,
    on_since: Instant,
    lines: u16,
    paging: bool,
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn now() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// ponytail: clock-based pick, plenty for rumors and random screens
pub fn rand(n: usize) -> usize {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    t.subsec_nanos() as usize % n.max(1)
}

/// DOS-era file names are any case; match them case-insensitively.
pub fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.eq_ignore_ascii_case(name)))
}

/// Text with `|XX` codes removed, for measuring/printing raw.
pub fn strip_pipes(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '|' {
            it.next();
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

pub fn run(sh: Arc<Shared>, stream: TcpStream, node: usize) {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    let idle = Duration::from_secs(sh.cfg().idle_timeout_minutes.max(1) * 60);
    let mut t = match Term::new(stream, idle) {
        Ok(t) => t,
        Err(_) => {
            sh.db().online.remove(&node);
            return;
        }
    };
    t.tap = sh.db().online.get(&node).map(|o| o.tap.clone()).unwrap_or_default();
    let mut s = Session {
        sh,
        t,
        node,
        user: User::default(),
        unum: 0,
        menu: Menu::default(),
        menu_name: String::new(),
        menu_stack: Vec::new(),
        board: 0,
        vars: HashMap::new(),
        hdr: String::new(),
        pct: false,
        pkey: None,
        shuttle: 0,
        chop: None,
        mconf: 0,
        fconf: 0,
        board_pw: Vec::new(),
        fbase: 0,
        fbase_pw: Vec::new(),
        batch: Vec::new(),
        new_date: String::new(),
        help: false,
        ran_cmd: false,
        on_since: Instant::now(),
        lines: 0,
        paging: true,
    };
    s.log(&format!("Connect from {peer}"));
    if let Err(e) = s.main()
        && e.kind() != io::ErrorKind::ConnectionAborted
    {
        s.log(&format!("Disconnected: {e}"));
    }
    s.logoff();
}

impl Session {
    fn main(&mut self) -> io::Result<()> {
        self.cls()?;
        self.sprint("|U3Impulse |U4v|VV |U1(Rust port)  |U2Node |NN")?;
        if !(self.cfg().shuttle_log && self.matrix()?) {
            self.printf("welcome")?;
            self.getuser()?;
        }
        self.logon()?;
        let start = self.cfg().start_menu.clone();
        match self.goto_menu(&start)? {
            Flow::Menu(Some(c)) => self.exec(&c)?,
            Flow::Menu(None) => {}
            _ => {
                self.sprint(&format!("|12Start menu {start} is missing. Please inform the SysOp."))?;
                return Err(hangup());
            }
        }
        loop {
            self.check_time()?;
            let cmd = self.menu_prompt()?;
            self.exec(&cmd)?;
        }
    }

    pub fn cfg(&self) -> Arc<Config> {
        self.sh.cfg()
    }

    pub fn log(&self, s: &str) {
        let who = if self.unum > 0 { format!(" {}", self.user.name) } else { String::new() };
        self.sh.db().log(&format!("[{}] node {}{who}: {s}", now(), self.node));
    }

    pub fn save_user(&self) {
        if self.unum == 0 {
            return;
        }
        let mut db = self.sh.db();
        db.users[self.unum - 1] = self.user.clone();
        if let Err(e) = db.save() {
            db.log(&format!("Error saving user data: {e}"));
        }
    }

    /// Pick up changes the SysOp made to this account from another node.
    pub fn check_reload(&mut self) {
        let mut db = self.sh.db();
        if let Some(o) = db.online.get_mut(&self.node)
            && std::mem::take(&mut o.reload)
            && let Some(u) = db.users.get(self.unum.wrapping_sub(1))
        {
            self.user = u.clone();
        }
    }

    pub fn set_doing(&self, doing: &str) {
        if let Some(o) = self.sh.db().online.get_mut(&self.node) {
            o.doing = doing.to_string();
        }
    }

    pub fn is_sysop(&self) -> bool {
        self.unum > 0 && self.acs(&self.cfg().sysop_acs)
    }

    pub fn on_since_secs(&self) -> u64 {
        self.on_since.elapsed().as_secs()
    }

    pub fn time_left_min(&self) -> i64 {
        let left = self.user.minutes_today - (self.on_since.elapsed().as_secs() / 60) as i64;
        match self.chop {
            Some(t) => left.min(t.saturating_duration_since(Instant::now()).as_secs() as i64 / 60),
            None => left,
        }
    }

    fn check_time(&mut self) -> io::Result<()> {
        if self.time_left_min() <= 0 && !self.is_sysop() {
            if !self.printf("timeup")? {
                self.sprint("|NL|12Your time for today has expired.")?;
            }
            return Err(hangup());
        }
        Ok(())
    }

    // ---- ACS --------------------------------------------------------------

    pub fn acs(&self, s: &str) -> bool {
        self.acs_for(&self.user, self.unum, s)
    }

    /// ACS check against another user (aacs1).
    pub fn acs_for(&self, u: &User, unum: usize, s: &str) -> bool {
        let now = Local::now();
        let cfg = self.cfg();
        acs::eval(s, &mut |c, arg| {
            let n: i64 = arg.parse().unwrap_or(0);
            let ch = arg.chars().next().unwrap_or(' ');
            match c {
                'A' => age(&u.birthday) >= n,
                'C' => self.mconf as i64 == n,
                'D' => u.dsl as i64 >= n,
                'F' => u.ar.contains(ch),
                'G' => u.sex.starts_with(ch),
                'H' => now.hour() as i64 == n,
                'P' => u.file_points as i64 >= n,
                'R' => u.ac.contains(ch),
                'S' => u.sl as i64 >= n,
                'T' => self.time_left_min() >= n,
                'U' => unum as i64 == n,
                'V' => u.sl > cfg.new_sl || u.dsl > cfg.new_dsl,
                'W' => now.weekday().num_days_from_sunday() as i64 == n,
                'Y' => (now.hour() * 60 + now.minute()) as i64 >= n,
                _ => true, // B (baud): every telnet caller is fast enough
            }
        })
    }

    // ---- output -----------------------------------------------------------

    /// Raw output. Characters are bytes (CP437 passes through untouched).
    pub fn out(&mut self, s: &str) -> io::Result<()> {
        for ch in s.chars() {
            self.t.put(if (ch as u32) < 256 { ch as u8 } else { b'?' });
            if ch == '\n' {
                self.lines += 1;
                if self.paging && self.user.pause && self.lines + 1 >= self.user.page_len.max(5) {
                    self.pause()?;
                }
            }
        }
        Ok(())
    }

    pub fn nl(&mut self) -> io::Result<()> {
        self.out("\n")
    }

    pub fn cls(&mut self) -> io::Result<()> {
        self.lines = 0;
        self.out(if self.user.ansi { "\x1b[2J\x1b[H" } else { "\x0c" })
    }

    pub fn setc(&mut self, c: u8) -> io::Result<()> {
        if self.user.ansi && c != self.t.curco {
            let s = term::ansi_color(self.t.curco, c);
            self.t.put_str(&s);
            self.t.curco = c;
        }
        Ok(())
    }

    /// User colour 0-9 (`|U0`, `^0`).
    pub fn cl(&mut self, n: u8) -> io::Result<()> {
        self.setc(self.user.colors[n as usize % 10])
    }

    pub fn sprint(&mut self, s: &str) -> io::Result<()> {
        self.sprompt(s)?;
        self.nl()
    }

    /// Output with `|XX` pipe/MCI codes, `^N` user colours and `%XX` variables.
    pub fn sprompt(&mut self, s: &str) -> io::Result<()> {
        let c: Vec<char> = s.chars().collect();
        let mut i = 0;
        let mut text = String::new();
        while i < c.len() {
            let ch = c[i];
            if ch == '|' && i + 2 < c.len() {
                let code: String = c[i + 1..i + 3].iter().collect::<String>().to_ascii_uppercase();
                i += 3;
                self.out(&std::mem::take(&mut text))?;
                if !self.pipe(&code, &c, &mut i)? {
                    text.push('|');
                    text.extend(&c[i - 2..i]);
                }
                continue;
            }
            if (ch == '^' || ch == '\x03') && i + 1 < c.len() {
                let d = c[i + 1];
                let n = match d {
                    '0'..='9' => Some(d as u8 - b'0'),
                    '\0'..='\x09' if ch == '\x03' => Some(d as u8),
                    _ => None,
                };
                if let Some(n) = n {
                    self.out(&std::mem::take(&mut text))?;
                    self.cl(n)?;
                    i += 2;
                    continue;
                }
            }
            if ch == '%' && i + 2 < c.len() {
                let key: String = c[i + 1..i + 3].iter().collect::<String>().to_ascii_uppercase();
                if let Some(v) = self.vars.get(&key) {
                    text.push_str(v);
                    i += 3;
                    continue;
                }
                if self.pct {
                    self.out(&std::mem::take(&mut text))?;
                    i += 3;
                    if !self.pipe(&key, &c, &mut i)? {
                        text.push('%');
                        text.extend(&c[i - 2..i]);
                    }
                    continue;
                }
            }
            text.push(ch);
            i += 1;
        }
        self.out(&text)
    }

    /// One pipe code; returns false if unknown so it is printed literally.
    fn pipe(&mut self, code: &str, c: &[char], i: &mut usize) -> io::Result<bool> {
        let b = code.as_bytes();
        let digit = |x: u8| (x as char).to_digit(16).map(|d| d as u8);
        match (b[0], b[1]) {
            (b'0', d @ b'0'..=b'9') => self.setc(d - b'0')?,
            (b'1', d @ b'0'..=b'5') => self.setc(10 + d - b'0')?,
            (b'B', d) if digit(d).is_some() => {
                let bg = digit(d).unwrap_or(0);
                self.setc(self.t.curco & 0x8f | (bg & 7) << 4)?
            }
            (b'U', d @ b'0'..=b'9') => self.cl(d - b'0')?,
            (b'R', b'B') => self.setc(self.t.curco & 0x0f | (rand(8) as u8) << 4)?,
            (b'R', b'F') => self.setc(self.t.curco & 0xf0 | rand(16) as u8)?,
            (b'R', b'C') => self.setc(rand(128) as u8)?,
            (b'C', b'L') => self.cls()?,
            (b'N', b'L') => self.nl()?,
            (b'P', b'A') => self.pause()?,
            (b'L', b'S') if *i < c.len() => {
                // %LSx: a line of x under the header
                let line = c[*i].to_string().repeat(self.hdr.chars().count() + 1);
                *i += 1;
                self.out(&line)?;
            }
            (b'X', b'Y') => {
                let num = |s: &[char]| s.iter().collect::<String>().parse::<u16>().unwrap_or(0).max(1);
                if *i + 4 <= c.len() {
                    let (x, y) = (num(&c[*i..*i + 2]), num(&c[*i + 2..*i + 4]));
                    *i += 4;
                    if self.user.ansi {
                        self.out(&format!("\x1b[{y};{x}H"))?;
                    }
                }
            }
            _ => match self.mci_text(code) {
                Some(s) => self.out(&s)?,
                None => return Ok(false),
            },
        }
        Ok(true)
    }

    fn mci_text(&self, code: &str) -> Option<String> {
        let u = &self.user;
        let cfg = self.cfg();
        Some(match code {
            "BN" => cfg.bbs_name.clone(),
            "SN" => cfg.sysop_name.clone(),
            "UN" => u.name.clone(),
            "RN" => u.real_name.clone(),
            "UU" => self.unum.to_string(),
            "UL" => u.sl.to_string(),
            "FL" => u.dsl.to_string(),
            "UC" => u.note.clone(),
            "LO" => u.city_state.clone(),
            "LD" => u.last_on.clone(),
            "PN" => u.phone.clone(),
            "CS" => u.logons.to_string(),
            "PS" => u.posts.to_string(),
            "UD" => u.uploads.to_string(),
            "DN" => u.downloads.to_string(),
            "UK" => u.uk.to_string(),
            "DK" => u.dk.to_string(),
            "FP" => u.file_points.to_string(),
            "TL" => self.time_left_min().max(0).to_string(),
            "TT" => u.minutes_today.to_string(),
            "CT" => self.sh.db().sys.callers.to_string(),
            "TC" => self.sh.db().sys.zlog().calls.to_string(),
            "NT" => self.sh.db().sys.zlog().new_users.to_string(),
            "NP" | "TP" => self.sh.db().sys.zlog().pub_posts.to_string(),
            "FT" | "NF" => self.sh.db().sys.zlog().uploads.to_string(),
            "LC" => self.sh.db().sys.last_callers.get(usize::from(self.unum > 0)).map(|l| l.name.clone()).unwrap_or_default(),
            "CM" => cfg.boards.get(self.board).map(|b| b.name.clone()).unwrap_or_default(),
            // ponytail: CF (current file base) is ours; 7.1 had no such code
            "CF" => cfg.fbases.get(self.fbase).map(|b| b.name.clone()).unwrap_or_default(),
            "NM" => strip_pipes(&self.menu.title[0]),
            "NN" => self.node.to_string(),
            "SP" => "TELNET".into(),
            "DT" => today(),
            "VA" => "imp".into(),
            "VN" => "Impulse".into(),
            "VV" => env!("CARGO_PKG_VERSION").into(),
            "OS" => std::env::consts::OS.into(),
            "HS" => self.hdr.clone(),
            "BM" => cfg.max_time_bank.to_string(),
            "BT" => u.time_bank.to_string(),
            _ => return None,
        })
    }

    /// Show a text file from `text_path` (printf): `NAME` picks NAME.ANS/.ASC,
    /// or a random NAME.1-NAME.9 if those exist. Returns false if missing.
    pub fn printf(&mut self, name: &str) -> io::Result<bool> {
        match self.text_file(name) {
            Some(t) => self.show_text(&t).map(|_| true),
            None => Ok(false),
        }
    }

    /// The text of display file `name` as `printf` would pick it, up to ^Z.
    pub fn text_file(&self, name: &str) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        let dir = self.cfg().text_path.clone();
        let path = if name.contains('.') {
            find_ci(&dir, name)
        } else {
            let variants: Vec<PathBuf> = (1..=9).filter_map(|n| find_ci(&dir, &format!("{name}.{n}"))).collect();
            if !variants.is_empty() {
                Some(variants[rand(variants.len())].clone())
            } else {
                let (a, b) = if self.user.ansi { ("ANS", "ASC") } else { ("ASC", "ANS") };
                find_ci(&dir, &format!("{name}.{a}")).or_else(|| find_ci(&dir, &format!("{name}.{b}")))
            }
        };
        let bytes = std::fs::read(path?).ok()?;
        let end = bytes.iter().position(|&b| b == 0x1a).unwrap_or(bytes.len()); // ^Z / SAUCE
        Some(bytes[..end].iter().map(|&b| b as char).collect())
    }

    /// Show a display file's text with `%` codes on and paging off.
    pub fn show_text(&mut self, text: &str) -> io::Result<()> {
        let paging = std::mem::replace(&mut self.paging, false);
        let pct = std::mem::replace(&mut self.pct, true);
        let r = self.sprompt(text);
        self.paging = paging;
        self.pct = pct;
        r
    }

    /// Show `file` with `%XX` vars set, or `fallback` if the file is missing.
    pub fn tmpl(&mut self, file: &str, vars: &[(&str, String)], fallback: &str) -> io::Result<()> {
        for (k, v) in vars {
            self.vars.insert(k.to_string(), v.clone());
        }
        let r = match self.printf(file) {
            Ok(false) if !fallback.is_empty() => self.sprint(fallback),
            r => r.map(|_| ()),
        };
        self.vars.clear();
        r
    }

    // ---- input ------------------------------------------------------------

    pub fn getkey(&mut self) -> io::Result<char> {
        self.lines = 0;
        self.t.getkey()
    }

    pub fn input_main(&mut self, max: usize, upper: bool, echo: Option<char>) -> io::Result<String> {
        let s = self.input_raw(max, upper, echo)?;
        self.nl()?;
        Ok(s)
    }

    /// Line input that leaves the cursor after the text (no newline).
    pub fn input_raw(&mut self, max: usize, upper: bool, echo: Option<char>) -> io::Result<String> {
        let mut s = String::new();
        loop {
            match self.getkey()? {
                '\r' => break,
                '\x08' | '\x7f' => {
                    if s.pop().is_some() {
                        self.out("\x08 \x08")?;
                    }
                }
                '\x18' => {
                    while s.pop().is_some() {
                        self.out("\x08 \x08")?;
                    }
                }
                c if c >= ' ' && s.chars().count() < max => {
                    let c = if upper { c.to_ascii_uppercase() } else { c };
                    s.push(c);
                    self.out(&echo.unwrap_or(c).to_string())?;
                }
                _ => {}
            }
        }
        Ok(s)
    }

    pub fn input(&mut self, max: usize) -> io::Result<String> {
        self.input_main(max, false, None).map(|s| s.trim().to_string())
    }

    pub fn input_caps(&mut self, max: usize) -> io::Result<String> {
        self.input_main(max, true, None).map(|s| s.trim().to_string())
    }

    pub fn input_pw(&mut self, max: usize) -> io::Result<String> {
        self.input_main(max, false, Some('*'))
    }

    /// Prompt until a non-empty answer.
    pub fn ask(&mut self, prompt: &str, max: usize) -> io::Result<String> {
        loop {
            self.sprompt(prompt)?;
            let s = self.input(max)?;
            if !s.is_empty() {
                return Ok(s);
            }
        }
    }

    /// Wait for one of `valid` (upper case), echo it, newline.
    pub fn onek(&mut self, valid: &str) -> io::Result<char> {
        loop {
            let c = self.getkey()?.to_ascii_uppercase();
            if valid.contains(c) {
                if c != '\r' {
                    self.out(&c.to_string())?;
                }
                self.nl()?;
                return Ok(c);
            }
        }
    }

    pub fn pynq(&mut self, prompt: &str, default: bool) -> io::Result<bool> {
        self.sprompt(prompt)?;
        self.sprompt(if default { "|U2(|U3Y|U2/n) |U3" } else { "|U2(y/|U3N|U2) |U3" })?;
        let yes = loop {
            match self.getkey()?.to_ascii_uppercase() {
                'Y' => break true,
                'N' => break false,
                '\r' => break default,
                _ => {}
            }
        };
        self.out(if yes { "Yes" } else { "No" })?;
        self.nl()?;
        Ok(yes)
    }

    pub fn pause(&mut self) -> io::Result<()> {
        let paging = std::mem::replace(&mut self.paging, false);
        let p = self.cfg().strings.pause.clone();
        self.sprompt(&p)?;
        self.getkey()?;
        self.out(if self.user.ansi { "\r\x1b[K" } else { "\r                              \r" })?;
        self.paging = paging;
        Ok(())
    }

    // ---- logon ------------------------------------------------------------

    fn getuser(&mut self) -> io::Result<()> {
        let sh = self.sh.clone();
        let cfg = sh.cfg();
        let st = &cfg.strings;
        let mut tries = 0;
        loop {
            if tries >= cfg.max_logon_tries {
                self.log("Too many logon attempts");
                return Err(hangup());
            }
            self.sprompt(&st.enter_handle)?;
            let s = self.input_caps(36)?;
            if s.is_empty() {
                tries += 1;
                continue;
            }
            if s == "NEW" {
                return self.new_user("");
            }
            let found = sh.db().find_user(&s);
            let Some(n) = found else {
                self.sprint(&st.name_not_found)?;
                if s.parse::<usize>().is_err() && self.pynq(&st.logon_new, true)? {
                    return self.new_user(&s);
                }
                tries += 1;
                continue;
            };
            self.sprompt(&st.enter_pw)?;
            let pw = self.input_pw(20)?;
            let u = sh.db().users[n - 1].clone();
            if db::check_login(&u.pw_hash, &pw) {
                self.user = u;
                self.unum = n;
                break;
            }
            self.sprint(&st.wrong_password)?;
            self.log(&format!("Wrong password for {}", u.name));
            tries += 1;
        }
        if self.user.locked_out {
            let f = if self.user.locked_file.is_empty() { "lockout".to_string() } else { self.user.locked_file.clone() };
            if !self.printf(&f)? {
                self.sprint("|12This account has been locked out.")?;
            }
            self.log("Locked-out user tried to log on");
            return Err(hangup());
        }
        Ok(())
    }

    /// Matrix logon (doshuttle). True if O2/O3 already established the user.
    fn matrix(&mut self) -> io::Result<bool> {
        self.printf("premtx")?;
        let cfg = self.cfg();
        if !matches!(self.goto_menu(&cfg.shuttle_menu)?, Flow::Menu(_)) {
            return Ok(false);
        }
        let mut tries = 0;
        loop {
            if tries >= cfg.max_logon_tries {
                return Err(hangup());
            }
            self.shuttle = 0;
            let cmd = self.menu_prompt()?;
            self.exec(&cmd)?;
            match std::mem::take(&mut self.shuttle) {
                1 if cfg.shuttle_pw.is_empty() => return Ok(false),
                1 => {
                    self.sprompt("|NL|U1System password: |U3")?;
                    if self.input_pw(20)?.eq_ignore_ascii_case(&cfg.shuttle_pw) {
                        return Ok(false);
                    }
                    self.log("Illegal system password at the matrix");
                    tries += 1;
                }
                2 => {
                    if self.pynq(&cfg.strings.logon_new, false)? {
                        self.new_user("")?;
                        return Ok(true);
                    }
                }
                3 => {
                    self.sprompt(&cfg.strings.enter_handle)?;
                    let name = self.input_caps(36)?;
                    self.sprompt(&cfg.strings.enter_pw)?;
                    let pw = self.input_pw(20)?;
                    let found = {
                        let db = self.sh.db();
                        db.find_user(&name).map(|n| (n, db.users[n - 1].clone()))
                    };
                    match found {
                        Some((n, u)) if db::check_login(&u.pw_hash, &pw) && !u.locked_out => {
                            if u.sl <= cfg.new_sl {
                                if !self.printf("notval")? {
                                    self.sprint("|U1You have not been validated yet.")?;
                                }
                                continue;
                            }
                            self.log(&format!("{} found out the matrix password", u.name));
                            if !self.printf("val")? && !cfg.shuttle_pw.is_empty() {
                                self.sprint(&format!("|U1System password: |U3{}", cfg.shuttle_pw))?;
                            }
                            if self.pynq("|U1Would you like to log on now? ", true)? {
                                self.user = u;
                                self.unum = n;
                                return Ok(true);
                            }
                        }
                        _ => {
                            self.sprint("|12Illegal logon.")?;
                            tries += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn new_user(&mut self, typed: &str) -> io::Result<()> {
        let sh = self.sh.clone();
        let cfg = sh.cfg();
        if cfg.closed_system {
            if !self.printf("nonewusr")? {
                self.sprint("|12Sorry, this system is closed to new users.")?;
            }
            return Err(hangup());
        }
        if !cfg.new_user_password.is_empty() {
            let mut tries = 0;
            loop {
                self.sprompt(&cfg.strings.new_user_pw)?;
                if self.input_pw(20)?.eq_ignore_ascii_case(&cfg.new_user_password) {
                    break;
                }
                tries += 1;
                if tries >= cfg.max_logon_tries {
                    self.log("Bad new user password");
                    return Err(hangup());
                }
            }
        }
        self.cls()?;
        if self.printf("newuinfo")? {
            self.pause()?;
        }
        let mut u = User { ar: cfg.new_ar.to_uppercase(), ..User::default() };
        let mut name = typed.to_string();
        loop {
            if name.is_empty() {
                self.sprompt("|NL|U1Enter the handle you want to use: |U3")?;
                name = self.input_caps(36)?;
            }
            let bad = name.is_empty() || name == "NEW" || name.parse::<usize>().is_ok() || name.contains(['|', '%', '^', ';', '~']);
            if bad || sh.db().find_user(&name).is_some() {
                if !name.is_empty() {
                    self.sprint("|12That handle can't be used. Try another.")?;
                }
                name.clear();
                continue;
            }
            break;
        }
        u.name = name;
        for q in &cfg.new_user_order {
            match q.as_str() {
                "real_name" => u.real_name = self.ask("|U1Real name: |U3", 36)?,
                "city" => u.city_state = self.ask("|U1City, State: |U3", 30)?,
                "phone" => u.phone = self.ask("|U1Phone number: |U3", 12)?,
                "birthday" => loop {
                    u.birthday = self.ask("|U1Birthday (MM/DD/YYYY): |U3", 10)?;
                    if NaiveDate::parse_from_str(&u.birthday, "%m/%d/%Y").is_ok() {
                        break;
                    }
                },
                "sex" => {
                    self.sprompt("|U1Sex (M/F): |U3")?;
                    u.sex = self.onek("MF")?.to_string();
                }
                "ansi" => u.ansi = self.pynq("|U1Does your terminal support ANSI color? ", true)?,
                _ => {}
            }
        }
        u.fse = u.ansi;
        loop {
            self.sprompt("|U1Choose a password (4+ chars): |U3")?;
            let p1 = self.input_pw(20)?;
            self.sprompt("|U1Enter it again: |U3")?;
            let p2 = self.input_pw(20)?;
            if p1.chars().count() >= 4 && p1 == p2 {
                u.pw_hash = db::hash_pw(&p1);
                break;
            }
            self.sprint("|12Passwords didn't match or were too short.")?;
        }
        u.first_on = today();
        let n = {
            let mut db = sh.db();
            if db.find_user(&u.name).is_some() {
                drop(db);
                self.sprint("|12Someone just took that handle. Please call back.")?;
                return Err(hangup());
            }
            // The first account is the SysOp, like Impulse's install.
            (u.sl, u.dsl) = if db.users.is_empty() { (255, 255) } else { (cfg.new_sl, cfg.new_dsl) };
            db.users.push(u.clone());
            db.sys.zlog().new_users += 1;
            db.save()?;
            db.users.len()
        };
        self.user = u;
        self.unum = n;
        self.log(&format!("New user #{n} created"));
        self.sprint(&format!("|NL|U1Welcome aboard, |U3{}|U1! You are user |U3#{n}|U1.", self.user.name))?;
        if self.user.ansi {
            self.obv_config()?;
        }
        if find_ci(&cfg.text_path, "NEWUSER.INF").is_some() {
            self.questionnaire("newuser")?;
        }
        if cfg.auto_validate_new {
            let mut u = self.user.clone();
            self.auto_validate(&mut u);
            self.user = u;
            self.save_user();
        } else if cfg.use_nuv && self.user.sl <= cfg.new_sl {
            self.nuv_add()?;
        }
        Ok(())
    }

    fn logon(&mut self) -> io::Result<()> {
        let today = today();
        if self.user.last_on != today {
            self.user.on_today = 0;
            self.user.minutes_today = self.cfg().minutes_per_day;
        }
        if self.user.minutes_today <= 0 && !self.is_sysop() {
            if !self.printf("notlefta")? {
                self.sprint("|12You have no time left today.")?;
            }
            return Err(hangup());
        }
        self.event_checks()?;
        self.user.on_today += 1;
        self.user.logons += 1;
        self.new_date = std::mem::replace(&mut self.user.last_on, today);
        self.fbase = self.user.last_fbase.min(self.cfg().fbases.len().saturating_sub(1));
        self.board = self.user.last_board.min(self.cfg().boards.len().saturating_sub(1));
        self.on_since = Instant::now();
        {
            let mut db = self.sh.db();
            db.sys.callers += 1;
            db.sys.zlog().calls += 1;
            let lc = LastCaller { caller_num: db.sys.callers, name: self.user.name.clone(), city_state: self.user.city_state.clone(), when: now() };
            db.sys.last_callers.insert(0, lc);
            db.sys.last_callers.truncate(10);
            let o = db.online.entry(self.node).or_default();
            (o.name, o.doing, o.unum) = (self.user.name.clone(), "Logging on".into(), self.unum);
        }
        self.save_user();
        self.log("Logon");
        if self.printf("logon")? {
            self.pause()?;
        }
        if self.user.prompt == 0 && !self.cfg().prompts.is_empty() {
            self.choose_prompt()?;
        }
        if self.nuv_pending() {
            self.new_user_vote()?;
        }
        let waiting = self.sh.db().msgs("email")?.iter().filter(|m| m.to_num == self.unum).count();
        if waiting > 0 && self.pynq(&format!("|NL|U1You have |U3{waiting}|U1 letter(s) waiting. Read them now? "), true)? {
            self.read_mail()?;
        }
        Ok(())
    }

    /// Restricted-time `A` events and the event time chop (getacsuser, findchoptime).
    fn event_checks(&mut self) -> io::Result<()> {
        let now = Local::now().naive_local();
        let cfg = self.cfg();
        for (i, e) in cfg.events.iter().enumerate() {
            if e.kind == "A" && events::in_window(e, now) && !self.acs(&e.data) {
                if !self.printf(&format!("acsea{}", i + 1))? {
                    self.sprint("|NL|12Restricted time zone.|NLOnly certain users allowed online at this time.")?;
                }
                self.log(&format!("Refused during event \"{}\"", e.description));
                return Err(hangup());
            }
        }
        let left = self.time_left_min();
        let soon = cfg.events.iter().filter(|e| e.active && e.kind != "A").filter_map(|e| events::minutes_until(e, now, left)).min();
        if let Some(m) = soon
            && !self.is_sysop()
        {
            let m = (m - 1).max(1);
            self.chop = Some(Instant::now() + Duration::from_secs(m as u64 * 60));
            self.sprint(&format!("|NL|U5- System event approaching - online time adjusted to |U3{m}|U5 minutes."))?;
        }
        Ok(())
    }

    fn logoff(&mut self) {
        let _ = self.t.flush();
        if self.unum > 0 {
            let used = (self.on_since.elapsed().as_secs() / 60) as i64;
            self.user.minutes_today = (self.user.minutes_today - used).max(0);
            self.user.total_minutes += used;
            self.sh.db().sys.zlog().active += used;
            self.save_user();
            self.log(&format!("Logoff after {used} min"));
        }
        self.sh.db().online.remove(&self.node);
    }

    // ---- menus ------------------------------------------------------------

    fn load_menu(&self, name: &str) -> Option<Menu> {
        let read = |n: &str| {
            let p = find_ci(&self.cfg().menu_path, &format!("{n}.MNU"))?;
            let bytes = std::fs::read(p).ok()?;
            Some(menu::parse(&bytes.iter().map(|&b| b as char).collect::<String>()))
        };
        let mut m = read(name)?;
        if !m.flags.contains('G')
            && let Some(g) = read("GLOBAL")
        {
            m.cmds.extend(g.cmds);
        }
        Some(m)
    }

    /// Switch menus; Flow::Next means it failed and we stayed put.
    pub fn goto_menu(&mut self, name: &str) -> io::Result<Flow> {
        let Some(m) = self.load_menu(name) else {
            self.log(&format!("Menu {name} is MISSING"));
            self.sprint(&format!("|NL|12Menu \"{name}\" is missing. Please inform the SysOp."))?;
            return Ok(Flow::Next);
        };
        if !self.acs(&m.acs) {
            self.sprint("|NL|12Access denied.")?;
            return Ok(Flow::Next);
        }
        if !m.password.is_empty() {
            self.sprompt("|U1Password: |U3")?;
            if self.input_pw(15)?.to_uppercase() != m.password {
                self.sprint("|NL|12Access denied.")?;
                return Ok(Flow::Next);
            }
        }
        let first = m.cmds.iter().any(|c| c.keys == "FIRSTCMD" && self.acs(&c.acs));
        self.menu = m;
        self.menu_name = name.to_uppercase();
        Ok(Flow::Menu(first.then(|| "FIRSTCMD".to_string())))
    }

    /// `-^` goto, `-/` gosub, `-\` return (dochangemenu in MENUS3.PAS).
    /// Data is `MENU;C;CMD`: C clears the menu stack, CMD runs on arrival.
    pub fn change_menu(&mut self, kind: char, data: &str) -> io::Result<Flow> {
        let parts: Vec<&str> = data.split(';').collect();
        let (name, flag, then) = if kind == '\\' {
            let name = self.menu_stack.last().cloned().unwrap_or_else(|| self.menu_name.clone());
            (name, parts.first().copied().unwrap_or(""), parts.get(1).copied())
        } else {
            (parts[0].trim().to_string(), parts.get(1).copied().unwrap_or(""), parts.get(2).copied())
        };
        if name.is_empty() {
            return Ok(Flow::Next);
        }
        let from = self.menu_name.clone();
        let flow = self.goto_menu(&name)?;
        let Flow::Menu(first) = flow else {
            return Ok(flow);
        };
        match kind {
            '/' if self.menu_stack.len() < 8 => self.menu_stack.push(from),
            '\\' => {
                self.menu_stack.pop();
            }
            _ => {}
        }
        if flag.starts_with(['C', 'c']) {
            self.menu_stack.clear();
        }
        let then = then.map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty());
        Ok(Flow::Menu(then.or(first)))
    }

    fn show_menu_help(&mut self) -> io::Result<()> {
        let help = self.menu.help.clone();
        if self.printf(&help)? {
            return Ok(());
        }
        // No help file: list the commands, like the old generic menus.
        if self.menu.flags.contains('C') {
            self.cls()?;
        }
        self.nl()?;
        for t in self.menu.title.clone().iter().filter(|t| !t.is_empty()) {
            let pad = 40usize.saturating_sub(strip_pipes(t).chars().count() / 2);
            let line = if self.menu.flags.contains('D') { t.clone() } else { format!("{}{t}", " ".repeat(pad)) };
            self.sprint(&line)?;
        }
        self.nl()?;
        let cmds: Vec<Command> = self.menu.cmds.clone();
        let mut col = 0;
        for c in cmds.iter().filter(|c| !c.ldesc.is_empty() && !c.flags.contains('H')) {
            if !(c.flags.contains('U') || self.acs(&c.acs)) {
                continue;
            }
            let w = strip_pipes(&c.ldesc).chars().count();
            self.sprompt(&format!("|U1{}", c.ldesc))?;
            col += 1;
            if col % 2 == 0 || w >= 38 {
                col = 0;
                self.nl()?;
            } else {
                self.out(&" ".repeat(39 - w))?;
            }
        }
        if col != 0 {
            self.nl()?;
        }
        Ok(())
    }

    fn menu_prompt(&mut self) -> io::Result<String> {
        self.check_reload();
        self.show_node_msgs()?;
        let every: Vec<Command> = self.menu.cmds.iter().filter(|c| c.keys == "EVERYTIME").cloned().collect();
        for c in every {
            if self.acs(&c.acs) {
                self.do_cmd(&c.cmd, &c.data)?;
            }
        }
        self.set_doing(&format!("Menu: {}", self.menu_name));
        if std::mem::take(&mut self.ran_cmd) && self.menu.flags.contains('F') {
            self.pause()?;
        }
        if self.menu.flags.contains('P')
            && self.user.ansi
            && let Some(keys) = self.pulldown()?
        {
            return Ok(keys);
        }
        if self.help || self.user.novice {
            self.help = false;
            self.show_menu_help()?;
        }
        if !self.menu.flags.contains('N') {
            if self.menu.flags.contains('T') {
                self.sprint("|U2Time Left: |TL")?;
            }
            let cfg = self.cfg();
            let p = match cfg.prompts.get(usize::from(self.user.prompt).wrapping_sub(1)) {
                Some(p) if self.user.prompt <= 4 => p.prompt.clone(),
                _ => self.menu.prompt.clone(),
            };
            match p.strip_prefix("~~") {
                Some(f) => {
                    self.printf(f)?;
                }
                None => self.sprompt(&p)?,
            }
        }
        if self.wait_key_or_chat()? {
            return Ok(String::new());
        }
        let cmd = self.getcmd()?;
        self.ran_cmd = !cmd.is_empty();
        if cmd == "?" {
            self.help = true;
        }
        if !cmd.is_empty() && cmd.chars().all(|c| c.is_ascii_digit()) {
            if self.menu.cmds.iter().any(|c| c.cmd == "M#") {
                self.board_by_number(&cmd)?;
                return Ok(String::new());
            }
            if self.menu.cmds.iter().any(|c| c.cmd == "F#") {
                self.fbase_by_number(&cmd)?;
                return Ok(String::new());
            }
        }
        Ok(cmd)
    }

    /// Lightbar for pulldown (`P`) menus: each command with a short
    /// description and an X/Y position is drawn there; arrows move, Enter or
    /// a command's own key picks. None if the menu has nothing to draw.
    // ponytail: DoPulls isn't in the 7.1 source, so the look is our own.
    fn pulldown(&mut self) -> io::Result<Option<String>> {
        let pos = |c: &Command| Some((c.x.parse::<u16>().ok().filter(|&x| x > 0)?, c.y.parse::<u16>().ok().filter(|&y| y > 0)?));
        let items: Vec<(Command, (u16, u16))> = self
            .menu
            .cmds
            .clone()
            .into_iter()
            .filter(|c| !c.sdesc.is_empty() && !c.flags.contains('H') && (c.flags.contains('U') || self.acs(&c.acs)))
            .filter_map(|c| pos(&c).map(|p| (c, p)))
            .collect();
        if items.is_empty() {
            return Ok(None);
        }
        let help = self.menu.help.clone();
        if !self.printf(&help)? {
            self.cls()?;
            for t in self.menu.title.clone().iter().filter(|t| !t.is_empty()) {
                self.sprint(t)?;
            }
        }
        let draw = |s: &mut Session, i: usize, on: bool| -> io::Result<()> {
            let (c, (x, y)) = &items[i];
            s.out(&format!("\x1b[{y};{x}H"))?;
            if on {
                s.setc(0x1f)?
            } else {
                s.cl(1)?
            }
            s.out(&strip_pipes(&c.sdesc))?;
            if on {
                s.setc(7)?;
                s.out(&format!("\x1b[24;1H\x1b[K{}", strip_pipes(&c.ldesc)))?;
            }
            Ok(())
        };
        for i in 0..items.len() {
            draw(self, i, false)?;
        }
        let mut cur = 0;
        loop {
            draw(self, cur, true)?;
            let k = self.t.getkey_ext()?;
            let next = match k {
                term::UP | term::LEFT => (cur + items.len() - 1) % items.len(),
                term::DOWN | term::RIGHT => (cur + 1) % items.len(),
                term::HOME => 0,
                term::END => items.len() - 1,
                '\r' => break,
                _ => {
                    let k = k.to_ascii_uppercase().to_string();
                    if self.menu.cmds.iter().any(|c| c.keys == k && self.acs(&c.acs)) {
                        self.setc(7)?;
                        self.out("\x1b[24;1H\x1b[K")?;
                        self.ran_cmd = true;
                        return Ok(Some(k));
                    }
                    cur
                }
            };
            draw(self, cur, false)?;
            cur = next;
        }
        self.setc(7)?;
        self.out("\x1b[24;1H\x1b[K")?;
        self.ran_cmd = true;
        Ok(Some(items[cur].0.keys.clone()))
    }

    /// Read a command: whole line normally, hot keys in one-key mode
    /// (getcmd in MENUS.PAS: `/X` two-key commands, `//TEXT` long commands).
    fn getcmd(&mut self) -> io::Result<String> {
        if !self.user.hotkeys {
            return self.input_caps(60);
        }
        let mut singles = String::from("?");
        let mut slash = String::new();
        let mut long = false;
        for c in &self.menu.cmds {
            if !self.acs(&c.acs) {
                continue;
            }
            let k: Vec<char> = c.keys.chars().collect();
            match k.as_slice() {
                [a] => singles.push(*a),
                ['/', b] => slash.push(*b),
                _ => long = true,
            }
        }
        let areas = self.menu.cmds.iter().any(|c| c.cmd == "M#" || c.cmd == "F#");
        let mut ss = String::new();
        loop {
            let c = self.getkey()?.to_ascii_uppercase();
            let bs = c == '\x08' || c == '\x7f';
            if ss.is_empty() {
                if c == '\r' {
                    self.nl()?;
                    return Ok(ss);
                } else if c == '/' && (!slash.is_empty() || long) {
                    ss.push(c);
                    self.out("/")?;
                } else if areas && c.is_ascii_digit() {
                    ss.push(c);
                    self.out(&c.to_string())?;
                } else if singles.contains(c) {
                    self.out(&c.to_string())?;
                    self.nl()?;
                    return Ok(c.to_string());
                }
            } else if ss == "/" {
                if bs {
                    ss.clear();
                    self.out("\x08 \x08")?;
                } else if c == '/' && long {
                    ss.push(c);
                    self.out("/")?;
                } else if slash.contains(c) {
                    self.out(&c.to_string())?;
                    self.nl()?;
                    return Ok(format!("/{c}"));
                }
            } else if let Some(rest) = ss.strip_prefix("//") {
                if c == '\r' {
                    let r = rest.to_string();
                    self.nl()?;
                    return Ok(r);
                } else if bs {
                    ss.pop();
                    self.out("\x08 \x08")?;
                } else if c >= ' ' && c != '\x7f' && ss.len() < 62 {
                    ss.push(c);
                    self.out(&c.to_string())?;
                }
            } else {
                // area number
                if c == '\r' {
                    self.nl()?;
                    return Ok(ss);
                } else if bs {
                    ss.pop();
                    self.out("\x08 \x08")?;
                } else if c.is_ascii_digit() {
                    ss.push(c);
                    self.out(&c.to_string())?;
                    if ss.len() == 3 {
                        self.nl()?;
                        return Ok(ss);
                    }
                }
            }
        }
    }

    /// Prompt through a READP/EMAILP/FILEP-style menu whose `@x`, `$x` and `!x`
    /// commands hand a key back (mainmenuhandle + readpmnu). Without the menu
    /// file a one-key prompt over `keys` stands in. Enter means 'N'.
    pub fn sub_prompt(&mut self, menu: &str, prompt: &str, keys: &str) -> io::Result<char> {
        let Some(m) = self.load_menu(menu) else {
            self.sprompt(prompt)?;
            let c = self.onek(&format!("{keys}\r"))?;
            return Ok(if c == '\r' { 'N' } else { c });
        };
        let old_menu = std::mem::replace(&mut self.menu, m);
        let old_name = std::mem::replace(&mut self.menu_name, menu.to_uppercase());
        self.pkey = None;
        let r = self.sub_loop();
        self.menu = old_menu;
        self.menu_name = old_name;
        r?;
        Ok(self.pkey.take().unwrap_or('Q'))
    }

    fn sub_loop(&mut self) -> io::Result<()> {
        while self.pkey.is_none() {
            self.check_time()?;
            let cmd = self.menu_prompt()?;
            if cmd.is_empty() && !self.menu.cmds.iter().any(|c| c.keys == "ENTER") {
                self.pkey = Some('N');
            } else {
                self.exec(&cmd)?;
            }
        }
        Ok(())
    }

    /// Run every accessible command bound to `keys` (domenuexec).
    pub fn exec(&mut self, keys: &str) -> io::Result<()> {
        let mut keys = if keys.is_empty() { "ENTER".to_string() } else { keys.to_string() };
        'outer: loop {
            let cmds: Vec<Command> = self.menu.cmds.iter().filter(|c| c.keys == keys).cloned().collect();
            let mut ran = false;
            for c in &cmds {
                if !self.acs(&c.acs) {
                    continue;
                }
                ran = true;
                match self.do_cmd(&c.cmd, &c.data)? {
                    Flow::Next => {}
                    Flow::Stop => break,
                    Flow::Menu(Some(next)) => {
                        keys = next;
                        continue 'outer;
                    }
                    Flow::Menu(None) => break,
                }
            }
            if !ran && cmds.iter().any(|c| c.flags.contains('U')) {
                self.sprint("|NL|12You don't have enough access for this command.")?;
            }
            return Ok(());
        }
    }
}

/// Age in years from MM/DD/YYYY, 0 if unparseable.
pub fn age(bday: &str) -> i64 {
    let Ok(b) = NaiveDate::parse_from_str(bday, "%m/%d/%Y") else {
        return 0;
    };
    let t = Local::now().date_naive();
    let mut a = t.year() - b.year();
    if (t.month(), t.day()) < (b.month(), b.day()) {
        a -= 1;
    }
    a as i64
}

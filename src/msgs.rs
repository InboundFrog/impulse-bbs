//! Message bases and e-mail (MAIL*.PAS).

use crate::config::{Anon, Board};
use crate::db::Msg;
use crate::session::{Session, now, strip_pipes};
use std::io;

fn re(s: &str) -> String {
    if s.starts_with("Re:") { s.to_string() } else { format!("Re: {s}") }
}

/// Quote prefix from a name's initials: "Joe Bloggs" -> " JB> ".
fn quote_prefix(name: &str) -> String {
    let ini: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    format!(" {}> ", ini.to_uppercase())
}

const READ_HELP: &[&str] = &[
    "|NL|U1(|U3CR|U1) Next      (|U3-|U1) Previous   (|U3J|U1)ump      (|U3A|U1)gain    (|U3T|U1)itles",
    "|U1(|U3R|U1)eply      (|U3W|U1)rite public reply  (|U3P|U1)ost   (|U3D|U1)elete",
    "|U1(|U3B|U1) Next base (|U3H|U1)igh pointer  (|U3Z|U1) Toggle newscan  (|U3Q|U1)uit",
];
const READ_HELP_MSO: &str = "|U1SysOp: (|U3V|U1)alidate (|U3M|U1)ove (|U3X|U1)tract (|U3!|U1) Permanent (|U3*|U1) Anonymous";
const MAIL_HELP: &[&str] = &[
    "|NL|U1(|U3CR|U1) Next  (|U3-|U1) Previous  (|U3J|U1)ump  (|U3A|U1)gain  (|U3T|U1)itles",
    "|U1(|U3R|U1)eply and delete  (|U3S|U1)end reply, keep  (|U3D|U1)elete  (|U3F|U1)orward  (|U3Q|U1)uit",
];

impl Session {
    // ---- message bases ----------------------------------------------------

    /// Message SysOp (mso).
    pub fn mso(&self) -> bool {
        self.is_sysop() || self.acs(&self.cfg().msop_acs)
    }

    /// Board indexes this user may read.
    pub fn boards(&self) -> Vec<usize> {
        let b = &self.cfg().boards;
        (0..b.len()).filter(|&i| self.acs(&b[i].acs) && (self.mconf == 0 || b[i].conf == 0 || b[i].conf == self.mconf)).collect()
    }

    // ---- conferences (CONF.PAS) ---------------------------------------------

    /// `CM` / `CF`: join a message or file conference; data `A` = all, or a number.
    pub fn change_conf(&mut self, file: bool, data: &str) -> io::Result<()> {
        let confs = if file { self.cfg().file_confs.clone() } else { self.cfg().msg_confs.clone() };
        let n = match data.trim().to_uppercase().as_str() {
            "A" => 0,
            s if s.parse::<usize>().is_ok() => s.parse().unwrap_or(0),
            _ => {
                self.write_hdr(if file { "File Confs" } else { "Msg Confs" })?;
                loop {
                    self.sprompt("|NL|U1Join which conference |U2(|U3?|U2=list, Enter=all)|U1: |U3")?;
                    let s = self.input(2)?;
                    if s != "?" {
                        break s.parse().unwrap_or(0);
                    }
                    self.sprint("|U9--------------------")?;
                    for (i, c) in confs.iter().enumerate() {
                        self.sprompt(&format!("|U0{} - ", i + 1))?;
                        self.out(&c.name)?;
                        self.nl()?;
                    }
                    self.sprint("|U00 - All Conferences|NL|U9--------------------")?;
                }
            }
        };
        let (name, acs) = match n {
            0 => ("All Conferences".to_string(), String::new()),
            _ => match confs.get(n - 1) {
                Some(c) => (c.name.clone(), c.acs.clone()),
                None => return self.sprint("|NL|12No such conference."),
            },
        };
        if !self.acs(&acs) {
            return self.sprint("|NL|12You do not have ACS for that Conference");
        }
        if file {
            self.fconf = n;
        } else {
            self.mconf = n;
            if let Some(&b) = self.boards().first() {
                self.board = b;
            }
        }
        self.sprompt("|NL|U1Joined |U3")?;
        self.out(&name)?;
        self.nl()
    }

    /// `CE`: edit the conference lists.
    pub fn conf_editor(&mut self) -> io::Result<()> {
        if !self.is_sysop() {
            return Ok(());
        }
        self.sprompt("|NL|U1Edit Message or File Conf? |U2[|U3M|U2/|U3F|U2]|U1: |U3")?;
        let file = match self.onek("MFQ\r")? {
            'M' => false,
            'F' => true,
            _ => return Ok(()),
        };
        let confs = if file { self.cfg().file_confs.clone() } else { self.cfg().msg_confs.clone() };
        if let Some(new) = self.edit_vec(if file { "File conferences" } else { "Msg conferences" }, &confs, &[])? {
            self.sh.edit_cfg(|c| if file { c.file_confs = new } else { c.msg_confs = new })?;
        }
        Ok(())
    }

    /// Asks for the base password once per call.
    fn board_ok(&mut self, b: usize) -> io::Result<bool> {
        let board = self.cfg().boards[b].clone();
        if board.password.is_empty() || self.board_pw.contains(&board.file) {
            return Ok(true);
        }
        self.sprompt(&format!("|NL|U3{}|U1 password: |U3", board.name))?;
        if self.input_pw(20)?.eq_ignore_ascii_case(&board.password) {
            self.board_pw.push(board.file);
            return Ok(true);
        }
        self.sprint("|12Wrong password.")?;
        Ok(false)
    }

    pub fn board_by_number(&mut self, n: &str) -> io::Result<()> {
        let list = self.boards();
        match n.parse::<usize>() {
            Ok(n @ 1..) if n <= list.len() => {
                if !self.board_ok(list[n - 1])? {
                    return Ok(());
                }
                self.board = list[n - 1];
                self.user.last_board = self.board;
                self.sprint("|U1Joined |U3|CM")
            }
            _ => self.sprint("|12No such message base."),
        }
    }

    /// The board list (mbaselist); returns the visible board indexes.
    fn list_boards(&mut self) -> io::Result<Vec<usize>> {
        let list = self.boards();
        self.tmpl("msgbase.top", &[], "|NL|U3Message bases")?;
        for (i, &b) in list.iter().enumerate() {
            let scan = if self.user.no_scan.contains(&self.cfg().boards[b].file) { " " } else { "*" };
            let vars = [("ME", self.cfg().boards[b].name.clone()), ("MI", (i + 1).to_string()), ("MT", "Local".into()), ("MZ", scan.into())];
            self.tmpl("msgbase.mid", &vars, "|U2%MI. |U3%MZ|U1%ME")?;
        }
        self.tmpl("msgbase.bot", &[], "")?;
        Ok(list)
    }

    pub fn change_board(&mut self, data: &str) -> io::Result<()> {
        let list = self.boards();
        if list.is_empty() {
            return self.sprint("|NL|12No message bases available.");
        }
        let pos = list.iter().position(|&b| b == self.board).unwrap_or(0);
        let n = match data.trim() {
            "+" => (pos + 1) % list.len() + 1,
            "-" => (pos + list.len() - 1) % list.len() + 1,
            _ => {
                self.list_boards()?;
                self.sprompt("|NL|U1Change to which base? |U3")?;
                match self.input(3)?.parse() {
                    Ok(n) => n,
                    Err(_) => return Ok(()),
                }
            }
        };
        self.board_by_number(&n.to_string())
    }

    /// `MZ` (chbds): choose which bases newscan reads.
    pub fn newscan_boards(&mut self) -> io::Result<()> {
        loop {
            let list = self.list_boards()?;
            self.sprompt("|NL|U1Toggle newscan of which base |U2(|U3*|U2=scanned, Enter quits)|U1? |U3")?;
            match self.input(3)?.parse::<usize>() {
                Ok(n @ 1..) if n <= list.len() => self.toggle_scan(list[n - 1])?,
                _ => return Ok(()),
            }
        }
    }

    fn toggle_scan(&mut self, b: usize) -> io::Result<()> {
        let board = self.cfg().boards[b].clone();
        self.sprompt(&format!("|NL|U3{}", board.name))?;
        if let Some(i) = self.user.no_scan.iter().position(|f| *f == board.file) {
            self.user.no_scan.remove(i);
            self.log(&format!("* Put \"{}\" back in NewScan", board.name));
            self.sprint("|U1 WILL be scanned in future NewScans.")
        } else {
            self.user.no_scan.push(board.file);
            self.log(&format!("* Took \"{}\" out of NewScan", board.name));
            self.sprint("|U1 will NOT be scanned in future NewScans.")
        }
    }

    /// `M$` (mbasestats): the current base's settings through MBSTATS.
    pub fn board_stats(&mut self) -> io::Result<()> {
        let b = self.cfg().boards.get(self.board).cloned().unwrap_or_default();
        let num = self.boards().iter().position(|&x| x == self.board).map_or(0, |i| i + 1);
        let anon = match b.anonymous {
            Anon::No => "None",
            Anon::Yes => "Allowed",
            Anon::Forced => "All",
            Anon::DearAbby => "Dear Abby",
            Anon::AnyName => "Any Name",
        };
        let or_none = |s: &str| if s.is_empty() { "None".to_string() } else { s.to_string() };
        let vars = [
            ("MI", num.to_string()),
            ("ME", b.name.clone()),
            ("MW", or_none(&b.password)),
            ("MM", if b.max_msgs == 0 { "No limit".into() } else { b.max_msgs.to_string() }),
            ("MA", anon.into()),
            ("MS", or_none(&b.acs)),
            ("MP", or_none(&b.post_acs)),
            ("MC", or_none(&b.mci_acs)),
            ("MF", format!("{}.json", b.file)),
            ("MT", "Local".into()),
        ];
        self.tmpl(
            "mbstats",
            &vars,
            "|NL|U1Base |U3%MI|U1: |U3%ME|NL|U1Access: |U3%MS  |U1Post: |U3%MP  |U1MCI: |U3%MC|NL|U1Password: |U3%MW  |U1Max messages: |U3%MM  |U1Anonymous: |U3%MA|NL|U1Type: |U3%MT  |U1File: |U3%MF",
        )
    }

    // ---- reading ------------------------------------------------------------

    /// Stand-in name for an anonymous sender (what00).
    fn anon_name(m: &Msg) -> Option<&str> {
        match m.anon {
            1 | 2 => Some("Anonymous"),
            3 => Some("Abby"),
            4 => Some("Problemed Person"),
            5 => Some(&m.alias),
            _ => None,
        }
    }

    /// Whether this user sees through `m`'s anonymity.
    fn sees_anon(&self, m: &Msg, private: bool) -> bool {
        let c = self.cfg();
        if m.anon == 2 { self.is_sysop() } else { self.acs(if private { &c.anon_priv_read } else { &c.anon_pub_read }) }
    }

    /// Sender as this user may see it.
    pub fn sender(&self, m: &Msg, private: bool) -> String {
        match Self::anon_name(m) {
            None => m.from.clone(),
            Some(fake) if self.sees_anon(m, private) => format!("{} ({fake})", m.from),
            Some(fake) => fake.to_string(),
        }
    }

    pub fn show_msg(&mut self, area: &str, n: usize, total: usize, m: &Msg, private: bool) -> io::Result<()> {
        if self.user.ansi {
            self.cls()?;
        } else {
            self.nl()?;
        }
        self.sprint(&format!("|U1Msg |U3{n}|U1 of |U3{total} |U1in |U3{area}"))?;
        let from = self.sender(m, private);
        self.out_field("From", &from)?;
        self.out_field("To", &m.to)?;
        self.out_field("Subj", &m.subject)?;
        self.out_field("Date", &m.date)?;
        if m.unvalidated || m.permanent {
            let f = [(m.unvalidated, "Unvalidated"), (m.permanent, "Permanent")];
            let s: Vec<&str> = f.iter().filter(|x| x.0).map(|x| x.1).collect();
            self.sprint(&format!("|12[{}]", s.join(", ")))?;
        }
        self.nl()?;
        for l in &m.body {
            self.sprint(&format!("|U0{l}"))?;
        }
        Ok(())
    }

    /// Header field, printed raw so names can't inject pipe codes.
    pub fn out_field(&mut self, label: &str, v: &str) -> io::Result<()> {
        self.sprompt(&format!("|U1{label:<5}: |U3"))?;
        self.out(v)?;
        self.nl()
    }

    /// Subject list from message `from` on.
    fn titles(&mut self, msgs: &[Msg], from: usize, private: bool) -> io::Result<()> {
        self.sprint("|NL|U3   # |U1Subject                                  From")?;
        for (n, m) in msgs.iter().enumerate().skip(from).take(20) {
            self.sprompt(&format!("|U3{:>4} |U1", n + 1))?;
            let subj: String = m.subject.chars().take(40).collect();
            let from = self.sender(m, private);
            self.out(&format!("{subj:<40} {from}"))?;
            self.nl()?;
        }
        Ok(())
    }

    fn jump(&mut self, total: usize) -> io::Result<Option<usize>> {
        self.sprompt(&format!("|NL|U1Jump to (1-{total}): |U3"))?;
        Ok(match self.input(5)?.parse::<usize>() {
            Ok(n @ 1..) if n <= total => Some(n - 1),
            _ => None,
        })
    }

    /// `X`: append a message to a text file in the data directory.
    fn extract(&mut self, m: &Msg) -> io::Result<()> {
        self.sprompt("|NL|U1Extract filename? (default=\"EXT.TXT\") : |U3")?;
        let s = self.input(40)?;
        // keep it inside data_path
        let name = std::path::Path::new(if s.is_empty() { "EXT.TXT" } else { &s }).file_name().map(|n| n.to_string_lossy().to_string());
        let Some(name) = name else { return Ok(()) };
        if !self.pynq("|U1Are you sure? ", true)? {
            return Ok(());
        }
        let strip = self.pynq("|U1Strip color codes from output? ", true)?;
        let mut text = format!("From: {}\nTo: {}\nSubj: {}\nDate: {}\n\n", m.from, m.to, m.subject, m.date);
        for l in &m.body {
            text += &if strip { strip_pipes(l) } else { l.clone() };
            text.push('\n');
        }
        let p = self.cfg().data_path.join(name);
        use std::io::Write;
        std::fs::OpenOptions::new().create(true).append(true).open(p)?.write_all(text.as_bytes())?;
        self.sprint("|U1Done!")
    }

    fn move_msg(&mut self, from: usize, m: &Msg) -> io::Result<bool> {
        let list = self.list_boards()?;
        self.sprompt("|NL|U1Move to which base? |U3")?;
        let to = match self.input(3)?.parse::<usize>() {
            Ok(n @ 1..) if n <= list.len() && list[n - 1] != from => list[n - 1],
            _ => return Ok(false),
        };
        let (src, dst) = { let c = self.cfg(); (c.boards[from].clone(), c.boards[to].clone()) };
        {
            let mut db = self.sh.db();
            db.delete_msg(&src.file, m.id)?;
            db.post(&dst.file, m.clone(), dst.max_msgs)?;
        }
        self.log(&format!("* Moved \"{}\" to {}", m.subject, dst.name));
        self.sprint(&format!("|U1Moved to |U3{}|U1.", dst.name))?;
        Ok(true)
    }

    /// Read a board (doscan), from the first unread message. Returns true if
    /// the user quit.
    pub fn read_board(&mut self, b: usize, new_only: bool) -> io::Result<bool> {
        if !self.board_ok(b)? {
            return Ok(false);
        }
        let board = self.cfg().boards[b].clone();
        let mso = self.mso();
        let load = |s: &Self| -> io::Result<Vec<Msg>> {
            Ok(s.sh.db().msgs(&board.file)?.into_iter().filter(|m| !m.unvalidated || mso || m.from_num == s.unum).collect())
        };
        let mut msgs = load(self)?;
        let last = self.user.last_read.get(&board.file).copied().unwrap_or(0);
        let mut i = match msgs.iter().position(|m| m.id > last) {
            Some(i) => i,
            None if new_only => return Ok(false),
            None if msgs.is_empty() => {
                self.sprint(&format!("|NL|U1No messages in |U3{}|U1.", board.name))?;
                return self.ask_post_here(b).map(|_| false);
            }
            None => match self.jump(msgs.len())? {
                Some(n) => n,
                None => return Ok(false),
            },
        };
        self.set_doing(&format!("Reading {}", board.name));
        let prompt = self.cfg().strings.read_prompt.clone();
        let mut show = true;
        loop {
            if i >= msgs.len() {
                self.sprint("|U1End of messages.")?;
                break;
            }
            let m = msgs[i].clone();
            if show {
                self.show_msg(&board.name, i + 1, msgs.len(), &m, false)?;
                let read = self.user.last_read.entry(board.file.clone()).or_default();
                *read = (*read).max(m.id);
            }
            show = false;
            match self.sub_prompt("READP", &prompt, "NA-JRWPBQTHZDVM!*X?")? {
                'N' => {
                    i += 1;
                    show = true;
                }
                'A' => show = true,
                '-' if i == 0 => self.sprint("|NL|U1Already at the first message.")?,
                '-' => {
                    i -= 1;
                    show = true;
                }
                'J' => {
                    if let Some(n) = self.jump(msgs.len())? {
                        i = n;
                        show = true;
                    }
                }
                'R' => {
                    if self.pynq("|NL|U1Is this to be a private reply? ", false)? {
                        self.reply_private(&m, false)?;
                    } else {
                        self.post(b, Some(&m))?;
                    }
                }
                'W' => self.post(b, Some(&m))?,
                'P' => self.post(b, None)?,
                'B' => return Ok(false),
                'Q' => return Ok(true),
                'T' => self.titles(&msgs, i, false)?,
                'H' => {
                    self.user.last_read.insert(board.file.clone(), m.id);
                    self.sprint(&format!("|NL|U1Highest-read pointer for this base set to message #{}.", i + 1))?;
                }
                'Z' => self.toggle_scan(b)?,
                'D' if m.permanent => self.sprint("|NL|U1This is a permanent message.")?,
                'D' if m.from_num == self.unum || self.is_sysop() => {
                    self.sh.db().delete_msg(&board.file, m.id)?;
                    self.log(&format!("* Deleted \"{}\"", m.subject));
                    self.sprint("|NL|U1Deleted message.")?;
                    msgs = load(self)?;
                    show = true;
                }
                'D' => self.sprint("|NL|U1You didn't post that!")?,
                'V' if mso => {
                    let v = !m.unvalidated;
                    self.sh.db().update_msg(&board.file, m.id, |x| x.unvalidated = v)?;
                    msgs[i].unvalidated = v;
                    self.log(&format!("* {} \"{}\"", if v { "Unvalidated" } else { "Validated" }, m.subject));
                    self.sprint(if v { "|NL|U1Message unvalidated." } else { "|NL|U1Message validated." })?;
                }
                '!' if mso => {
                    let v = !m.permanent;
                    self.sh.db().update_msg(&board.file, m.id, |x| x.permanent = v)?;
                    msgs[i].permanent = v;
                    self.log(&format!("* \"{}\" made {}", m.subject, if v { "permanent" } else { "unpermanent" }));
                    self.sprint(if v { "|NL|U1Message is now permanent." } else { "|NL|U1Message is no longer permanent." })?;
                }
                '*' if self.is_sysop() => {
                    let a = match m.anon {
                        0 => {
                            let db = self.sh.db();
                            let poster = db.users.get(m.from_num.wrapping_sub(1)).cloned();
                            drop(db);
                            let sysop_acs = self.cfg().sysop_acs.clone();
                            if poster.is_some_and(|u| self.acs_for(&u, m.from_num, &sysop_acs)) { 2 } else { 1 }
                        }
                        _ => 0,
                    };
                    self.sh.db().update_msg(&board.file, m.id, |x| x.anon = a)?;
                    msgs[i].anon = a;
                    self.log(&format!("* \"{}\" made {}", m.subject, if a == 0 { "non-anonymous" } else { "anonymous" }));
                    self.sprint(if a == 0 { "|NL|U1Message is no longer anonymous." } else { "|NL|U1Message is now anonymous." })?;
                }
                'M' if mso => {
                    if self.move_msg(b, &m)? {
                        msgs = load(self)?;
                        show = true;
                    }
                }
                'X' if mso => self.extract(&m)?,
                '?' => {
                    for l in READ_HELP {
                        self.sprint(l)?;
                    }
                    if mso {
                        self.sprint(READ_HELP_MSO)?;
                    }
                }
                _ => {}
            }
        }
        self.ask_post_here(b)?;
        Ok(false)
    }

    fn ask_post_here(&mut self, b: usize) -> io::Result<()> {
        let board = self.cfg().boards[b].clone();
        let q = self.cfg().strings.post_here.clone();
        if self.can_post(&board) && self.pynq(&q, false)? {
            self.post(b, None)?;
        }
        Ok(())
    }

    pub fn newscan(&mut self) -> io::Result<()> {
        for b in self.boards() {
            let board = self.cfg().boards[b].clone();
            if self.user.no_scan.contains(&board.file) || (!board.password.is_empty() && !self.board_pw.contains(&board.file)) {
                continue;
            }
            self.sprint(&format!("|NL|U1Scanning |U3{}|U1...", board.name))?;
            if self.read_board(b, true)? {
                break;
            }
        }
        self.sprint("|U1Newscan complete.")
    }

    // ---- writing ------------------------------------------------------------

    /// Line editor: `/S` saves, `/A` aborts, `/L` lists, `/Q` quotes.
    // ponytail: no word wrap or full-screen editor yet
    pub fn editor(&mut self, max: usize, quote: Option<&Msg>) -> io::Result<Option<Vec<String>>> {
        let q = if quote.is_some() { ", |U3/Q|U1 quotes" } else { "" };
        self.sprint(&format!("|NL|U1Enter up to |U3{max}|U1 lines. |U3/S|U1 saves, |U3/A|U1 aborts, |U3/L|U1 lists{q}."))?;
        let mut lines: Vec<String> = Vec::new();
        loop {
            self.sprompt(&format!("|U2{:>3}: |U0", lines.len() + 1))?;
            let l = self.input_main(78, false, None)?; // untrimmed: keep indentation
            match l.trim().to_ascii_uppercase().as_str() {
                "/S" => return Ok((!lines.is_empty()).then_some(lines)),
                "/A" => return Ok(None),
                "/L" => {
                    for (i, l) in lines.clone().iter().enumerate() {
                        self.sprompt(&format!("|U2{:>3}: |U0", i + 1))?;
                        self.out(l)?;
                        self.nl()?;
                    }
                }
                "/Q" if quote.is_some() => {
                    let m = quote.unwrap();
                    let pre = quote_prefix(Self::anon_name(m).unwrap_or(&m.from));
                    for (i, l) in m.body.iter().enumerate() {
                        self.sprompt(&format!("|U2{:>3}: |U0", i + 1))?;
                        self.out(&strip_pipes(l))?;
                        self.nl()?;
                    }
                    self.sprompt(&format!("|U1Quote from line (1-{}): |U3", m.body.len()))?;
                    let a = self.input(3)?.parse::<usize>().unwrap_or(0);
                    self.sprompt(&format!("|U1To line ({a}-{}): |U3", m.body.len()))?;
                    let z = self.input(3)?.parse::<usize>().unwrap_or(a).min(m.body.len());
                    if a >= 1 {
                        for l in m.body.iter().take(z).skip(a - 1) {
                            if lines.len() < max {
                                lines.push(format!("{pre}{}", strip_pipes(l)).chars().take(78).collect());
                            }
                        }
                    }
                }
                _ => lines.push(l),
            }
            if lines.len() >= max {
                self.sprint("|U1Maximum lines reached, saving.")?;
                return Ok(Some(lines));
            }
        }
    }

    pub fn can_post(&self, board: &Board) -> bool {
        !self.user.ac.contains('P') && self.acs(&board.post_acs)
    }

    /// Anonymous status for a new post (`None` board = e-mail).
    fn ask_anon(&mut self, board: Option<&Board>) -> io::Result<(u8, String)> {
        let c = self.cfg();
        let an = match board {
            Some(_) if self.user.ac.contains('*') => Anon::No,
            Some(b) if b.anonymous == Anon::No && self.acs(&c.anon_pub_post) => Anon::Yes,
            Some(b) => b.anonymous,
            None if self.acs(&c.anon_priv_post) => Anon::Yes,
            None => Anon::No,
        };
        let mark = if self.is_sysop() { 2 } else { 1 };
        Ok(match an {
            Anon::No => (0, String::new()),
            Anon::Forced => (mark, String::new()),
            Anon::Yes => (if self.pynq(&format!("|NL{}", c.strings.post_anon), false)? { mark } else { 0 }, String::new()),
            Anon::DearAbby => {
                self.sprint(if board.is_some() { "|NL|U1Post as:" } else { "|NL|U1Send as:" })?;
                self.sprint("|U31. |U1Abby")?;
                self.sprint("|U32. |U1Problemed Person")?;
                self.sprompt("|U33. |U1")?;
                let me = self.user.name.clone();
                self.out(&me)?;
                self.nl()?;
                self.sprompt("|NL|U1Which? |U3")?;
                (match self.onek("123N\r")? { '1' => 3, '2' => 4, _ => 0 }, String::new())
            }
            Anon::AnyName => {
                self.sprint("|NL|U1You can post your message under any name you want on this base.")?;
                self.sprompt("|U1Name (Enter for your own): |U3")?;
                let s = self.input(36)?;
                if s.is_empty() { (0, s) } else { (5, s) }
            }
        })
    }

    pub fn post(&mut self, b: usize, reply: Option<&Msg>) -> io::Result<()> {
        let board = self.cfg().boards[b].clone();
        if !self.can_post(&board) {
            return self.sprint("|NL|12You can't post on this base.");
        }
        if !self.board_ok(b)? {
            return Ok(());
        }
        self.set_doing(&format!("Posting in {}", board.name));
        let (to, mut subject) = match reply {
            Some(m) => (Self::anon_name(m).unwrap_or(&m.from).to_string(), re(&m.subject)),
            None => {
                self.sprompt("|NL|U1To (Enter for All): |U3")?;
                let to = self.input(36)?;
                let title = self.cfg().strings.msg_title.clone();
                (if to.is_empty() { "All".into() } else { to }, self.ask(&title, 60)?)
            }
        };
        if subject.is_empty() {
            return self.sprint(&self.cfg().strings.aborted.clone());
        }
        let (anon, alias) = self.ask_anon(Some(&board))?;
        let max = self.cfg().max_message_lines;
        let Some(mut body) = self.editor(max, reply)? else { return self.sprint(&self.cfg().strings.aborted.clone()) };
        if !self.acs(&board.mci_acs) {
            subject = strip_pipes(&subject);
            body = body.iter().map(|l| strip_pipes(l)).collect();
        }
        let from = if board.real_names && !self.user.real_name.is_empty() { self.user.real_name.clone() } else { self.user.name.clone() };
        let m = Msg {
            from,
            from_num: self.unum,
            to,
            subject,
            date: now(),
            body,
            anon,
            alias,
            reply_to: reply.map_or(0, |m| m.id),
            unvalidated: self.user.ac.contains('V'),
            ..Msg::default()
        };
        {
            let mut db = self.sh.db();
            db.sys.zlog().pub_posts += 1;
            db.post(&board.file, m, board.max_msgs)?;
        }
        self.user.posts += 1;
        self.save_user();
        self.log(&format!("Posted on {}", board.name));
        self.sprint(&format!("|U1Message posted on |U3{}|U1.", board.name))
    }

    // ---- e-mail -------------------------------------------------------------

    fn can_mail(&mut self) -> io::Result<bool> {
        if self.user.ac.contains('E') {
            self.sprint("|NL|12You can't send e-mail.")?;
            return Ok(false);
        }
        Ok(true)
    }

    pub fn send_email(&mut self, data: &str) -> io::Result<()> {
        if !self.can_mail()? {
            return Ok(());
        }
        let to = if data.trim().is_empty() {
            let p = self.cfg().strings.msg_send_to.clone();
            self.sprompt(&format!("|NL{p}"))?;
            self.input(36)?
        } else {
            data.trim().to_string()
        };
        let found = self.sh.db().find_user(&to);
        let Some(n) = found else { return self.sprint("|12No such user.") };
        let to_name = self.sh.db().users[n - 1].name.clone();
        self.sprompt("|U1To: |U3")?;
        self.out(&to_name)?;
        self.nl()?;
        let title = self.cfg().strings.msg_title.clone();
        let subject = self.ask(&title, 60)?;
        if subject.is_empty() {
            return self.sprint(&self.cfg().strings.aborted.clone());
        }
        self.email_to(n, to_name, subject, None)
    }

    pub fn email_to(&mut self, n: usize, to: String, subject: String, quote: Option<&Msg>) -> io::Result<()> {
        let (anon, alias) = self.ask_anon(None)?;
        let max = self.cfg().max_message_lines;
        let Some(body) = self.editor(max, quote)? else { return self.sprint(&self.cfg().strings.aborted.clone()) };
        let m = Msg { anon, alias, reply_to: quote.map_or(0, |q| q.id), ..self.letter(subject, body) };
        self.deliver(&[(n, to)], m)?;
        self.sprint("|U1Mail sent.")
    }

    /// A letter from this user, not yet addressed.
    fn letter(&self, subject: String, body: Vec<String>) -> Msg {
        Msg { from: self.user.name.clone(), from_num: self.unum, subject, date: now(), body, ..Msg::default() }
    }

    fn deliver(&mut self, to: &[(usize, String)], m: Msg) -> io::Result<()> {
        {
            let mut db = self.sh.db();
            for (n, name) in to {
                let z = db.sys.zlog();
                z.priv_posts += 1;
                if *n == 1 {
                    z.feedback += 1;
                }
                db.post("email", Msg { to: name.clone(), to_num: *n, ..m.clone() }, 0)?;
            }
        }
        self.user.emails += to.len() as u32;
        self.save_user();
        Ok(())
    }

    /// Private reply to a message (autoreply).
    fn reply_private(&mut self, m: &Msg, private: bool) -> io::Result<()> {
        if !self.can_mail()? {
            return Ok(());
        }
        if (m.anon != 0 && !self.sees_anon(m, private)) || m.from_num == 0 {
            return self.sprint("|NL|U1Can't reply now.");
        }
        if m.from_num == self.unum {
            return self.sprint("|NL|U1Can't send mail to yourself!");
        }
        let name = self.sh.db().users.get(m.from_num - 1).map(|u| u.name.clone());
        let Some(name) = name else { return self.sprint("|12No such user.") };
        self.email_to(m.from_num, name, re(&m.subject), Some(m))
    }

    /// `ML` (smail): one letter to up to 20 users, or by ACS / to everyone
    /// for SysOps.
    pub fn mass_mail(&mut self) -> io::Result<()> {
        if !self.can_mail()? {
            return Ok(());
        }
        let cso = self.is_sysop();
        self.sprint("|NL|U1Mass mail: send mail to more than one user.")?;
        self.sprint("|U1Enter up to 20 user numbers or names, separated by commas.")?;
        if cso {
            self.sprint("|U1SysOps: |U3ALL|U1 sends to every user, |U3ACS|U1 to an ACS group.")?;
        }
        self.sprompt("|U1: |U3")?;
        let s = self.input(78)?;
        let users: Vec<(usize, crate::db::User)> = self.sh.db().users.iter().cloned().enumerate().map(|(i, u)| (i + 1, u)).filter(|(_, u)| !u.deleted).collect();
        let mut to: Vec<(usize, String)> = Vec::new();
        match s.to_uppercase().as_str() {
            "" => return Ok(()),
            "ALL" if cso => to = users.iter().map(|(n, u)| (*n, u.name.clone())).collect(),
            "ACS" if cso => {
                self.sprompt("|U1Enter ACS: |U3")?;
                let acs = self.input(80)?;
                if acs.is_empty() {
                    return Ok(());
                }
                to = users.iter().filter(|(n, u)| self.acs_for(u, *n, &acs)).map(|(n, u)| (*n, u.name.clone())).collect();
            }
            _ => {
                for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let found = self.sh.db().find_user(part);
                    match found {
                        Some(n) if n == self.unum => self.sprint(&format!("|U1[> {part}: Can't send mail to yourself!"))?,
                        Some(n) if to.iter().any(|t| t.0 == n) => self.sprint(&format!("|U1[> {part}: Can't send more than once."))?,
                        Some(n) if to.len() < 20 => to.push((n, self.sh.db().users[n - 1].name.clone())),
                        Some(_) => {}
                        None => self.sprint(&format!("|U1[> {part}: No such user."))?,
                    }
                }
            }
        }
        if to.is_empty() {
            return self.sprint("|U1Nobody to send to.");
        }
        self.sprint("|NL|U1Users marked:")?;
        for (n, name) in to.clone() {
            self.out(&format!("   {name} #{n}"))?;
            self.nl()?;
        }
        if !self.pynq("|NL|U1Is this OK? ", true)? {
            return Ok(());
        }
        let title = self.cfg().strings.msg_title.clone();
        let subject = self.ask(&title, 60)?;
        if subject.is_empty() {
            return self.sprint(&self.cfg().strings.aborted.clone());
        }
        let max = self.cfg().max_message_lines;
        let Some(body) = self.editor(max, None)? else { return self.sprint(&self.cfg().strings.aborted.clone()) };
        let m = self.letter(subject, body);
        self.deliver(&to, m)?;
        self.log(&format!("Mass-mail sent to {} users", to.len()));
        self.sprint(&format!("|U1Mail sent to |U3{}|U1 users.", to.len()))
    }

    pub fn read_mail(&mut self) -> io::Result<()> {
        let load = |s: &Self| -> io::Result<Vec<Msg>> { Ok(s.sh.db().msgs("email")?.into_iter().filter(|m| m.to_num == s.unum).collect()) };
        let mut mine = load(self)?;
        if mine.is_empty() {
            return self.sprint("|NL|U1You have no mail waiting.");
        }
        self.set_doing("Reading e-mail");
        let prompt = self.cfg().strings.mail_prompt.clone();
        let del_q = self.cfg().strings.delete_read_mail.clone();
        let (mut i, mut show) = (0, true);
        while i < mine.len() {
            let m = mine[i].clone();
            if show {
                self.show_msg("E-mail", i + 1, mine.len(), &m, true)?;
            }
            show = false;
            let k = self.sub_prompt("EMAILP", &prompt, "-FJINQATXRSDZ?")?;
            match k {
                'N' | 'I' => {
                    i += 1;
                    show = true;
                }
                'A' => show = true,
                '-' if i == 0 => self.sprint("|NL|U1Already at the first letter.")?,
                '-' => {
                    i -= 1;
                    show = true;
                }
                'J' => {
                    if let Some(n) = self.jump(mine.len())? {
                        i = n;
                        show = true;
                    }
                }
                'T' => self.titles(&mine, i, true)?,
                'Q' => break,
                'X' if self.mso() => self.extract(&m)?,
                'F' => {
                    self.sprompt("|NL|U1Forward letter to which user? |U3")?;
                    let s = self.input(36)?;
                    let found = self.sh.db().find_user(&s);
                    match found {
                        None => self.sprint("|U1Unknown user.")?,
                        Some(n) if n == self.unum => self.sprint("|U1Can't send mail to yourself!")?,
                        Some(n) => {
                            let name = self.sh.db().users[n - 1].name.clone();
                            self.sh.db().update_msg("email", m.id, |x| {
                                x.to_num = n;
                                x.to = name.clone();
                            })?;
                            self.log(&format!("Forwarded letter to {name} #{n}"));
                            mine = load(self)?;
                            show = true;
                        }
                    }
                }
                'R' | 'S' | 'D' | 'Z' => {
                    if matches!(k, 'R' | 'S') {
                        self.reply_private(&m, true)?;
                    }
                    if k != 'S' && self.pynq(&format!("|NL{del_q}"), true)? {
                        self.sh.db().delete_msg("email", m.id)?;
                        mine = load(self)?;
                    }
                    show = true;
                }
                '?' => {
                    for l in MAIL_HELP {
                        self.sprint(l)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `MK` (purgepriv): delete all your letters.
    pub fn purge_mail(&mut self) -> io::Result<()> {
        let n = self.sh.db().msgs("email")?.iter().filter(|m| m.to_num == self.unum).count();
        if n == 0 {
            return self.sprint("|NL|U1You have no mail waiting.");
        }
        if self.pynq(&format!("|NL|U1Delete all |U3{n}|U1 letters? "), false)? {
            let mut db = self.sh.db();
            let ids: Vec<u64> = db.msgs("email")?.iter().filter(|m| m.to_num == self.unum).map(|m| m.id).collect();
            for id in ids {
                db.delete_msg("email", id)?;
            }
            drop(db);
            self.sprint("|U1Deleted.")?;
        }
        Ok(())
    }

    /// `MJ` (dopurgepub): go through your own public posts and delete them.
    pub fn purge_posts(&mut self) -> io::Result<()> {
        for b in self.boards() {
            let board = self.cfg().boards[b].clone();
            let mine: Vec<Msg> = self.sh.db().msgs(&board.file)?.into_iter().filter(|m| m.from_num == self.unum && !m.permanent).collect();
            for m in mine {
                self.sprompt(&format!("|NL|U3{}|U1: ", board.name))?;
                self.out(&m.subject)?;
                self.sprompt(" |U1- Delete |U2(|U3Y|U2/|U3N|U2/|U3Q|U2)|U1? |U3")?;
                match self.onek("YNQ")? {
                    'Y' => self.sh.db().delete_msg(&board.file, m.id)?,
                    'Q' => return Ok(()),
                    _ => {}
                }
            }
        }
        self.sprint("|NL|U1Done.")
    }

    /// `%3` (mailr): read everyone's e-mail.
    pub fn read_all_mail(&mut self) -> io::Result<()> {
        self.log("* Read private mail");
        let all = self.sh.db().msgs("email")?;
        let mut i = 0;
        while i < all.len() {
            let m = all[i].clone();
            self.show_msg("All e-mail", i + 1, all.len(), &m, true)?;
            self.sprompt("|NL|U1[|U3N|U1]ext [|U3D|U1]elete [|U3Q|U1]uit: |U3")?;
            match self.onek("NDQ\r")? {
                'D' => {
                    self.sh.db().delete_msg("email", m.id)?;
                    self.sprint("|U1Deleted.")?;
                    i += 1;
                }
                'Q' => break,
                _ => i += 1,
            }
        }
        if all.is_empty() {
            self.sprint("|NL|U1No e-mail on the system.")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn quote_prefix() {
        assert_eq!(super::quote_prefix("joe bloggs smith"), " JB> ");
        assert_eq!(super::re("Re: x"), "Re: x");
        assert_eq!(super::re("x"), "Re: x");
    }
}

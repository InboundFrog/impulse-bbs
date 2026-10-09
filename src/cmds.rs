//! Menu command implementations (domenucommand in MENUS.PAS and the units it
//! calls: MAIL*.PAS, MISC*.PAS, RUMORS.PAS, CMD.PAS, MULTINOD.PAS ...).

use crate::db::Msg;
use crate::session::{Flow, Session, hangup, now, rand};
use std::io;

impl Session {
    pub fn do_cmd(&mut self, cmd: &str, data: &str) -> io::Result<Flow> {
        let mut k = cmd.chars();
        let (c1, c2) = (k.next().unwrap_or(' '), k.next().unwrap_or(' '));
        match (c1, c2) {
            ('-', '^' | '/' | '\\') => return self.change_menu(c2, data),
            ('-', 'F') => {
                self.printf(data)?;
            }
            ('-', 'L') => self.sprint(&format!("|NL{data}"))?,
            ('-', 'S' | 'C') => self.log(data),
            ('-', '$') => {
                // Password gate: DATA is PASSWORD;PROMPT;FAIL-MESSAGE
                let p: Vec<&str> = data.split(';').collect();
                self.sprompt(p.get(1).copied().filter(|s| !s.is_empty()).unwrap_or(":"))?;
                if self.input_pw(20)? != p[0] {
                    if let Some(m) = p.get(2) {
                        self.sprint(m)?;
                    }
                    return Ok(Flow::Stop);
                }
            }
            ('H', 'C') => {
                if self.pynq(&format!("|NL{data}"), false)? {
                    self.cls()?;
                    self.printf("logoff")?;
                    return Err(hangup());
                }
            }
            ('H', 'I') => return Err(hangup()),
            ('H', 'M') => {
                self.nl()?;
                self.printf(data)?;
                return Err(hangup());
            }
            ('O', 'I') => {
                self.printf("logon")?;
                self.pause()?;
                if !self.printf("system")? {
                    self.sprint("|U1This is |U3|BN|U1, running |U3|VN |U1v|U3|VV|U1 (a Rust port of Impulse 7.1) on |U3|OS|U1.")?;
                }
            }
            ('O', 'Y') => self.your_info()?,
            ('O', 'C') => self.chat(data)?,
            ('O', 'L') => self.oneliners()?,
            ('O', 'K' | 'P') => self.user_config()?,
            ('X', 'C') => self.last_callers()?,
            ('X', 'U') => self.user_list()?,
            ('X', 'S') => {
                self.printf("sysinfo")?;
            }
            ('X', 'H') => self.user.hotkeys = true,
            ('X', 'L') => self.user.hotkeys = false,
            ('X', 'A') => {
                let mut d = data.chars();
                if let (Some(op), Some(f)) = (d.next(), d.next().map(|c| c.to_ascii_uppercase())) {
                    self.user.ar.retain(|c| c != f);
                    if op == '+' {
                        self.user.ar.push(f);
                    }
                }
            }
            ('U', 'R') => self.read_auto()?,
            ('U', 'W') => self.write_auto()?,
            ('R', 'R') => self.random_rumor()?,
            ('R', 'A') => self.add_rumor()?,
            ('R', 'L') => self.list_rumors()?,
            ('R', 'D') => self.delete_rumor()?,
            ('S', 'P') => self.pause()?,
            ('S', 'C') => self.cls()?,
            ('M', 'A') => self.change_board(data)?,
            ('M', 'P') => self.post(self.board, None)?,
            ('M', 'S') => {
                self.read_board(self.board, false)?;
            }
            ('M', 'N') => self.newscan()?,
            ('M', 'E') => self.send_email(data)?,
            ('M', 'M') => self.read_mail()?,
            ('M', '#') => self.sprint("|NL|U1Enter the number of the message base to change to it.")?,
            ('N', 'W') => self.whos_online()?,
            _ => {
                self.sprint("|NL|12Sorry, that command isn't implemented yet.")?;
                self.log(&format!("Unimplemented command {cmd} {data}"));
            }
        }
        Ok(Flow::Next)
    }

    // ---- user stuff -------------------------------------------------------

    fn your_info(&mut self) -> io::Result<()> {
        if self.printf("yourinfo")? {
            return Ok(());
        }
        for l in [
            "|NL|U3Your account on |BN",
            "|U1Handle  : |U3|UN |U1(#|U3|UU|U1)",
            "|U1Real    : |U3|RN",
            "|U1From    : |U3|LO",
            "|U1Level   : |U1SL |U3|UL|U1  DSL |U3|FL",
            "|U1Calls   : |U3|CS|U1  Posts: |U3|PS",
            "|U1Time    : |U3|TL|U1 minutes left today",
            "|U1Last on : |U3|LD",
        ] {
            self.sprint(l)?;
        }
        Ok(())
    }

    fn user_config(&mut self) -> io::Result<()> {
        loop {
            let yn = |b: bool| if b { "Yes" } else { "No" };
            let u = &self.user;
            let lines = [
                "|NL|U3User configuration".to_string(),
                format!("|U1[|U3A|U1] ANSI color       : |U3{}", yn(u.ansi)),
                format!("|U1[|U3P|U1] Screen pausing   : |U3{}", yn(u.pause)),
                format!("|U1[|U3H|U1] Hot keys         : |U3{}", yn(u.hotkeys)),
                format!("|U1[|U3E|U1] Expert (no menus): |U3{}", yn(!u.novice)),
                format!("|U1[|U3L|U1] Screen length    : |U3{}", u.page_len),
                "|U1[|U3W|U1] Change password".to_string(),
                "|U1[|U3Q|U1] Quit".to_string(),
            ];
            for l in lines {
                self.sprint(&l)?;
            }
            self.sprompt("|U1Choice: |U3")?;
            match self.onek("APHELWQ\r")? {
                'A' => self.user.ansi = !self.user.ansi,
                'P' => self.user.pause = !self.user.pause,
                'H' => self.user.hotkeys = !self.user.hotkeys,
                'E' => self.user.novice = !self.user.novice,
                'L' => {
                    self.sprompt("|U1Lines on your screen (5-99): |U3")?;
                    if let Ok(n @ 5..=99) = self.input(2)?.parse() {
                        self.user.page_len = n;
                    }
                }
                'W' => {
                    self.sprompt("|U1Current password: |U3")?;
                    let old = self.input_pw(20)?;
                    if !crate::db::check_pw(&self.user.pw_hash, &old) {
                        self.sprint("|12Wrong password.")?;
                        continue;
                    }
                    self.sprompt("|U1New password: |U3")?;
                    let p1 = self.input_pw(20)?;
                    self.sprompt("|U1Again: |U3")?;
                    if p1.chars().count() >= 4 && p1 == self.input_pw(20)? {
                        self.user.pw_hash = crate::db::hash_pw(&p1);
                        self.sprint("|U1Password changed.")?;
                    } else {
                        self.sprint("|12Passwords didn't match or were too short.")?;
                    }
                }
                _ => break,
            }
        }
        self.save_user();
        Ok(())
    }

    fn chat(&mut self, _data: &str) -> io::Result<()> {
        self.sprint("|NL|U1The SysOp isn't available for chat right now.")?;
        if self.pynq("|U1Leave a message to the SysOp instead? ", true)? {
            self.send_email("1")?;
        }
        Ok(())
    }

    fn last_callers(&mut self) -> io::Result<()> {
        let list = self.sh.db().sys.last_callers.clone();
        self.sprint("|NL|U3Last callers to |BN|NL|U2#      Handle                    From                      When")?;
        for c in list {
            self.sprint(&format!("|U1{:<6} |U3{:<25} |U1{:<25} |U2{}", c.caller_num, c.name, c.city_state, c.when))?;
        }
        Ok(())
    }

    fn user_list(&mut self) -> io::Result<()> {
        let users: Vec<(usize, String, String, u8)> = self
            .sh
            .db()
            .users
            .iter()
            .enumerate()
            .filter(|(_, u)| !u.deleted)
            .map(|(i, u)| (i + 1, u.name.clone(), u.city_state.clone(), u.sl))
            .collect();
        self.sprint("|NL|U2#     Handle                         From                       SL")?;
        for (n, name, from, sl) in users {
            self.sprint(&format!("|U1{n:<5} |U3{name:<30} |U1{from:<26} |U2{sl}"))?;
        }
        Ok(())
    }

    fn whos_online(&mut self) -> io::Result<()> {
        let list: Vec<(usize, String, String)> =
            self.sh.db().online.iter().map(|(n, o)| (*n, o.name.clone(), o.doing.clone())).collect();
        self.sprint("|NL|U2Node  Handle                         Activity")?;
        for (n, name, doing) in list {
            self.sprint(&format!("|U3{n:<5} |U1{name:<30} |U3{doing}"))?;
        }
        Ok(())
    }

    // ---- automessage, oneliners, rumors -----------------------------------

    fn read_auto(&mut self) -> io::Result<()> {
        let lines = self.sh.db().lines("auto.msg");
        let Some((author, body)) = lines.split_first() else {
            return self.sprint("|NL|U1No AutoMessage available.");
        };
        let author = match author.strip_prefix('@') {
            Some(a) if self.is_sysop() => format!("{a} (Posted Anonymously)"),
            Some(_) => "Anonymous".into(),
            None => author.clone(),
        };
        let body = body.to_vec();
        self.vars.insert("AN".into(), author);
        if !self.printf("AUTOREAD.TOP")? {
            self.sprint("|NL|U1AutoMessage by |U3%AN|U1:")?;
        }
        for l in &body {
            self.sprint(l)?;
        }
        self.printf("AUTOREAD.BOT")?;
        self.vars.clear();
        Ok(())
    }

    fn write_auto(&mut self) -> io::Result<()> {
        if self.user.ac.contains('A') {
            return self.sprint("|NL|12You are restricted from writing automessages.");
        }
        self.printf("AUTOWRT")?;
        let Some(body) = self.editor(30)? else { return self.sprint(&self.sh.cfg.strings.aborted.clone()) };
        let anon = self.pynq("|U1Post anonymously? ", false)?;
        let mut lines = vec![if anon { format!("@{}", self.user.name) } else { self.user.name.clone() }];
        lines.extend(body);
        self.sh.db().put_lines("auto.msg", &lines)?;
        self.log("Changed the AutoMessage");
        self.sprint("|U1AutoMessage saved.")
    }

    fn oneliners(&mut self) -> io::Result<()> {
        let mut list = self.sh.db().lines("oneliners.txt");
        self.cls()?;
        if !self.printf("oneline.top")? {
            self.sprint("|U3Oneliners|NL")?;
        }
        if list.is_empty() {
            self.sprint("|U1No oneliners exist!")?;
        }
        for l in &list {
            self.sprint(l)?;
        }
        self.printf("oneline.bot")?;
        if !self.pynq("|NL|U1Add a oneliner? ", false)? {
            return Ok(());
        }
        self.sprompt("|U1: |U3")?;
        let r = self.input(70)?;
        if r.is_empty() {
            return Ok(());
        }
        self.log(&format!("Oneliner: {r}"));
        list.push(r);
        let skip = list.len().saturating_sub(15);
        self.sh.db().put_lines("oneliners.txt", &list[skip..])?;
        self.sprint("|U1Your oneliner was added.")
    }

    fn random_rumor(&mut self) -> io::Result<()> {
        let list = self.sh.db().lines("rumors.txt");
        if list.is_empty() {
            return self.sprint("|NL|U1No rumors exist!");
        }
        let n = rand(list.len());
        self.tmpl("rumors.ran", &[("RR", list[n].clone()), ("RI", (n + 1).to_string())], "|NL|U1Rumor: |U3%RR")
    }

    fn add_rumor(&mut self) -> io::Result<()> {
        self.sprompt("|NL|U1Enter your rumor:|NL|U3: ")?;
        let r = self.input(79)?;
        if r.is_empty() {
            return Ok(());
        }
        let mut list = self.sh.db().lines("rumors.txt");
        list.push(r.clone());
        self.sh.db().put_lines("rumors.txt", &list)?;
        self.log(&format!("Added rumor: {r}"));
        self.sprint("|U1Rumor added.")
    }

    fn list_rumors(&mut self) -> io::Result<()> {
        let list = self.sh.db().lines("rumors.txt");
        self.tmpl("rumors.top", &[], "|NL|U3Rumors")?;
        for (i, r) in list.iter().enumerate() {
            self.tmpl("rumors.mid", &[("RR", r.clone()), ("RI", (i + 1).to_string())], "|U2%RI. |U1%RR")?;
        }
        self.tmpl("rumors.bot", &[], "")
    }

    fn delete_rumor(&mut self) -> io::Result<()> {
        if !self.is_sysop() {
            return self.sprint("|NL|12Only the SysOp can delete rumors.");
        }
        self.list_rumors()?;
        self.sprompt("|NL|U1Delete which rumor? |U3")?;
        let mut list = self.sh.db().lines("rumors.txt");
        if let Ok(n @ 1..) = self.input(4)?.parse::<usize>()
            && n <= list.len() {
                list.remove(n - 1);
                self.sh.db().put_lines("rumors.txt", &list)?;
                self.sprint("|U1Rumor deleted.")?;
            }
        Ok(())
    }

    // ---- messages ---------------------------------------------------------

    /// Board indexes this user may read.
    fn boards(&self) -> Vec<usize> {
        let b = &self.sh.cfg.boards;
        (0..b.len()).filter(|&i| self.acs(&b[i].acs)).collect()
    }

    pub fn board_by_number(&mut self, n: &str) -> io::Result<()> {
        let list = self.boards();
        match n.parse::<usize>() {
            Ok(n @ 1..) if n <= list.len() => {
                self.board = list[n - 1];
                self.user.last_board = self.board;
                self.sprint("|U1Joined |U3|CM")
            }
            _ => self.sprint("|12No such message base."),
        }
    }

    fn change_board(&mut self, data: &str) -> io::Result<()> {
        let list = self.boards();
        if list.is_empty() {
            return self.sprint("|NL|12No message bases available.");
        }
        let pos = list.iter().position(|&b| b == self.board).unwrap_or(0);
        let n = match data.trim() {
            "+" => (pos + 1) % list.len() + 1,
            "-" => (pos + list.len() - 1) % list.len() + 1,
            _ => {
                self.tmpl("msgbase.top", &[], "|NL|U3Message bases")?;
                for (i, &b) in list.iter().enumerate() {
                    let vars = [("ME", self.sh.cfg.boards[b].name.clone()), ("MI", (i + 1).to_string()), ("MT", "Local".into())];
                    self.tmpl("msgbase.mid", &vars, "|U2%MI. |U1%ME")?;
                }
                self.tmpl("msgbase.bot", &[], "")?;
                self.sprompt("|NL|U1Change to which base? |U3")?;
                match self.input(3)?.parse() {
                    Ok(n) => n,
                    Err(_) => return Ok(()),
                }
            }
        };
        self.board_by_number(&n.to_string())
    }

    fn show_msg(&mut self, area: &str, n: usize, total: usize, m: &Msg) -> io::Result<()> {
        if self.user.ansi {
            self.cls()?;
        } else {
            self.nl()?;
        }
        self.sprint(&format!("|U1Msg |U3{n}|U1 of |U3{total} |U1in |U3{area}"))?;
        self.out_field("From", &m.from)?;
        self.out_field("To", &m.to)?;
        self.out_field("Subj", &m.subject)?;
        self.out_field("Date", &m.date)?;
        self.nl()?;
        for l in &m.body {
            self.sprint(&format!("|U0{l}"))?;
        }
        Ok(())
    }

    /// Header field, printed raw so names can't inject pipe codes.
    fn out_field(&mut self, label: &str, v: &str) -> io::Result<()> {
        self.sprompt(&format!("|U1{label:<5}: |U3"))?;
        self.out(v)?;
        self.nl()
    }

    /// Read a board from the first unread message. Returns true if the user quit.
    fn read_board(&mut self, b: usize, new_only: bool) -> io::Result<bool> {
        let board = self.sh.cfg.boards[b].clone();
        let msgs = self.sh.db().msgs(&board.file)?;
        let last = self.user.last_read.get(&board.file).copied().unwrap_or(0);
        let mut i = match msgs.iter().position(|m| m.id > last) {
            Some(i) => i,
            None if new_only => return Ok(false),
            None if msgs.is_empty() => {
                self.sprint(&format!("|NL|U1No messages in |U3{}|U1.", board.name))?;
                return Ok(false);
            }
            None => {
                self.sprompt(&format!("|NL|U1No new messages. Start at (1-{}): |U3", msgs.len()))?;
                match self.input(5)?.parse::<usize>() {
                    Ok(n @ 1..) if n <= msgs.len() => n - 1,
                    _ => return Ok(false),
                }
            }
        };
        self.set_doing(&format!("Reading {}", board.name));
        loop {
            self.show_msg(&board.name, i + 1, msgs.len(), &msgs[i])?;
            let read = self.user.last_read.entry(board.file.clone()).or_default();
            *read = (*read).max(msgs[i].id);
            self.sprompt("|NL|U1[|U3N|U1]ext [|U3A|U1]gain [|U3R|U1]eply [|U3P|U1]ost [|U3Q|U1]uit: |U3")?;
            match self.onek("NARPQ\r")? {
                'A' => {}
                'R' => self.post(b, Some(&msgs[i]))?,
                'P' => self.post(b, None)?,
                'Q' => return Ok(true),
                _ => {
                    i += 1;
                    if i >= msgs.len() {
                        self.sprint("|U1End of messages.")?;
                        return Ok(false);
                    }
                }
            }
        }
    }

    fn newscan(&mut self) -> io::Result<()> {
        for b in self.boards() {
            let name = self.sh.cfg.boards[b].name.clone();
            self.sprint(&format!("|NL|U1Scanning |U3{name}|U1..."))?;
            if self.read_board(b, true)? {
                break;
            }
        }
        self.sprint("|U1Newscan complete.")
    }

    /// Line editor: `/S` saves, `/A` aborts, `/L` lists.
    // ponytail: no word wrap or full-screen editor yet
    fn editor(&mut self, max: usize) -> io::Result<Option<Vec<String>>> {
        self.sprint(&format!("|NL|U1Enter up to |U3{max}|U1 lines. |U3/S|U1 saves, |U3/A|U1 aborts, |U3/L|U1 lists."))?;
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
                _ => lines.push(l),
            }
            if lines.len() >= max {
                self.sprint("|U1Maximum lines reached, saving.")?;
                return Ok(Some(lines));
            }
        }
    }

    fn post(&mut self, b: usize, reply: Option<&Msg>) -> io::Result<()> {
        let board = self.sh.cfg.boards[b].clone();
        if self.user.ac.contains('P') || !self.acs(&board.post_acs) {
            return self.sprint("|NL|12You can't post on this base.");
        }
        self.set_doing(&format!("Posting in {}", board.name));
        let (to, subject) = match reply {
            Some(m) => (m.from.clone(), if m.subject.starts_with("Re:") { m.subject.clone() } else { format!("Re: {}", m.subject) }),
            None => {
                self.sprompt("|NL|U1To (Enter for All): |U3")?;
                let to = self.input(36)?;
                let title = self.sh.cfg.strings.msg_title.clone();
                (if to.is_empty() { "All".into() } else { to }, self.ask(&title, 60)?)
            }
        };
        let max = self.sh.cfg.max_message_lines;
        let Some(body) = self.editor(max)? else { return self.sprint(&self.sh.cfg.strings.aborted.clone()) };
        let m = Msg { id: 0, from: self.user.name.clone(), from_num: self.unum, to, to_num: 0, subject, date: now(), body };
        self.sh.db().post(&board.file, m)?;
        self.user.posts += 1;
        self.save_user();
        self.log(&format!("Posted on {}", board.name));
        self.sprint(&format!("|U1Message posted on |U3{}|U1.", board.name))
    }

    fn send_email(&mut self, data: &str) -> io::Result<()> {
        if self.user.ac.contains('E') {
            return self.sprint("|NL|12You can't send e-mail.");
        }
        let to = if data.trim().is_empty() {
            let p = self.sh.cfg.strings.msg_send_to.clone();
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
        let title = self.sh.cfg.strings.msg_title.clone();
        let subject = self.ask(&title, 60)?;
        self.email_to(n, to_name, subject)
    }

    fn email_to(&mut self, n: usize, to: String, subject: String) -> io::Result<()> {
        let max = self.sh.cfg.max_message_lines;
        let Some(body) = self.editor(max)? else { return self.sprint(&self.sh.cfg.strings.aborted.clone()) };
        let m = Msg { id: 0, from: self.user.name.clone(), from_num: self.unum, to, to_num: n, subject, date: now(), body };
        self.sh.db().post("email", m)?;
        self.user.emails += 1;
        self.save_user();
        self.sprint("|U1Mail sent.")
    }

    fn read_mail(&mut self) -> io::Result<()> {
        let mine: Vec<Msg> = self.sh.db().msgs("email")?.into_iter().filter(|m| m.to_num == self.unum).collect();
        if mine.is_empty() {
            return self.sprint("|NL|U1You have no mail waiting.");
        }
        self.set_doing("Reading e-mail");
        let mut i = 0;
        while i < mine.len() {
            let m = mine[i].clone();
            self.show_msg("E-mail", i + 1, mine.len(), &m)?;
            self.sprompt("|NL|U1[|U3N|U1]ext [|U3R|U1]eply [|U3D|U1]elete [|U3Q|U1]uit: |U3")?;
            match self.onek("NRDQ\r")? {
                'R' => {
                    let s = if m.subject.starts_with("Re:") { m.subject.clone() } else { format!("Re: {}", m.subject) };
                    self.email_to(m.from_num, m.from.clone(), s)?;
                }
                'D' => {
                    self.sh.db().delete_msg("email", m.id)?;
                    self.sprint("|U1Deleted.")?;
                    i += 1;
                }
                'Q' => break,
                _ => i += 1,
            }
        }
        Ok(())
    }
}

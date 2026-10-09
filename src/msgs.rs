//! Message bases and e-mail (MAIL*.PAS).

use crate::db::Msg;
use crate::session::{Session, now};
use std::io;

impl Session {
    // ---- messages ---------------------------------------------------------

    /// Board indexes this user may read.
    pub fn boards(&self) -> Vec<usize> {
        let b = &self.cfg().boards;
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
                self.tmpl("msgbase.top", &[], "|NL|U3Message bases")?;
                for (i, &b) in list.iter().enumerate() {
                    let vars = [("ME", self.cfg().boards[b].name.clone()), ("MI", (i + 1).to_string()), ("MT", "Local".into())];
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

    pub fn show_msg(&mut self, area: &str, n: usize, total: usize, m: &Msg) -> io::Result<()> {
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
    pub fn out_field(&mut self, label: &str, v: &str) -> io::Result<()> {
        self.sprompt(&format!("|U1{label:<5}: |U3"))?;
        self.out(v)?;
        self.nl()
    }

    /// Read a board from the first unread message. Returns true if the user quit.
    pub fn read_board(&mut self, b: usize, new_only: bool) -> io::Result<bool> {
        let board = self.cfg().boards[b].clone();
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

    pub fn newscan(&mut self) -> io::Result<()> {
        for b in self.boards() {
            let name = self.cfg().boards[b].name.clone();
            self.sprint(&format!("|NL|U1Scanning |U3{name}|U1..."))?;
            if self.read_board(b, true)? {
                break;
            }
        }
        self.sprint("|U1Newscan complete.")
    }

    /// Line editor: `/S` saves, `/A` aborts, `/L` lists.
    // ponytail: no word wrap or full-screen editor yet
    pub fn editor(&mut self, max: usize) -> io::Result<Option<Vec<String>>> {
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

    pub fn post(&mut self, b: usize, reply: Option<&Msg>) -> io::Result<()> {
        let board = self.cfg().boards[b].clone();
        if self.user.ac.contains('P') || !self.acs(&board.post_acs) {
            return self.sprint("|NL|12You can't post on this base.");
        }
        self.set_doing(&format!("Posting in {}", board.name));
        let (to, subject) = match reply {
            Some(m) => (m.from.clone(), if m.subject.starts_with("Re:") { m.subject.clone() } else { format!("Re: {}", m.subject) }),
            None => {
                self.sprompt("|NL|U1To (Enter for All): |U3")?;
                let to = self.input(36)?;
                let title = self.cfg().strings.msg_title.clone();
                (if to.is_empty() { "All".into() } else { to }, self.ask(&title, 60)?)
            }
        };
        let max = self.cfg().max_message_lines;
        let Some(body) = self.editor(max)? else { return self.sprint(&self.cfg().strings.aborted.clone()) };
        let m = Msg { id: 0, from: self.user.name.clone(), from_num: self.unum, to, to_num: 0, subject, date: now(), body };
        {
            let mut db = self.sh.db();
            db.sys.zlog().pub_posts += 1;
            db.post(&board.file, m)?;
        }
        self.user.posts += 1;
        self.save_user();
        self.log(&format!("Posted on {}", board.name));
        self.sprint(&format!("|U1Message posted on |U3{}|U1.", board.name))
    }

    pub fn send_email(&mut self, data: &str) -> io::Result<()> {
        if self.user.ac.contains('E') {
            return self.sprint("|NL|12You can't send e-mail.");
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
        self.email_to(n, to_name, subject)
    }

    pub fn email_to(&mut self, n: usize, to: String, subject: String) -> io::Result<()> {
        let max = self.cfg().max_message_lines;
        let Some(body) = self.editor(max)? else { return self.sprint(&self.cfg().strings.aborted.clone()) };
        let m = Msg { id: 0, from: self.user.name.clone(), from_num: self.unum, to, to_num: n, subject, date: now(), body };
        {
            let mut db = self.sh.db();
            let z = db.sys.zlog();
            z.priv_posts += 1;
            if n == 1 {
                z.feedback += 1;
            }
            db.post("email", m)?;
        }
        self.user.emails += 1;
        self.save_user();
        self.sprint("|U1Mail sent.")
    }

    pub fn read_mail(&mut self) -> io::Result<()> {
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

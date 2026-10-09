//! Smaller user commands: bulletins and the BBS list (MISC2.PAS), the time
//! bank (TIMEBANK.PAS), InfoForm questionnaires (MISC4.PAS), and the `-;`,
//! `-W`, `-P`, `XY`, `OA` and `UA` menu commands.

use crate::session::{Flow, Session, now, strip_pipes};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct BbsEntry {
    pub name: String,
    pub speed: String,
    pub phone: String,
    pub soft: String,
    pub sysop: String,
    pub comment: String,
}

impl Session {
    /// `OB`/`OS`: data is `MAIN;PREFIX`; MAIN lists them, PREFIXn is bulletin n.
    pub fn bulletins(&mut self, par: &str) -> io::Result<()> {
        let par = if par.is_empty() { format!("bulletin;{}", self.cfg().bulletin_prefix) } else { par.to_string() };
        let (main, subs) = par.split_once(';').unwrap_or((&par, &par));
        let (main, subs) = (main.to_string(), subs.to_string());
        self.nl()?;
        if !self.printf(&main)? {
            return self.sprint("|U1There are no bulletins.");
        }
        loop {
            let p = self.cfg().strings.bulletin_line.clone();
            self.sprompt(&p)?;
            let s = self.input_caps(8usize.saturating_sub(subs.len()).max(1))?.replace(' ', "");
            match s.as_str() {
                "" | "Q" => return Ok(()),
                "?" => {
                    self.printf(&main)?;
                }
                n => {
                    if !self.printf(&format!("{subs}{n}"))? {
                        self.sprint("|12No such bulletin.")?;
                    }
                }
            }
        }
    }

    /// The bank never holds more than `max_time_bank` or less than nothing.
    fn check_time_bank(&mut self) {
        self.user.time_bank = self.user.time_bank.clamp(0, self.cfg().max_time_bank.max(0));
    }

    fn ask_minutes(&mut self, given: &str, prompt: String) -> io::Result<Option<i64>> {
        if let Ok(n) = given.trim().parse::<i64>()
            && n != 0
        {
            return Ok(Some(n));
        }
        self.sprompt(&prompt)?;
        match self.input(5)?.parse::<i64>() {
            Ok(n) if n > 0 => Ok(Some(n)),
            _ => {
                let a = self.cfg().strings.aborted.clone();
                self.sprint(&a)?;
                Ok(None)
            }
        }
    }

    /// `UD`: move minutes from today's time into the bank.
    pub fn time_bank_deposit(&mut self, data: &str) -> io::Result<()> {
        self.check_time_bank();
        let p = self.cfg().strings.time_bank_deposit.clone();
        let Some(n) = self.ask_minutes(data, p)? else { return Ok(()) };
        let room = self.cfg().max_time_bank - self.user.time_bank;
        if n > self.time_left_min() || n > room {
            return self.sprint("|12You can't deposit that much.");
        }
        self.user.time_bank += n;
        self.user.minutes_today -= n;
        self.save_user();
        self.sprint(&format!("|U1Deposited |U3{n}|U1 minutes. Bank balance: |U3|BT"))
    }

    /// `UL`: take minutes back out of the bank.
    pub fn time_bank_withdraw(&mut self, data: &str) -> io::Result<()> {
        self.check_time_bank();
        let p = self.cfg().strings.time_bank_withdraw.clone();
        let Some(n) = self.ask_minutes(data, p)? else { return Ok(()) };
        if n > self.user.time_bank {
            return self.sprint("|12You don't have that much time in the bank.");
        }
        self.user.time_bank -= n;
        self.user.minutes_today += n;
        self.save_user();
        self.sprint(&format!("|U1Withdrew |U3{n}|U1 minutes. Bank balance: |U3|BT"))
    }

    /// `-V`
    pub fn bbs_list_view(&mut self) -> io::Result<()> {
        let list: Vec<BbsEntry> = self.sh.db().get("bbslist.json")?;
        if list.is_empty() {
            self.sprint("|NL|U1The BBS list is empty!")?;
            return self.pause();
        }
        self.tmpl("BBSLIST.TOP", &[], "|NL|U2BBS Name                       Phone        Speed Software SysOp")?;
        for b in list {
            let vars = [("BN", b.name), ("SP", b.speed), ("CM", b.comment), ("PH", b.phone), ("SY", b.sysop), ("TY", b.soft)];
            self.tmpl("BBSLIST.MID", &vars, "|U3%BN |U1%PH |U2%SP |U1%TY |U3%SY|NL       |U1%CM")?;
        }
        self.tmpl("BBSLIST.BOT", &[], "")?;
        self.pause()
    }

    /// `-A`
    pub fn bbs_list_add(&mut self) -> io::Result<()> {
        let ask = |s: &mut Session, q: &str, n: usize| -> io::Result<String> {
            s.sprompt(q)?;
            s.input(n)
        };
        let b = BbsEntry {
            name: ask(self, "|NL|U1BBS name: |U3", 30)?,
            speed: ask(self, "|U1Maximum speed: |U3", 5)?,
            phone: ask(self, "|U1Phone number / address: |U3", 12)?,
            soft: ask(self, "|U1BBS software: |U3", 8)?,
            sysop: ask(self, "|U1SysOp: |U3", 10)?,
            comment: ask(self, "|U1Comment: |U3", 30)?,
        };
        if b.name.is_empty() || !self.pynq("|U1Save this? ", true)? {
            return Ok(());
        }
        let db = self.sh.db();
        let mut list: Vec<BbsEntry> = db.get("bbslist.json")?;
        list.push(b);
        db.put("bbslist.json", &list)
    }

    /// `-W`: show the HDR display file with `%HS` set to `s`.
    pub fn write_hdr(&mut self, s: &str) -> io::Result<()> {
        self.hdr = s.to_string();
        if self.printf("hdr")? {
            return Ok(());
        }
        let w = strip_pipes(s).chars().count();
        let pad = " ".repeat(38usize.saturating_sub(w / 2));
        self.sprint(&format!("|U4{pad}\u{da}{}\u{bf}", "\u{c4}".repeat(w + 2)))?;
        self.sprint(&format!("|U4{pad}\u{b3} |U3{s}|U4 \u{b3}"))?;
        self.sprint(&format!("|U4{pad}\u{c0}{}\u{d9}", "\u{c4}".repeat(w + 2)))
    }

    /// `-;`: type keys for the caller; `;` is Enter.
    pub fn macro_keys(&mut self, s: &str) {
        self.t.stuffed.extend(s.chars().map(|c| if c == ';' { '\r' } else { c }));
    }

    /// `-P`: print a string from the `[strings]` table by name.
    pub fn print_string(&mut self, name: &str) -> io::Result<()> {
        let v = serde_json::to_value(&self.cfg().strings).unwrap_or_default();
        match v.get(name.trim().to_lowercase()).and_then(|v| v.as_str()) {
            Some(s) => self.sprint(s),
            None => Ok(()),
        }
    }

    /// `XY`: data is `PROMPT;CMDKEYS DATA`; run the command if they say yes.
    pub fn yes_no_cmd(&mut self, data: &str) -> io::Result<Flow> {
        let (prompt, cmd) = data.split_once(';').unwrap_or((data, ""));
        if cmd.len() < 2 || !self.pynq(prompt, false)? {
            return Ok(Flow::Next);
        }
        let (k, d) = cmd.split_at(cmd.char_indices().nth(2).map_or(cmd.len(), |(i, _)| i));
        self.do_cmd(&k.to_uppercase(), d)
    }

    /// Give a user the auto-validation levels (autovalidate in COMMON.PAS).
    pub fn auto_validate(&self, u: &mut crate::db::User) {
        let cfg = self.cfg();
        u.sl = cfg.auto_sl;
        u.dsl = cfg.auto_dsl;
        u.ar = cfg.auto_ar.to_uppercase();
        u.ac = cfg.auto_ac.to_uppercase();
    }

    /// `UA`: e-mail whoever wrote the AutoMessage.
    pub fn reply_auto(&mut self) -> io::Result<()> {
        let lines = self.sh.db().lines("auto.msg");
        match lines.first() {
            Some(a) if !a.starts_with('@') || self.is_sysop() => {
                let to = a.trim_start_matches('@').to_string();
                self.send_email(&to)
            }
            Some(_) => self.sprint("|NL|U1Can't reply now."),
            None => self.sprint("|NL|U1Nothing to reply to."),
        }
    }

    /// `-Q`: fill in an InfoForm. In the .INF file `*` asks for a line and
    /// `%NN` for NN characters; text and answers go to `<data>/NAME.ASW`.
    pub fn questionnaire(&mut self, name: &str) -> io::Result<()> {
        let stem = name.split('.').next().unwrap_or(name).to_uppercase();
        let Some(path) = crate::session::find_ci(&self.cfg().text_path, &format!("{stem}.INF")) else {
            self.log(&format!("InfoForm not found: {name}"));
            return self.sprint(&format!("|12InfoForm not found: {name}"));
        };
        let form: Vec<char> = std::fs::read(path)?.iter().map(|&b| b as char).collect();
        self.log(&format!("Answered InfoForm {stem}"));
        let mut answer = format!("User: {} #{}\nDate: {}\n\n", self.user.name, self.unum, now());
        let mut i = 0;
        let r: io::Result<()> = (|| {
            while i < form.len() {
                let ch = form[i];
                i += 1;
                let len = match ch {
                    '*' => 79,
                    '%' if i + 1 < form.len() => {
                        let n = form[i..i + 2].iter().collect::<String>().parse().unwrap_or(79);
                        i += 2;
                        n
                    }
                    '\x1a' => break,
                    _ => {
                        self.out(&ch.to_string())?;
                        answer.push(ch);
                        continue;
                    }
                };
                // inputl echoes no newline: the form text carries on after it
                let a = self.input_raw(len, false, None)?;
                answer.push_str(&a);
            }
            Ok(())
        })();
        if r.is_err() {
            answer.push_str("\n* HUNG UP *\n");
        }
        let p = self.sh.db().path(&format!("{stem}.ASW"));
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
        f.write_all(&answer.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>())?;
        writeln!(f)?;
        r
    }

    /// `-R`: show the answers one user gave to an InfoForm (SysOp).
    pub fn read_answers(&mut self, name: &str) -> io::Result<()> {
        self.sprint("|NL|U1Read InfoForm answers -")?;
        let name = if name.is_empty() { self.ask("|U1Enter filename: |U3", 8)? } else { name.to_string() };
        let stem = name.split('.').next().unwrap_or("").to_uppercase();
        let Ok(text) = std::fs::read(self.sh.db().path(&format!("{stem}.ASW"))) else {
            return self.sprint(&format!("|12InfoForm answer file not found: {stem}.ASW"));
        };
        let text: String = text.iter().map(|&b| b as char).collect();
        self.sprompt("|U1Enter user number or name (Enter for all): |U3")?;
        let who = self.input(36)?;
        let n = self.sh.db().find_user(&who);
        let Some(n) = n else {
            if self.pynq("|U1List entire answer file? ", false)? {
                self.out(&text)?;
            }
            return Ok(());
        };
        if !self.show_answers(&stem, n)? {
            self.sprint("|12Questionnaire answers not found.")?;
        }
        Ok(())
    }

    /// Print user `n`'s answers from `<data>/STEM.ASW`; false if there are none.
    pub fn show_answers(&mut self, stem: &str, n: usize) -> io::Result<bool> {
        let Ok(text) = std::fs::read(self.sh.db().path(&format!("{stem}.ASW"))) else { return Ok(false) };
        let text: String = text.iter().map(|&b| b as char).collect();
        let Some(name) = self.sh.db().users.get(n - 1).map(|u| u.name.clone()) else { return Ok(false) };
        let head = format!("User: {name} #{n}");
        let (mut found, mut show) = (false, false);
        for l in text.lines() {
            if l.starts_with("User: ") {
                show = l.eq_ignore_ascii_case(&head);
                found |= show;
            }
            if show {
                self.out(l)?;
                self.nl()?;
            }
        }
        Ok(found)
    }
}

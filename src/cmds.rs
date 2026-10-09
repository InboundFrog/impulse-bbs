//! Menu command implementations (domenucommand in MENUS.PAS and the units it
//! calls: MAIL*.PAS, MISC*.PAS, RUMORS.PAS, CMD.PAS, MULTINOD.PAS ...).

use crate::session::{Flow, Session, hangup, rand};
use std::io;

impl Session {
    pub fn do_cmd(&mut self, cmd: &str, data: &str) -> io::Result<Flow> {
        let mut k = cmd.chars();
        let (c1, c2) = (k.next().unwrap_or(' '), k.next().unwrap_or(' '));
        match (c1, c2) {
            ('-', '^' | '/' | '\\') => return self.change_menu(c2, data),
            // prompt menus (readp/emailp/filep): hand the key back to the reader
            ('@' | '$' | '!', _) | ('V', 'Y' | 'N' | 'V' | 'D') => self.pkey = Some(c2),
            ('-', 'N') => self.new_user_vote()?,
            ('-', 'F') => {
                self.printf(data)?;
            }
            ('-', 'L') => self.sprint(&format!("|NL{data}"))?,
            // -C writes on the local status line; there is no local screen
            ('-', 'S' | 'C') => self.log(data),
            ('-', ';') => self.macro_keys(data),
            ('-', 'W') => self.write_hdr(data)?,
            ('-', 'P') => self.print_string(data)?,
            ('-', 'V') => self.bbs_list_view()?,
            ('-', 'A') => self.bbs_list_add()?,
            ('-', 'Q') => self.questionnaire(data)?,
            ('-', 'R') => self.read_answers(data)?,
            // -T forced the file/message scan prompt in old menus; prompts here are always live
            ('-', 'T') => {}
            ('X', 'Y') => return self.yes_no_cmd(data),
            ('O', 'A') => {
                let mut u = self.user.clone();
                self.auto_validate(&mut u);
                self.user = u;
                self.save_user();
            }
            ('O', 'B') => self.bulletins("")?,
            ('O', 'S') => self.bulletins(data)?,
            ('U', 'A') => self.reply_auto()?,
            ('U', 'D') => self.time_bank_deposit(data)?,
            ('U', 'L') => self.time_bank_withdraw(data)?,
            ('S', 'S' | 'H') => self.sprint("|NL|U1Memory is managed by Rust; there is no stack or heap limit to show.")?,
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
            ('C', 'M') => self.change_conf(false, data)?,
            ('C', 'F') => self.change_conf(true, data)?,
            ('C', 'E') => self.conf_editor()?,
            ('C', 'N') => self.newscan()?,
            ('M', 'A') => self.change_board(data)?,
            ('M', 'P') => self.post(self.board, None)?,
            ('M', 'S') => {
                self.read_board(self.board, false)?;
            }
            ('M', 'N') => self.newscan()?,
            ('M', 'E') => self.send_email(data)?,
            ('M', 'M') => self.read_mail()?,
            ('M', 'L') => self.mass_mail()?,
            ('M', 'J') => self.purge_posts()?,
            ('M', 'K') => self.purge_mail()?,
            ('M', 'Z') => self.newscan_boards()?,
            ('M', '$') => self.board_stats()?,
            ('M', '#') => self.sprint("|NL|U1Enter the number of the message base to change to it.")?,
            ('N', 'W') => self.whos_online()?,
            ('%', _) => {
                if !self.check_sysop_pw()? {
                    return Ok(Flow::Next);
                }
                match c2 {
                    'U' => self.user_editor()?,
                    'B' => self.board_editor()?,
                    'S' => self.config_editor()?,
                    '#' => self.menu_editor()?,
                    'C' => self.change_user()?,
                    'L' => self.show_log()?,
                    'Z' => self.show_zlog()?,
                    'D' => self.mini_dos()?,
                    '3' => self.read_all_mail()?,
                    _ => self.no_cmd(cmd, data)?,
                }
            }
            _ => self.no_cmd(cmd, data)?,
        }
        Ok(Flow::Next)
    }

    /// Unknown command keys: logged for the SysOp, as MENUS.PAS does.
    fn no_cmd(&mut self, cmd: &str, data: &str) -> io::Result<()> {
        self.log(&format!("Invalid command: cmdkeys \"{cmd}\" data \"{data}\""));
        if self.is_sysop() {
            self.sprint(&format!("|NL|12Invalid command keys \"{cmd}\"."))?;
        }
        Ok(())
    }

    // ---- user stuff -------------------------------------------------------

    pub fn your_info(&mut self) -> io::Result<()> {
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

    pub fn user_config(&mut self) -> io::Result<()> {
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

    pub fn chat(&mut self, _data: &str) -> io::Result<()> {
        self.sprint("|NL|U1The SysOp isn't available for chat right now.")?;
        if self.pynq("|U1Leave a message to the SysOp instead? ", true)? {
            self.send_email("1")?;
        }
        Ok(())
    }

    pub fn last_callers(&mut self) -> io::Result<()> {
        let list = self.sh.db().sys.last_callers.clone();
        self.sprint("|NL|U3Last callers to |BN|NL|U2#      Handle                    From                      When")?;
        for c in list {
            self.sprint(&format!("|U1{:<6} |U3{:<25} |U1{:<25} |U2{}", c.caller_num, c.name, c.city_state, c.when))?;
        }
        Ok(())
    }

    pub fn user_list(&mut self) -> io::Result<()> {
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

    pub fn whos_online(&mut self) -> io::Result<()> {
        let list: Vec<(usize, String, String)> =
            self.sh.db().online.iter().map(|(n, o)| (*n, o.name.clone(), o.doing.clone())).collect();
        self.sprint("|NL|U2Node  Handle                         Activity")?;
        for (n, name, doing) in list {
            self.sprint(&format!("|U3{n:<5} |U1{name:<30} |U3{doing}"))?;
        }
        Ok(())
    }

    // ---- automessage, oneliners, rumors -----------------------------------

    pub fn read_auto(&mut self) -> io::Result<()> {
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

    pub fn write_auto(&mut self) -> io::Result<()> {
        if self.user.ac.contains('A') {
            return self.sprint("|NL|12You are restricted from writing automessages.");
        }
        self.printf("AUTOWRT")?;
        let Some(body) = self.editor(30, None)? else { return self.sprint(&self.cfg().strings.aborted.clone()) };
        let anon = self.pynq("|U1Post anonymously? ", false)?;
        let mut lines = vec![if anon { format!("@{}", self.user.name) } else { self.user.name.clone() }];
        lines.extend(body);
        self.sh.db().put_lines("auto.msg", &lines)?;
        self.log("Changed the AutoMessage");
        self.sprint("|U1AutoMessage saved.")
    }

    pub fn oneliners(&mut self) -> io::Result<()> {
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

    pub fn random_rumor(&mut self) -> io::Result<()> {
        let list = self.sh.db().lines("rumors.txt");
        if list.is_empty() {
            return self.sprint("|NL|U1No rumors exist!");
        }
        let n = rand(list.len());
        self.tmpl("rumors.ran", &[("RR", list[n].clone()), ("RI", (n + 1).to_string())], "|NL|U1Rumor: |U3%RR")
    }

    pub fn add_rumor(&mut self) -> io::Result<()> {
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

    pub fn list_rumors(&mut self) -> io::Result<()> {
        let list = self.sh.db().lines("rumors.txt");
        self.tmpl("rumors.top", &[], "|NL|U3Rumors")?;
        for (i, r) in list.iter().enumerate() {
            self.tmpl("rumors.mid", &[("RR", r.clone()), ("RI", (i + 1).to_string())], "|U2%RI. |U1%RR")?;
        }
        self.tmpl("rumors.bot", &[], "")
    }

    pub fn delete_rumor(&mut self) -> io::Result<()> {
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
}

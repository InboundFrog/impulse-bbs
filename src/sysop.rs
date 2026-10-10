//! SysOp `%` commands (MENUS.PAS, SYSOP*.PAS): user editor, board and
//! system configuration editors, menu editor, logs, ZLOG and impDos.

use crate::db::User;
use crate::edit::tmpl;
use crate::menu;
use crate::session::{Session, find_ci};
use std::io;
use std::path::{Path, PathBuf};

impl Session {
    /// checkpw: the SysOp password, when one is set.
    pub fn check_sysop_pw(&mut self) -> io::Result<bool> {
        let pw = self.cfg().sysop_password.clone();
        if pw.is_empty() {
            return Ok(true);
        }
        self.sprompt("|NL|U1SysOp password: |U3")?;
        let s = self.input_pw(20)?;
        if s != pw {
            if !s.is_empty() {
                self.log("* Wrong SysOp password");
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Store an edited user, and tell their session if they're online.
    pub fn store_user(&mut self, n: usize, u: User) -> io::Result<()> {
        if n == self.unum {
            self.user = u.clone();
        }
        let mut db = self.sh.db();
        db.users[n - 1] = u;
        for o in db.online.values_mut().filter(|o| o.unum == n) {
            o.reload = true;
        }
        db.save()
    }

    /// Only edit users below you, unless you're user #1 or it's you.
    fn can_edit(&self, n: usize, u: &User) -> bool {
        self.unum == 1 || n == self.unum || (self.user.sl > u.sl && self.user.dsl > u.dsl)
    }

    fn show_user(&mut self, n: usize, u: &User) -> io::Result<()> {
        let yn = |b: bool| if b { "Yes" } else { "No" };
        let mut flags = String::new();
        if u.deleted {
            flags += " |12[DELETED]";
        }
        if u.locked_out {
            flags += " |12[LOCKED OUT]";
        }
        self.sprint(&format!("|NL|U3User #{n}{flags}"))?;
        let rows = [
            ("Name", u.name.clone()),
            ("Real name", u.real_name.clone()),
            ("City/State", u.city_state.clone()),
            ("Phone", u.phone.clone()),
            ("Birthday/Sex", format!("{} {}", u.birthday, u.sex)),
            ("SL / DSL", format!("{} / {}", u.sl, u.dsl)),
            ("AR / Restrict", format!("{} / {}", u.ar, u.ac)),
            ("Note", u.note.clone()),
            ("First / Last on", format!("{} / {}", u.first_on, u.last_on)),
            ("Calls / Posts", format!("{} / {}", u.logons, u.posts)),
            ("UL / DL", format!("{} ({}k) / {} ({}k)", u.uploads, u.uk, u.downloads, u.dk)),
            ("File points", u.file_points.to_string()),
            ("Time today/bank", format!("{} / {}", u.minutes_today, u.time_bank)),
            ("ANSI / Hot keys", format!("{} / {}", yn(u.ansi), yn(u.hotkeys))),
        ];
        for (k, v) in rows {
            self.sprompt(&format!("|U1{k:<16}: |U3"))?;
            self.out(&v)?;
            self.nl()?;
        }
        Ok(())
    }

    /// `%U` (uedit in SYSOP3.PAS).
    pub fn user_editor(&mut self) -> io::Result<()> {
        self.log("* User editor");
        let mut n = self.unum.max(1);
        loop {
            let (total, u) = {
                let db = self.sh.db();
                (db.users.len(), db.users.get(n - 1).cloned())
            };
            let Some(mut u) = u else { return Ok(()) };
            self.show_user(n, &u)?;
            self.sprompt("|NL|U1User editor |U2(|U3?|U2=help)|U1: |U3")?;
            let c = self.onek("Q?[]U*^@$E\r")?;
            let edits = "*^@$E".contains(c);
            if edits && !self.can_edit(n, &u) {
                self.log(&format!("UEDIT: tried to modify {} #{n}", u.name));
                self.sprint("|12Access denied.")?;
                continue;
            }
            match c {
                'Q' => return Ok(()),
                '?' => {
                    for l in [
                        "|U3[|U1/|U3]|U1 back/forward one user   |U3U|U1 go to user name/#",
                        "|U3E|U1 edit fields                   |U3*|U1 auto-validate",
                        "|U3^|U1 delete/restore                |U3@|U1 lock out/unlock",
                        "|U3$|U1 set password                  |U3Q|U1 quit",
                    ] {
                        self.sprint(l)?;
                    }
                }
                '[' => n = if n <= 1 { total } else { n - 1 },
                ']' => n = if n >= total { 1 } else { n + 1 },
                'U' => {
                    self.sprompt("|U1User name or #: |U3")?;
                    let s = self.input(36)?;
                    let found = {
                        let db = self.sh.db();
                        s.parse::<usize>().ok().filter(|&i| i >= 1 && i <= db.users.len()).or_else(|| db.find_user(&s))
                    };
                    match found {
                        Some(i) => n = i,
                        None => self.sprint("|12Not found.")?,
                    }
                }
                '*' => {
                    self.auto_validate(&mut u);
                    self.store_user(n, u)?;
                }
                '^' => {
                    if u.deleted {
                        if self.pynq("|U1Restore this user? ", false)? {
                            u.deleted = false;
                            self.store_user(n, u)?;
                        }
                    } else if u.ac.contains('4') || n == 1 {
                        self.log(&format!("* Attempt to delete user {} #{n}", u.name));
                        self.sprint("|12Access denied - this user is protected from deletion.")?;
                    } else if self.pynq("|U1Any e-mail to or from this user will be deleted. |12*DELETE*|U1 this user? ", false)? {
                        self.log(&format!("* Deleted user {} #{n}", u.name));
                        self.nuke_user(n)?;
                    }
                }
                '@' => {
                    u.locked_out = !u.locked_out;
                    if u.locked_out {
                        self.sprompt("|U1Lockout display file (Enter for LOCKOUT): |U3")?;
                        u.locked_file = self.input(8)?;
                        self.log(&format!("UEDIT: locked {} out", u.name));
                    }
                    self.sprint(if u.locked_out { "|U1User is now locked out." } else { "|U1User is no longer locked out." })?;
                    self.store_user(n, u)?;
                }
                '$' => {
                    self.sprompt("|U1New password: |U3")?;
                    let p = self.input(20)?;
                    if p.chars().count() >= 4 {
                        u.pw_hash = crate::db::hash_pw(&p);
                        self.store_user(n, u)?;
                        self.sprint("|U1Password changed.")?;
                    }
                }
                'E' => {
                    let title = format!("User #{n}: {}", u.name);
                    if let Some(new) = self.edit_rec(&title, &u, &["pw_hash", "last_read"], &[])? {
                        if new.name != u.name && self.sh.db().find_user(&new.name).is_some_and(|i| i != n) {
                            self.sprint("|12That name is already in use.")?;
                        } else {
                            self.store_user(n, new)?;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// `%B` (boardedit in SYSOP2*.PAS).
    pub fn board_editor(&mut self) -> io::Result<()> {
        self.log("* Message base edit");
        let boards = self.cfg().boards.clone();
        if let Some(new) = self.edit_vec("Message bases", &boards, &[])? {
            self.sh.edit_cfg(|c| c.boards = new)?;
            self.board = self.board.min(self.cfg().boards.len().saturating_sub(1));
        }
        Ok(())
    }

    /// Mark user `n` deleted and remove their e-mail (nukeuser).
    pub fn nuke_user(&mut self, n: usize) -> io::Result<()> {
        let Some(mut u) = self.sh.db().users.get(n - 1).cloned() else { return Ok(()) };
        u.deleted = true;
        {
            let mut db = self.sh.db();
            let mail = db.msgs("email")?;
            for m in mail.iter().filter(|m| m.to_num == n || m.from_num == n) {
                db.delete_msg("email", m.id)?;
            }
        }
        self.store_user(n, u)
    }

    /// `%S` (pullconfig): every system setting.
    pub fn config_editor(&mut self) -> io::Result<()> {
        self.log("* System configuration edit");
        let cfg = (*self.cfg()).clone();
        if let Some(new) = self.edit_rec("System configuration", &cfg, &[], &crate::config::templates())? {
            self.sh.edit_cfg(|c| *c = new)?;
            self.sprint("|U1Configuration saved.")?;
        }
        Ok(())
    }

    /// `%P` (exproedit): the transfer protocol list.
    pub fn protocol_editor(&mut self) -> io::Result<()> {
        self.log("* Protocol edit");
        let (_, tmpl) = tmpl::<crate::config::Protocol>("protocols");
        let mut v = serde_json::to_value(&self.cfg().protocols).map_err(io::Error::other)?;
        if self.edit_value("Transfer protocols", &mut v, Some(tmpl), &[])? {
            let new = serde_json::from_value(v).map_err(io::Error::other)?;
            self.sh.edit_cfg(|c| c.protocols = new)?;
            self.sprint("|U1Protocols saved.")?;
        }
        Ok(())
    }

    /// `%#` (menu_edit).
    pub fn menu_editor(&mut self) -> io::Result<()> {
        self.log("* Menu edit");
        let dir = self.cfg().menu_path.clone();
        loop {
            let mut names: Vec<String> = std::fs::read_dir(&dir)?
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|f| f.to_uppercase().ends_with(".MNU"))
                .map(|f| f[..f.len() - 4].to_uppercase())
                .collect();
            names.sort();
            self.sprint("|NL|U3Menus:")?;
            self.sprint(&format!("|U1{}", names.join("  ")))?;
            self.sprompt("|U1Menu to edit, or new name (Enter quits): |U3")?;
            let name = self.input_caps(8)?;
            if name.is_empty() {
                return Ok(());
            }
            let path = find_ci(&dir, &format!("{name}.MNU")).unwrap_or_else(|| dir.join(format!("{name}.MNU")));
            let m = match std::fs::read(&path) {
                Ok(b) => menu::parse(&b.iter().map(|&b| b as char).collect::<String>()),
                Err(_) => {
                    if !self.pynq(&format!("|U1Create menu |U3{name}|U1? "), true)? {
                        continue;
                    }
                    menu::Menu { title: [name.clone(), String::new(), String::new()], prompt: "|U1Command: |U3".into(), ..Default::default() }
                }
            };
            if let Some(new) = self.edit_rec(&format!("Menu {name}"), &m, &[], &[tmpl::<menu::Command>("cmds")])? {
                let bytes: Vec<u8> = menu::to_text(&new).chars().map(|c| c as u32 as u8).collect();
                std::fs::write(&path, bytes)?;
                self.sprint("|U1Menu saved.")?;
                if name == self.menu_name {
                    self.goto_menu(&name)?;
                }
            }
        }
    }

    /// `%C` (chuser): become another user.
    pub fn change_user(&mut self) -> io::Result<()> {
        self.sprompt("|NL|U1Change to which user? |U3")?;
        let s = self.input(36)?;
        let Some(n) = self.sh.db().find_user(&s) else { return self.sprint("|12Not found.") };
        self.save_user();
        self.log(&format!("* Changed to user #{n}"));
        self.user = self.sh.db().users[n - 1].clone();
        self.unum = n;
        if let Some(o) = self.sh.db().online.get_mut(&self.node) {
            o.name = self.user.name.clone();
            o.unum = n;
        }
        self.sprint("|U1You are now |U3|UN|U1.")
    }

    /// `%L`: the end of the SysOp log.
    pub fn show_log(&mut self) -> io::Result<()> {
        let text = self.sh.db().log_text();
        let lines: Vec<&str> = text.lines().collect();
        self.nl()?;
        for l in &lines[lines.len().saturating_sub(200)..] {
            self.out(l)?;
            self.nl()?;
        }
        Ok(())
    }

    /// `%Z`: daily statistics.
    pub fn show_zlog(&mut self) -> io::Result<()> {
        self.log("+ Viewed ZLOG");
        let days = {
            let mut db = self.sh.db();
            let mut v = vec![db.sys.zlog().clone()];
            v.extend(db.sys.history.iter().cloned());
            v
        };
        self.sprint("|NL|U3Date        Mins Calls New Posts Email Fback   UL   DL    UK    DK")?;
        for z in days {
            self.sprint(&format!(
                "|U1{:<10} {:>5} {:>5} {:>3} {:>5} {:>5} {:>5} {:>4} {:>4} {:>5} {:>5}",
                z.date, z.active, z.calls, z.new_users, z.pub_posts, z.priv_posts, z.feedback, z.uploads, z.downloads, z.uk, z.dk
            ))?;
        }
        Ok(())
    }

    /// `%D` impDos: a small file shell, kept inside the BBS directory.
    pub fn mini_dos(&mut self) -> io::Result<()> {
        self.log("* Entered impDos");
        let root = std::env::current_dir()?.canonicalize()?;
        let mut cwd = root.clone();
        self.sprint("|NL|U3impDos|U1 - type |U3HELP|U1 for commands, |U3EXIT|U1 to leave.")?;
        loop {
            let shown = cwd.strip_prefix(&root).unwrap_or(Path::new("")).display().to_string();
            self.sprompt(&format!("|U3\\{}|U1> |U3", shown.replace('/', "\\")))?;
            let line = self.input(78)?;
            if ["EXIT", "QUIT"].contains(&line.to_uppercase().as_str()) {
                return Ok(());
            }
            match dos_cmd(&root, &mut cwd, &line) {
                Ok(out) => {
                    for l in out {
                        self.out(&l)?;
                        self.nl()?;
                    }
                }
                Err(e) => self.sprint(&format!("|12{e}"))?,
            }
        }
    }
}

/// Resolve `p` against `cwd`; None if it would leave `root`.
fn resolve(root: &Path, cwd: &Path, p: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in cwd.join(p.replace('\\', "/")).components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out.starts_with(root).then_some(out)
}

/// One impDos command; returns the lines to print.
fn dos_cmd(root: &Path, cwd: &mut PathBuf, line: &str) -> Result<Vec<String>, String> {
    let mut it = line.split_whitespace();
    let cmd = it.next().unwrap_or("").to_uppercase();
    let (a, b, c) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or("."));
    let res = |p: &str| resolve(root, cwd, p).ok_or_else(|| "Access denied".to_string());
    let e = |e: io::Error| e.to_string();
    match cmd.as_str() {
        "" => Ok(vec![]),
        "HELP" | "?" => {
            Ok(vec!["DIR [path]  CD path  TYPE file  DEL file  REN old new  COPY from to  MD dir  RD dir  LADLE ansi template [dizdir]  EXIT".into()])
        }
        "DIR" | "LS" => {
            let mut ents: Vec<_> = std::fs::read_dir(res(a)?).map_err(e)?.flatten().collect();
            ents.sort_by_key(|e| e.file_name());
            Ok(ents
                .iter()
                .map(|en| {
                    let m = en.metadata().ok();
                    let size = match m {
                        Some(m) if m.is_dir() => "<DIR>".to_string(),
                        Some(m) => m.len().to_string(),
                        None => "?".into(),
                    };
                    format!("{:<30} {size:>12}", en.file_name().to_string_lossy())
                })
                .collect())
        }
        "CD" => {
            let p = res(a)?;
            if !p.is_dir() {
                return Err("Invalid directory".into());
            }
            *cwd = p;
            Ok(vec![])
        }
        "TYPE" => {
            let text: String = std::fs::read(res(a)?).map_err(e)?.iter().map(|&b| b as char).collect();
            Ok(text.lines().map(String::from).collect())
        }
        "DEL" => std::fs::remove_file(res(a)?).map(|_| vec![]).map_err(e),
        "MD" => std::fs::create_dir(res(a)?).map(|_| vec![]).map_err(e),
        "RD" => std::fs::remove_dir(res(a)?).map(|_| vec![]).map_err(e),
        "REN" => std::fs::rename(res(a)?, res(b)?).map(|_| vec![]).map_err(e),
        "COPY" => std::fs::copy(res(a)?, res(b)?).map(|_| vec![]).map_err(e),
        "LADLE" => {
            let tem = std::fs::read(res(b)?).map_err(|_| format!("{b} not found!"))?;
            let Some(sc) = sauce(&std::fs::read(res(a)?).map_err(e)?) else { return Ok(vec!["No SAUCE record.".into()]) };
            let tem: String = tem.iter().map(|&b| b as char).collect();
            let diz = [("|ST", 0), ("|SA", 1), ("|SG", 2), ("|SV", 3), ("|SD", 4)].iter().fold(tem, |t, (k, i)| t.replace(k, &sc[*i]));
            let bytes: Vec<u8> = diz.chars().map(|c| c as u8).collect();
            std::fs::write(res(c)?.join("FILE_ID.DIZ"), bytes).map(|_| vec!["FILE_ID.DIZ written.".into()]).map_err(e)
        }
        _ => Err("Bad command or file name.".into()),
    }
}

/// SAUCE (ASMSAUCE.PAS): title, author, group, version, date as MM/DD/CCYY.
fn sauce(b: &[u8]) -> Option<[String; 5]> {
    let r = b.get(b.len().checked_sub(128)?..)?;
    if !r.starts_with(b"SAUCE") {
        return None;
    }
    let f = |a: usize, n: usize| r[a..a + n].iter().map(|&c| c as char).collect::<String>().trim_end_matches(['\0', ' ']).to_string();
    let d = f(82, 8);
    let date = if d.len() == 8 { format!("{}/{}/{}", &d[4..6], &d[6..8], &d[..4]) } else { d };
    Some([f(7, 35), f(42, 20), f(62, 20), f(5, 2), date])
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn dos_stays_inside_root() {
        let root = Path::new("/bbs");
        assert_eq!(super::resolve(root, root, "text/x.ans"), Some("/bbs/text/x.ans".into()));
        assert_eq!(super::resolve(root, &root.join("text"), "..\\menus"), Some("/bbs/menus".into()));
        assert_eq!(super::resolve(root, root, "../etc/passwd"), None);
        assert_eq!(super::resolve(root, root, "/etc/passwd"), None);
    }

    #[test]
    fn sauce() {
        let mut b = b"art\x1aSAUCE00".to_vec();
        b.extend(format!("{:<35}{:<20}{:<20}19950704", "Frog Pic", "Mike", "ACiD").bytes());
        b.resize(4 + 128, 0);
        assert_eq!(super::sauce(&b).unwrap(), ["Frog Pic", "Mike", "ACiD", "00", "07/04/1995"].map(String::from));
        assert!(super::sauce(b"short").is_none());
    }
}

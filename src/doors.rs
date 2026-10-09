//! Doors and external programs (DOORS.PAS): drop files plus the telnet bridge.

use crate::session::{Session, strip_pipes};
use chrono::Timelike;
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// "JOHN Q SMITH" -> ("JOHN", "Q SMITH")
fn split_name(n: &str) -> (&str, &str) {
    n.split_once(' ').unwrap_or((n, ""))
}

/// Expand `@x` macros in a door command line (process_door).
fn expand(cline: &str, m: impl Fn(char) -> Option<String>) -> String {
    let mut out = String::new();
    let mut it = cline.chars();
    while let Some(c) = it.next() {
        if c != '@' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some(k) => out.push_str(&m(k.to_ascii_uppercase()).unwrap_or_default()),
            None => out.push('@'),
        }
    }
    out
}

impl Session {
    /// `Dx` / `ID` (dodoorfunc). `kind`: G DOOR.SYS, D DORINFO1.DEF,
    /// C CHAIN.TXT, I IMPULSE.IDF, anything else no drop file.
    /// A leading `R;` puts the real name in the drop file.
    pub fn door(&mut self, kind: char, cline: &str) -> io::Result<()> {
        let (real, cline) = match cline.get(..2) {
            Some(p) if p.eq_ignore_ascii_case("R;") => (true, &cline[2..]),
            _ => (false, cline),
        };
        if cline.trim().is_empty() {
            return self.sprint("|NL|U1This command is inoperative!");
        }
        let dir = self.cfg().data_path.join(format!("node{}", self.node));
        std::fs::create_dir_all(&dir)?;
        match kind {
            'G' => std::fs::write(dir.join("DOOR.SYS"), self.door_sys(real))?,
            'D' => std::fs::write(dir.join("DORINFO1.DEF"), self.dorinfo(real))?,
            'C' => std::fs::write(dir.join("CHAIN.TXT"), self.chain_txt())?,
            'I' => std::fs::write(dir.join("IMPULSE.IDF"), self.idf())?,
            _ => {}
        }
        let cmd = self.door_cmd(cline, &dir);
        self.log(&format!(">> Door \"{cmd}\""));
        self.set_doing("In a door");
        self.save_user();
        let t = Instant::now();
        let r = self.t.run_external(Command::new("sh").arg("-c").arg(&cmd).current_dir(&dir), true);
        match r {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Err(e),
            Err(e) => self.log(&format!("Door failed: {e}")),
            Ok(_) => {}
        }
        self.log(&format!(">> Returned (spent {} min)", t.elapsed().as_secs() / 60));
        Ok(())
    }

    fn door_cmd(&self, cline: &str, dir: &Path) -> String {
        let c = self.cfg();
        let real = self.user.real_name.to_uppercase();
        let (first, last) = split_name(&real);
        let fb = c.fbases.get(self.fbase);
        expand(cline, |k| {
            Some(match k {
                'B' => "0".into(),
                'D' => fb?.dl_path.display().to_string(),
                'F' => first.into(),
                'G' => (self.user.ansi as u8).to_string(),
                'I' => c.data_path.join("files").join(format!("{}.json", fb?.file)).display().to_string(),
                'L' => last.into(),
                'N' => self.user.name.clone(),
                'T' => self.time_left_min().to_string(),
                'R' => self.unum.to_string(),
                // ponytail: @P/@U are additions; Impulse ran doors from fixed dirs
                'P' => dir.display().to_string(),
                'U' => self.node.to_string(),
                _ => return None,
            })
        })
    }

    fn door_name(&self, real: bool) -> String {
        if real { self.user.real_name.clone() } else { self.user.name.clone() }
    }

    /// GAP's DOOR.SYS (write_door_sys).
    fn door_sys(&self, real: bool) -> String {
        let u = &self.user;
        let left = self.time_left_min();
        let ar: String = (1..=7).filter(|i| u.ar.contains((b'A' + i - 1) as char)).map(|i| i.to_string()).collect();
        let l = [
            "COM0:".into(),
            "38400".into(),
            " 8".into(),
            " 1".into(),
            " N".into(),
            " Y".into(),
            " Y".into(),
            " N".into(),
            " N".into(),
            self.door_name(real),
            u.city_state.clone(),
            u.phone.clone(),
            u.phone.clone(),
            String::new(), // password: only the hash is kept
            u.sl.to_string(),
            u.logons.to_string(),
            u.last_on.clone(),
            (left * 60).to_string(),
            left.to_string(),
            if u.ansi { "GR" } else { "NG" }.into(),
            u.page_len.to_string(),
            if u.novice { " N" } else { " Y" }.into(),
            ar,
            "7".into(),
            "12/31/99".into(),
            format!(" {}", self.unum),
            " X".into(),
            format!(" {}", u.uploads),
            format!(" {}", u.downloads),
            format!(" {}", u.dk),
            " 999999".into(),
        ];
        l.map(|s| s + "\r\n").concat()
    }

    /// RBBS-PC's DORINFO1.DEF (write_dorinfo1_def).
    fn dorinfo(&self, real: bool) -> String {
        let c = self.cfg();
        let (sf, sl) = split_name(&c.sysop_name);
        let name = self.door_name(real);
        let (f, l) = split_name(&name);
        let left = self.time_left_min().min(999);
        let v = [
            strip_pipes(&c.bbs_name),
            sf.into(),
            sl.into(),
            "COM0".into(),
            "0 BAUD,N,8,1".into(),
            "0".into(),
            f.to_uppercase(),
            l.to_uppercase(),
            self.user.city_state.clone(),
            (self.user.ansi as u8).to_string(),
            self.user.sl.to_string(),
            left.to_string(),
            "0".into(),
        ];
        v.map(|s| s + "\r\n").concat()
    }

    /// WWIV's CHAIN.TXT (write_chain_txt).
    fn chain_txt(&self) -> String {
        let c = self.cfg();
        let u = &self.user;
        let b = |x: bool| (x as u8).to_string();
        let used = self.on_since_secs();
        let midnight = chrono::Local::now().num_seconds_from_midnight() as u64;
        let v = [
            self.unum.to_string(),
            u.name.clone(),
            u.real_name.clone(),
            String::new(),
            crate::session::age(&u.birthday).to_string(),
            u.sex.clone(),
            "      0.00".into(),
            u.last_on.clone(),
            "80".into(),
            u.page_len.to_string(),
            u.sl.to_string(),
            b(self.is_sysop()),
            b(self.is_sysop()),
            b(u.ansi),
            "1".into(),
            format!("{:10.2}", (self.time_left_min() * 60) as f64),
            c.text_path.display().to_string(),
            c.data_path.display().to_string(),
            "SYSOP.LOG".into(),
            "0".into(),
            "0".into(),
            strip_pipes(&c.bbs_name),
            c.sysop_name.clone(),
            midnight.saturating_sub(used).to_string(),
            used.to_string(),
            u.uk.to_string(),
            u.uploads.to_string(),
            u.dk.to_string(),
            u.downloads.to_string(),
            "8N1".into(),
        ];
        v.map(|s| s + "\r\n").concat()
    }

    /// IMPULSE.IDF (imp_drop_file).
    fn idf(&self) -> String {
        let c = self.cfg();
        let u = &self.user;
        let v = [
            ("COMDRIVER", "TELNET".to_string()),
            ("COMPORT", "0".into()),
            ("BPSRATE", "38400".into()),
            ("VERSION", format!("v{}", env!("CARGO_PKG_VERSION"))),
            ("BBSNAME", c.bbs_name.clone()),
            ("SYSOPNAME", c.sysop_name.clone()),
            ("HANDLE", u.name.clone()),
            ("REALNAME", u.real_name.clone()),
            ("PASSWORD", String::new()),
            ("SLEVEL", u.sl.to_string()),
            ("DLEVEL", u.dsl.to_string()),
            ("FILEPOINTS", u.file_points.to_string()),
            ("PROMPT#", "0".into()),
            ("EMULATION", "ANSI".into()),
            ("UPLOADS", u.uploads.to_string()),
            ("DOWNLOADS", u.downloads.to_string()),
            ("UPLOADED-K", u.uk.to_string()),
            ("DOWNLOADED-K", u.dk.to_string()),
            ("CURRENT-TIME", self.time_left_min().to_string()),
            ("TIME-LEFT", self.time_left_min().to_string()),
        ];
        v.iter().map(|(k, x)| format!("{:<14}{x}\r\n", format!("{k}:"))).collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn door_macros() {
        let m = |k| match k {
            'N' => Some("BOB".to_string()),
            'T' => Some("42".to_string()),
            _ => None,
        };
        assert_eq!(super::expand("door.exe @N @t @Z x@", m), "door.exe BOB 42  x@");
        assert_eq!(super::split_name("JOHN Q SMITH"), ("JOHN", "Q SMITH"));
    }
}

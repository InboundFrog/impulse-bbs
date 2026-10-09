//! QWK offline mail: `MQ` packs new messages into BBSID.QWK, `MR` takes a
//! BBSID.REP of replies. Conference 0 is e-mail; conference n is board n.
//! Zipping is done by zip(1)/unzip(1).

use crate::db::Msg;
use crate::session::{Session, strip_pipes};
use std::io;
use std::path::Path;
use std::process::Command;

const BLOCK: usize = 128;
/// QWK line terminator.
const EOL: u8 = 0xE3;

fn field(s: &str, w: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take(w).collect();
    v.resize(w, b' ');
    v
}

/// "YYYY-MM-DD HH:MM" -> ("MM-DD-YY", "HH:MM")
fn qwk_date(d: &str) -> (String, String) {
    let (date, time) = d.split_once(' ').unwrap_or((d, "00:00"));
    let p: Vec<&str> = date.split('-').collect();
    match p.as_slice() {
        [y, m, d] => (format!("{m}-{d}-{}", &y[y.len().saturating_sub(2)..]), time.to_string()),
        _ => ("01-01-80".into(), time.to_string()),
    }
}

/// One message as header + body blocks; `from` is the name as shown.
pub fn pack_msg(status: u8, num: usize, conf: u16, from: &str, m: &Msg) -> Vec<u8> {
    let mut text: Vec<u8> = Vec::new();
    for l in &m.body {
        text.extend(field(l, l.chars().count()));
        text.push(EOL);
    }
    let blocks = text.len().div_ceil(BLOCK) + 1;
    text.resize((blocks - 1) * BLOCK, b' ');
    let (d, t) = qwk_date(&m.date);
    let mut h = Vec::with_capacity(BLOCK);
    h.push(status);
    h.extend(field(&num.to_string(), 7));
    h.extend(field(&d, 8));
    h.extend(field(&t, 5));
    h.extend(field(&m.to.to_uppercase(), 25));
    h.extend(field(&from.to_uppercase(), 25));
    h.extend(field(&m.subject, 25));
    h.extend(field("", 12));
    h.extend(field(&if m.reply_to > 0 { m.reply_to.to_string() } else { String::new() }, 8));
    h.extend(field(&blocks.to_string(), 6));
    h.push(0xE1);
    h.extend((conf).to_le_bytes());
    h.extend((num as u16).to_le_bytes());
    h.push(b' ');
    h.extend(text);
    h
}

pub struct Reply {
    pub conf: u16,
    pub private: bool,
    pub to: String,
    pub subject: String,
    pub body: Vec<String>,
}

/// Messages in a BBSID.MSG reply file (first block is the BBS id).
pub fn parse_rep(b: &[u8]) -> Vec<Reply> {
    let s = |r: &[u8]| r.iter().map(|&c| c as char).collect::<String>().trim().to_string();
    let mut out = Vec::new();
    let mut off = BLOCK;
    while off + BLOCK <= b.len() {
        let h = &b[off..off + BLOCK];
        let blocks: usize = s(&h[116..122]).parse().unwrap_or(1).max(1);
        let end = (off + blocks * BLOCK).min(b.len());
        // Readers put the conference in the number field; some also in 123-124.
        let conf = s(&h[1..8]).parse().unwrap_or(u16::from_le_bytes([h[123], h[124]]));
        let text: String = b[off + BLOCK..end].iter().map(|&c| c as char).collect();
        let mut body: Vec<String> = text.split(EOL as char).map(|l| l.trim_end().to_string()).collect();
        while body.last().is_some_and(|l| l.is_empty()) {
            body.pop();
        }
        out.push(Reply { conf, private: matches!(h[0], b'*' | b'+'), to: s(&h[21..46]), subject: s(&h[71..96]), body });
        off = end;
    }
    out
}

impl Session {
    fn qwk_dir(&self) -> io::Result<std::path::PathBuf> {
        let d = self.cfg().data_path.join("qwk").join(self.node.to_string());
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// `MQ` (sendqwk)
    pub fn send_qwk(&mut self) -> io::Result<()> {
        let c = self.cfg();
        let id = c.qwk_id.to_uppercase();
        let dir = self.qwk_dir()?;
        let mut dat = field(&format!("Produced by Impulse v{}", env!("CARGO_PKG_VERSION")), BLOCK);
        let mut n = 0;
        let mut new_last: Vec<(String, u64)> = Vec::new();
        let mail: Vec<Msg> = self.sh.db().msgs("email")?.into_iter().filter(|m| m.to_num == self.unum).collect();
        for m in &mail {
            n += 1;
            dat.extend(pack_msg(b'*', n, 0, &self.sender(m, true), m));
        }
        let mso = self.mso();
        for b in self.boards() {
            let board = c.boards[b].clone();
            if self.user.no_scan.contains(&board.file) || (!board.password.is_empty() && !self.board_pw.contains(&board.file)) {
                continue;
            }
            let last = self.user.last_read.get(&board.file).copied().unwrap_or(0);
            let msgs: Vec<Msg> =
                self.sh.db().msgs(&board.file)?.into_iter().filter(|m| m.id > last && (!m.unvalidated || mso || m.from_num == self.unum)).collect();
            for m in msgs.iter().take(c.qwk_max.saturating_sub(n)) {
                n += 1;
                let plain = Msg { subject: strip_pipes(&m.subject), body: m.body.iter().map(|l| strip_pipes(l)).collect(), ..m.clone() };
                dat.extend(pack_msg(b' ', n, b as u16 + 1, &self.sender(m, false), &plain));
                new_last.push((board.file.clone(), m.id));
            }
        }
        if n == 0 {
            return self.sprint("|NL|U1No new messages to pack.");
        }
        let now = chrono::Local::now();
        let mut ctl = vec![
            strip_pipes(&c.bbs_name),
            String::new(),
            String::new(),
            c.sysop_name.clone(),
            format!("00000,{id}"),
            now.format("%m-%d-%Y,%H:%M:%S").to_string(),
            self.user.name.to_uppercase(),
            String::new(),
            "0".into(),
            n.to_string(),
            c.boards.len().to_string(), // conferences - 1, counting e-mail as 0
            "0".into(),
            "E-mail".into(),
        ];
        for (i, b) in c.boards.iter().enumerate() {
            ctl.push((i + 1).to_string());
            ctl.push(strip_pipes(&b.name).chars().take(13).collect());
        }
        ctl.extend(["HELLO".into(), "NEWS".into(), "GOODBYE".into()]);
        std::fs::write(dir.join("CONTROL.DAT"), ctl.join("\r\n") + "\r\n")?;
        std::fs::write(dir.join("MESSAGES.DAT"), &dat)?;
        let pkt = format!("{id}.QWK");
        let st = Command::new("zip").args(["-q", "-j", &pkt, "CONTROL.DAT", "MESSAGES.DAT"]).current_dir(&dir).status();
        if !st.is_ok_and(|s| s.success()) {
            return self.sprint("|12Unable to create the QWK packet (is zip installed?).");
        }
        self.sprint(&format!("|NL|U1Packed |U3{n}|U1 message(s) into |U3{pkt}|U1."))?;
        let Some(p) = self.choose_protocol(false, false)? else { return Ok(()) };
        if !self.run_protocol(&p.dl_cmd, &[dir.join(&pkt)], &dir)? {
            return self.sprint("|NL|12Transfer aborted.");
        }
        if self.pynq("|NL|U1Update your last-read pointers? ", true)? {
            for (f, id) in new_last {
                self.user.last_read.insert(f, id);
            }
            self.save_user();
        }
        self.log(&format!("Downloaded QWK packet, {n} messages"));
        Ok(())
    }

    /// `MR` (recieverep)
    pub fn receive_rep(&mut self) -> io::Result<()> {
        let id = self.cfg().qwk_id.to_uppercase();
        let dir = self.qwk_dir()?;
        self.sprint(&format!("|NL|U1Upload your |U3{id}.REP|U1 packet."))?;
        let Some(p) = self.choose_protocol(true, false)? else { return Ok(()) };
        self.run_protocol(&p.ul_cmd, &[], &dir)?;
        let Some(rep) = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rep"))) else {
            return self.sprint("|NL|12No .REP packet received.");
        };
        let _ = Command::new("unzip").args(["-o", "-j", "-qq"]).arg(&rep).current_dir(&dir).status();
        let Some(msg) = crate::session::find_ci(&dir, &format!("{id}.MSG")) else { return self.sprint(&format!("|12{id}.MSG not found in the packet.")) };
        self.import_rep(&msg)
    }

    fn import_rep(&mut self, path: &Path) -> io::Result<()> {
        let c = self.cfg();
        let (mut ok, mut bad) = (0, 0);
        for r in parse_rep(&std::fs::read(path)?) {
            let subject = if r.subject.is_empty() { "(no subject)".to_string() } else { r.subject.clone() };
            if r.conf == 0 || r.private {
                let found = self.sh.db().find_user(&r.to);
                match found {
                    Some(n) if !self.user.ac.contains('E') => {
                        let to = self.sh.db().users[n - 1].name.clone();
                        self.deliver(&[(n, to)], self.letter(subject, r.body))?;
                        ok += 1;
                    }
                    _ => bad += 1,
                }
                continue;
            }
            let b = r.conf as usize - 1;
            let Some(board) = c.boards.get(b).filter(|_| self.boards().contains(&b)) else {
                bad += 1;
                continue;
            };
            if !self.can_post(board) {
                bad += 1;
                continue;
            }
            let strip = !self.acs(&board.mci_acs);
            let fix = |s: &str| if strip { strip_pipes(s) } else { s.to_string() };
            let to = if r.to.is_empty() || r.to.eq_ignore_ascii_case("ALL") { "All".to_string() } else { r.to.clone() };
            let m = Msg { to, subject: fix(&subject), body: r.body.iter().map(|l| fix(l)).collect(), ..self.letter(String::new(), Vec::new()) };
            self.store_post(board, m)?;
            ok += 1;
        }
        self.log(&format!("Imported REP packet: {ok} ok, {bad} refused"));
        self.sprint(&format!("|NL|U1Imported |U3{ok}|U1 message(s){}.", if bad > 0 { format!(", |12{bad} refused|U1") } else { String::new() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwk_roundtrip() {
        let body = vec!["Hello".to_string(), String::new(), "x".repeat(200)];
        let msg = Msg { to: "All".into(), subject: "Hi there".into(), date: "2026-10-09 14:05".into(), body: body.clone(), ..Msg::default() };
        let m = pack_msg(b' ', 3, 2, "Bob", &msg);
        assert_eq!(m.len() % 128, 0);
        assert_eq!(&m[8..21], b"10-09-2614:05");
        assert_eq!(&m[116..122], b"3     ");
        // a REP is the same layout after an id block
        let mut rep = field("IMPULSE", 128);
        rep.extend(m);
        let r = parse_rep(&rep);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].to.as_str(), r[0].subject.as_str(), r[0].conf), ("ALL", "Hi there", 3));
        assert_eq!(r[0].body, body);
    }
}

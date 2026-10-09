//! `impulse import DIR [config]`: one-time migration of an Impulse 7.1
//! system's USER.LST, BOARDS.DAT, UPLOADS.DAT and *.DIR files (RECORDS.PAS
//! layouts; Turbo Pascal records are packed, string[n] is a length byte + n).
//! Messages (*.BRD/*.MIX) and verbose descriptions aren't carried over.

use crate::config::{Anon, Board, Config, FBase};
use crate::db::{self, Db, FileRec, User};
use crate::session::find_ci;
use std::path::{Path, PathBuf};

const USER_LEN: usize = 843;
const BOARD_LEN: usize = 256;
const UBOARD_LEN: usize = 237;
const ULF_LEN: usize = 146;

/// Cursor over one record.
struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        s
    }
    /// string[n]
    fn s(&mut self, n: usize) -> String {
        let raw = self.take(n + 1);
        let len = (raw[0] as usize).min(n);
        raw[1..=len].iter().map(|&c| c as char).collect::<String>().trim().to_string()
    }
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    fn i16(&mut self) -> i16 {
        let s = self.take(2);
        i16::from_le_bytes([s[0], s[1]])
    }
    fn i32(&mut self) -> i32 {
        let s = self.take(4);
        i32::from_le_bytes([s[0], s[1], s[2], s[3]])
    }
    fn skip(&mut self, n: usize) {
        self.p += n;
    }
}

/// Is element `i` in a Pascal set whose lowest element is `lo`?
fn has(set: &[u8], i: usize, lo: usize) -> bool {
    let byte = i / 8 - lo / 8;
    set.get(byte).is_some_and(|b| b & (1 << (i % 8)) != 0)
}

/// "MM/DD/YY" -> "YYYY-MM-DD" (19xx from 70 up).
fn date(d: &str) -> String {
    let p: Vec<&str> = d.split(['/', '-']).collect();
    match p.as_slice() {
        [m, d, y] if y.len() == 2 => format!("{}{y}-{m:0>2}-{d:0>2}", if *y >= "70" { "19" } else { "20" }),
        _ => String::new(),
    }
}

/// DOS path -> something usable here: no drive, forward slashes.
fn dos_path(p: &str) -> PathBuf {
    let p = p.split_once(':').map_or(p, |(_, r)| r);
    PathBuf::from(p.replace('\\', "/").trim_end_matches('/').to_lowercase())
}

fn user(b: &[u8], boards: &[Board], fbases: &[FBase]) -> User {
    let mut r = R { b, p: 0 };
    let mut u = User { name: r.s(36), real_name: r.s(36), ..User::default() };
    u.pw_hash = db::hash_pw(&r.s(20));
    u.phone = r.s(12);
    let bday = r.s(8);
    u.birthday = match bday.split('/').collect::<Vec<_>>().as_slice() {
        [m, d, y] => format!("{m}/{d}/19{y}"),
        _ => String::new(),
    };
    u.first_on = date(&r.s(8));
    r.skip(2);
    u.last_on = date(&r.s(8));
    r.skip(2);
    r.s(30); // street
    u.city_state = r.s(30);
    r.s(10);
    r.skip(31);
    r.s(40); // autosig
    r.skip(41);
    u.note = r.s(39);
    r.u8();
    u.locked_out = r.u8() != 0;
    u.deleted = r.u8() != 0;
    u.locked_file = r.s(8);
    r.skip(2);
    let ac = r.take(3).to_vec();
    // uflags in RECORDS.PAS order; the unlettered ones are settings
    u.ac = "LCVBA*PEKM".chars().enumerate().filter(|&(i, _)| has(&ac, i, 0)).map(|(_, c)| c).collect();
    u.ac.extend((20..24).filter(|&i| has(&ac, i, 0)).map(|i| char::from(b'1' + (i - 20) as u8)));
    u.hotkeys = has(&ac, 11, 0);
    u.pause = has(&ac, 13, 0);
    u.novice = has(&ac, 14, 0);
    u.ansi = has(&ac, 15, 0);
    u.fse = u.ansi;
    let ff = r.take(1).to_vec();
    u.list_fields = "NSPDEWTC".chars().enumerate().filter(|&(i, _)| has(&ff, i, 0)).map(|(_, c)| c).collect();
    let ar = r.take(4).to_vec();
    u.ar = ('A'..='Z').filter(|&c| has(&ar, c as usize, '@' as usize)).collect();
    r.skip(128 + 128); // last-read pointers: messages aren't imported
    let scan = r.take(64).to_vec();
    u.no_scan = boards.iter().take(64).enumerate().filter(|&(i, _)| scan[i] == 0).map(|(_, b)| b.file.clone()).collect();
    let fscan = r.take(13).to_vec();
    u.no_fscan = fbases.iter().take(97).enumerate().filter(|&(i, _)| !has(&fscan, i, 0)).map(|(_, b)| b.file.clone()).collect();
    r.skip(20);
    u.sex = (r.u8() as char).to_string();
    u.total_minutes = r.i32() as i64;
    r.skip(2);
    u.uk = r.i32().max(0) as u64;
    r.skip(2);
    u.dk = r.i32().max(0) as u64;
    r.skip(2);
    u.uploads = r.i16().max(0) as u32;
    u.downloads = r.i16().max(0) as u32;
    u.logons = r.i16().max(0) as u32;
    u.minutes_today = r.i16() as i64;
    u.posts = r.i16().max(0) as u32;
    u.emails = r.i16().max(0) as u32;
    r.skip(4); // feedback, forward
    u.file_points = r.i16() as i32;
    r.skip(2); // waiting, linelen
    u.page_len = r.u8().max(5) as u16;
    u.on_today = r.u8() as u32;
    r.skip(1);
    u.sl = r.u8();
    u.dsl = r.u8();
    r.skip(10); // mono colours
    let cols = r.take(10);
    u.colors.copy_from_slice(cols);
    u.last_board = r.u8().saturating_sub(1) as usize;
    u.last_fbase = r.u8() as usize;
    r.skip(4 + 2);
    u.time_bank = r.i16() as i64;
    r.skip(5 + 2 + 2 + 4 + 2);
    r.s(8);
    r.skip(1 + 4);
    debug_assert_eq!(r.p, USER_LEN);
    u
}

fn board(b: &[u8]) -> Board {
    let mut r = R { b, p: 0 };
    let name = r.s(40);
    let file = r.s(8).to_lowercase();
    r.skip(4);
    r.s(40);
    let (acs, post_acs, mci_acs) = (r.s(20), r.s(20), r.s(20));
    let max_msgs = r.i16() as u16 as usize;
    let anonymous = match r.u8() {
        1 => Anon::Yes,
        2 => Anon::Forced,
        3 => Anon::DearAbby,
        4 => Anon::AnyName,
        _ => Anon::No,
    };
    let password = r.s(20).to_uppercase();
    let stat = r.take(2).to_vec();
    r.skip(4 + 2);
    r.s(50);
    r.skip(4 + 8);
    let conf = r.u8() as usize;
    debug_assert_eq!(r.p + 2, BOARD_LEN);
    Board { name, file, acs, post_acs, mci_acs, max_msgs, anonymous, password, real_names: has(&stat, 1, 0), conf }
}

/// The base plus its .DIR file name and whether that lives in the dl path.
fn fbase(b: &[u8]) -> (FBase, String, bool) {
    let mut r = R { b, p: 0 };
    let name = r.s(40);
    let dir = r.s(12);
    let (dl, ul) = (r.s(40), r.s(40));
    let max_files = r.i16().max(0) as usize;
    let password = r.s(20).to_uppercase();
    r.skip(1 + 1 + 2);
    let stat = r.u8();
    let (acs, ul_acs, name_acs) = (r.s(20), r.s(20), r.s(20));
    r.skip(4);
    let conf = r.u8() as usize;
    debug_assert_eq!(r.p + 5, UBOARD_LEN);
    let file = dir.split('.').next().unwrap_or(&dir).to_lowercase();
    let f = FBase { name, file, dl_path: dos_path(&dl), ul_path: dos_path(&ul), acs, ul_acs, name_acs, password, max_files, no_ratio: stat & 1 != 0, conf };
    let dir = if dir.contains('.') { dir } else { format!("{dir}.DIR") };
    (f, dir, stat & 4 != 0)
}

fn file_rec(b: &[u8], size_of: impl Fn(&str) -> Option<u64>) -> FileRec {
    let mut r = R { b, p: 0 };
    let name = r.s(12);
    let desc = r.s(60);
    let points = r.i16() as i32;
    let downloads = r.i16().max(0) as u32;
    r.skip(1);
    let blocks = r.i16() as u16 as u64;
    let owner_num = r.i16().max(0) as usize;
    let owner = r.s(36);
    let d = date(&r.s(8));
    r.skip(2 + 4);
    let unvalidated = r.u8() & 1 != 0;
    r.skip(1);
    r.s(8);
    debug_assert_eq!(r.p, ULF_LEN);
    let size = size_of(&name).unwrap_or(blocks * 128);
    FileRec { name, desc, ext: Vec::new(), points, downloads, size, owner, owner_num, date: d, unvalidated }
}

fn records(b: &[u8], len: usize) -> impl Iterator<Item = &[u8]> {
    b.chunks_exact(len)
}

pub fn run(dir: &Path, cfg_path: &str) -> Result<String, String> {
    let read = |name: &str| find_ci(dir, name).and_then(|p| std::fs::read(p).ok());
    let mut cfg: Config = crate::config::load(cfg_path)?;
    let db = Db::open(&cfg.data_path).map_err(|e| e.to_string())?;
    if !db.users.is_empty() {
        return Err(format!("{} already has users; import into a fresh data directory.", cfg.data_path.display()));
    }
    let mut out = Vec::new();
    if let Some(b) = read("BOARDS.DAT") {
        cfg.boards = records(&b, BOARD_LEN).map(board).filter(|b| !b.file.is_empty()).collect();
        out.push(format!("{} message bases", cfg.boards.len()));
    }
    let mut dirs = Vec::new();
    if let Some(b) = read("UPLOADS.DAT") {
        let v: Vec<_> = records(&b, UBOARD_LEN).map(fbase).filter(|(f, ..)| !f.file.is_empty()).collect();
        cfg.fbases = v.iter().map(|(f, ..)| f.clone()).collect();
        dirs = v;
        out.push(format!("{} file bases", cfg.fbases.len()));
    }
    let mut files = 0;
    for (f, name, in_dl) in &dirs {
        let found = if *in_dl { find_ci(&f.dl_path, name) } else { None }.or_else(|| find_ci(dir, name));
        let Some(b) = found.and_then(|p| std::fs::read(p).ok()) else { continue };
        let size_of = |n: &str| find_ci(&f.dl_path, n).and_then(|p| p.metadata().ok()).map(|m| m.len());
        // record 0 is a header
        let v: Vec<FileRec> = records(&b, ULF_LEN).skip(1).map(|r| file_rec(r, size_of)).filter(|r| !r.name.is_empty()).collect();
        files += v.len();
        db.put_files(&f.file, &v).map_err(|e| e.to_string())?;
    }
    if !dirs.is_empty() {
        out.push(format!("{files} files"));
    }
    let mut db = db;
    if let Some(b) = read("USER.LST") {
        // record 0 is unused; record n is user n
        db.users = records(&b, USER_LEN).skip(1).map(|r| user(r, &cfg.boards, &cfg.fbases)).collect();
        out.push(format!("{} users", db.users.len()));
    }
    if out.is_empty() {
        return Err(format!("No USER.LST, BOARDS.DAT or UPLOADS.DAT in {}", dir.display()));
    }
    let _ = std::fs::copy(cfg_path, format!("{cfg_path}.bak"));
    cfg.save(cfg_path).map_err(|e| e.to_string())?;
    db.save().map_err(|e| e.to_string())?;
    Ok(format!("Imported {}. {cfg_path} was backed up to {cfg_path}.bak; check the file paths, which came from DOS.", out.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pstr(v: &mut Vec<u8>, s: &str, n: usize) {
        v.push(s.len() as u8);
        v.extend(s.bytes());
        v.resize(v.len() + n - s.len(), 0);
    }

    #[test]
    fn layouts() {
        assert!(has(&[0, 0, 0b10], 'Q' as usize, '@' as usize));
        assert_eq!(date("07/27/98"), "1998-07-27");
        assert_eq!(dos_path("C:\\IMP\\FILES\\"), PathBuf::from("/imp/files"));

        let mut b = Vec::new();
        pstr(&mut b, "General", 40);
        pstr(&mut b, "GENERAL", 8);
        b.resize(b.len() + 4, 0);
        pstr(&mut b, "", 40);
        pstr(&mut b, "s20", 20);
        pstr(&mut b, "", 20);
        pstr(&mut b, "", 20);
        b.extend([100, 0, 2]);
        pstr(&mut b, "", 20);
        b.extend([2, 0]); // mbrealname
        b.resize(BOARD_LEN - 3, 0);
        b.extend([3, 0, 0]);
        assert_eq!(b.len(), BOARD_LEN);
        let bd = board(&b);
        assert_eq!((bd.name.as_str(), bd.file.as_str(), bd.acs.as_str(), bd.max_msgs), ("General", "general", "s20", 100));
        assert!(bd.anonymous == Anon::Forced && bd.real_names && bd.conf == 3);

        let mut u = Vec::new();
        pstr(&mut u, "THE DUDE", 36);
        pstr(&mut u, "Jeff", 36);
        pstr(&mut u, "SECRET", 20);
        u.resize(USER_LEN, 0);
        let us = user(&u, &[], &[]);
        assert_eq!((us.name.as_str(), us.real_name.as_str()), ("THE DUDE", "Jeff"));
        assert!(db::check_pw(&us.pw_hash, "SECRET"));
    }
}

//! Persistent data: users (USER.LST), system state (STATUS.DAT counters,
//! LASTON.DAT) and message bases. Stored as JSON under `data_path`.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng};
use argon2::Argon2;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default user colours, `|U0`..`|U9` (userrec.cols).
pub const DEFAULT_COLORS: [u8; 10] = [7, 3, 11, 15, 9, 13, 10, 14, 12, 8];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct User {
    pub name: String,
    pub real_name: String,
    pub pw_hash: String,
    pub phone: String,
    /// MM/DD/YYYY
    pub birthday: String,
    pub sex: String,
    pub city_state: String,
    pub note: String,
    pub sl: u8,
    pub dsl: u8,
    /// AR flags, letters A-Z.
    pub ar: String,
    /// Restriction flags by their Impulse letters (L C V B A * P E K M).
    pub ac: String,
    pub ansi: bool,
    pub pause: bool,
    pub hotkeys: bool,
    pub novice: bool,
    pub page_len: u16,
    pub colors: [u8; 10],
    pub first_on: String,
    pub last_on: String,
    pub logons: u32,
    pub on_today: u32,
    pub minutes_today: i64,
    pub total_minutes: i64,
    pub posts: u32,
    pub emails: u32,
    pub uploads: u32,
    pub downloads: u32,
    pub uk: u64,
    pub dk: u64,
    pub file_points: i32,
    pub time_bank: i64,
    pub locked_out: bool,
    pub deleted: bool,
    pub last_board: usize,
    /// Highest message id read, per board file.
    pub last_read: BTreeMap<String, u64>,
}

impl Default for User {
    fn default() -> Self {
        User {
            name: String::new(),
            real_name: String::new(),
            pw_hash: String::new(),
            phone: String::new(),
            birthday: String::new(),
            sex: String::new(),
            city_state: String::new(),
            note: String::new(),
            sl: 0,
            dsl: 0,
            ar: String::new(),
            ac: String::new(),
            ansi: true,
            pause: true,
            hotkeys: true,
            novice: true,
            page_len: 24,
            colors: DEFAULT_COLORS,
            first_on: String::new(),
            last_on: String::new(),
            logons: 0,
            on_today: 0,
            minutes_today: 0,
            total_minutes: 0,
            posts: 0,
            emails: 0,
            uploads: 0,
            downloads: 0,
            uk: 0,
            dk: 0,
            file_points: 0,
            time_bank: 0,
            locked_out: false,
            deleted: false,
            last_board: 0,
            last_read: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LastCaller {
    pub caller_num: u64,
    pub name: String,
    pub city_state: String,
    pub when: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SysState {
    pub callers: u64,
    pub next_msg_id: u64,
    pub last_callers: Vec<LastCaller>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Msg {
    pub id: u64,
    pub from: String,
    pub from_num: usize,
    pub to: String,
    /// Recipient user number for e-mail, 0 for public posts.
    pub to_num: usize,
    pub subject: String,
    pub date: String,
    pub body: Vec<String>,
}

pub struct Online {
    pub name: String,
    pub doing: String,
}

pub struct Db {
    dir: PathBuf,
    /// User number N is `users[N - 1]`, as in USER.LST.
    pub users: Vec<User>,
    pub sys: SysState,
    pub online: BTreeMap<usize, Online>,
}

fn load<T: DeserializeOwned + Default>(p: &Path) -> io::Result<T> {
    match fs::read(p) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

/// Write via a temp file + rename so a crash never leaves a half-written file.
fn store<T: Serialize + ?Sized>(p: &Path, v: &T) -> io::Result<()> {
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    fs::rename(tmp, p)
}

impl Db {
    pub fn open(dir: &Path) -> io::Result<Db> {
        fs::create_dir_all(dir.join("msgs"))?;
        Ok(Db {
            dir: dir.to_path_buf(),
            users: load(&dir.join("users.json"))?,
            sys: load(&dir.join("system.json"))?,
            online: BTreeMap::new(),
        })
    }

    pub fn save(&self) -> io::Result<()> {
        store(&self.dir.join("users.json"), &self.users)?;
        store(&self.dir.join("system.json"), &self.sys)
    }

    /// Find a live user by number or (case-insensitive) name.
    pub fn find_user(&self, s: &str) -> Option<usize> {
        let s = s.trim();
        let n = match s.parse::<usize>() {
            Ok(n) => n,
            Err(_) => self.users.iter().position(|u| u.name.eq_ignore_ascii_case(s))? + 1,
        };
        self.users.get(n.checked_sub(1)?).filter(|u| !u.deleted).map(|_| n)
    }

    fn msg_path(&self, file: &str) -> PathBuf {
        self.dir.join("msgs").join(format!("{file}.json"))
    }

    pub fn msgs(&self, file: &str) -> io::Result<Vec<Msg>> {
        load(&self.msg_path(file))
    }

    pub fn post(&mut self, file: &str, mut m: Msg) -> io::Result<()> {
        self.sys.next_msg_id += 1;
        m.id = self.sys.next_msg_id;
        let mut v = self.msgs(file)?;
        v.push(m);
        store(&self.msg_path(file), &v)?;
        self.save()
    }

    pub fn delete_msg(&mut self, file: &str, id: u64) -> io::Result<()> {
        let mut v = self.msgs(file)?;
        v.retain(|m| m.id != id);
        store(&self.msg_path(file), &v)
    }

    /// Any JSON data file under the data directory (default if missing).
    pub fn get<T: DeserializeOwned + Default>(&self, name: &str) -> io::Result<T> {
        load(&self.dir.join(name))
    }

    pub fn put<T: Serialize + ?Sized>(&self, name: &str, v: &T) -> io::Result<()> {
        store(&self.dir.join(name), v)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Plain text line files (rumors, oneliners, auto-message).
    pub fn lines(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.dir.join(name)).map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
    }

    pub fn put_lines(&self, name: &str, lines: &[String]) -> io::Result<()> {
        let p = self.dir.join(name);
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, lines.join("\n") + "\n")?;
        fs::rename(tmp, p)
    }

    pub fn log(&self, line: &str) {
        use std::io::Write;
        eprintln!("{line}");
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(self.dir.join("sysop.log")) {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub fn hash_pw(pw: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(pw.as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
}

pub fn check_pw(hash: &str, pw: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn passwords() {
        let h = super::hash_pw("hunter2");
        assert!(super::check_pw(&h, "hunter2"));
        assert!(!super::check_pw(&h, "HUNTER2"));
        assert!(!super::check_pw("", ""));
    }
}

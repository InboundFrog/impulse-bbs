//! Persistent data: users (USER.LST), system state (STATUS.DAT counters,
//! LASTON.DAT) and message bases. Stored as JSON under `data_path`, or in
//! PostgreSQL when `IMPULSE_DATABASE_URL` is set.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng};
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
    /// Added to the end of posts and letters (MAIL1.PAS).
    pub autosig: String,
    /// Chosen menu prompt (choosaprompt): 1-4 from `prompts`, 5 the menu's own, 0 not chosen yet.
    pub prompt: u8,
    pub sl: u8,
    pub dsl: u8,
    /// AR flags, letters A-Z.
    pub ar: String,
    /// Restriction flags by their Impulse letters (L C V B A * P E K M).
    pub ac: String,
    pub ansi: bool,
    pub pause: bool,
    pub hotkeys: bool,
    /// Full-screen editor rather than the line editor.
    pub fse: bool,
    pub novice: bool,
    pub page_len: u16,
    pub colors: [u8; 10],
    pub first_on: String,
    pub last_on: String,
    pub logons: u32,
    pub on_today: u32,
    /// Wrong passwords since the last good call (illegal).
    pub illegal: u32,
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
    /// Display file shown to a locked-out user.
    pub locked_file: String,
    pub deleted: bool,
    pub last_board: usize,
    /// Highest message id read, per board file.
    pub last_read: BTreeMap<String, u64>,
    /// Board files left out of newscans (mzscan).
    pub no_scan: Vec<String>,
    pub last_fbase: usize,
    /// File base files left out of new file scans (fzscan).
    pub no_fscan: Vec<String>,
    /// File list columns (fflag): N name, S size, P points, D description,
    /// E extended description, W uploader, T upload date, C download count.
    pub list_fields: String,
    /// Keystrokes go to TRAP/TRAP.MSG, or TRAPn.MSG when separate (trapactivity).
    pub trap_activity: bool,
    pub trap_separate: bool,
    /// SysOp chats are recorded to CHAT.MSG, or CHATn.MSG (chatauto).
    pub chat_auto: bool,
    pub chat_separate: bool,
    /// This user's log lines also go to TRAP/SLOGn.node (slogseperate).
    pub slog_separate: bool,
    /// Mail for this user goes to this user number instead (forusr; 0 = none).
    pub forward_to: usize,
    /// Mailbox closed: nobody but Co-SysOps can write (nomail).
    pub mail_closed: bool,
    /// Board files this user is message SysOp of (boardsysop).
    pub board_sysop: Vec<String>,
    /// Beep the console when this user logs on (alert).
    pub alert: bool,
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
            autosig: String::new(),
            prompt: 0,
            sl: 0,
            dsl: 0,
            ar: String::new(),
            ac: String::new(),
            ansi: true,
            pause: true,
            hotkeys: true,
            fse: false,
            novice: true,
            page_len: 24,
            colors: DEFAULT_COLORS,
            first_on: String::new(),
            last_on: String::new(),
            logons: 0,
            on_today: 0,
            illegal: 0,
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
            locked_file: String::new(),
            deleted: false,
            last_board: 0,
            last_read: BTreeMap::new(),
            no_scan: Vec::new(),
            last_fbase: 0,
            no_fscan: Vec::new(),
            list_fields: "NSPDE".into(),
            trap_activity: false,
            trap_separate: false,
            chat_auto: false,
            chat_separate: false,
            slog_separate: false,
            forward_to: 0,
            mail_closed: false,
            board_sysop: Vec::new(),
            alert: false,
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

/// One day of system usage (zlogrec in ZLOG.DAT).
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Zlog {
    pub date: String,
    pub active: i64,
    pub calls: u32,
    pub new_users: u32,
    pub pub_posts: u32,
    pub priv_posts: u32,
    pub feedback: u32,
    pub uploads: u32,
    pub downloads: u32,
    pub uk: u64,
    pub dk: u64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SysState {
    pub callers: u64,
    pub next_msg_id: u64,
    pub last_callers: Vec<LastCaller>,
    pub today: Zlog,
    /// Previous days, newest first.
    pub history: Vec<Zlog>,
}

impl SysState {
    /// Today's statistics, starting a new day when the date has changed.
    pub fn zlog(&mut self) -> &mut Zlog {
        let d = chrono::Local::now().format("%Y-%m-%d").to_string();
        if self.today.date != d {
            if !self.today.date.is_empty() {
                self.history.insert(0, std::mem::take(&mut self.today));
                self.history.truncate(90);
            }
            self.today = Zlog { date: d, ..Zlog::default() };
        }
        &mut self.today
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
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
    /// 0 no, 1 anonymous, 2 anonymous SysOp, 3 "Abby", 4 "Problemed Person",
    /// 5 posted under `alias` (fromi.anon).
    pub anon: u8,
    pub alias: String,
    /// Id of the message this answers.
    pub reply_to: u64,
    /// Kept by max-message pruning and can't be deleted (mipermanent).
    pub permanent: bool,
    /// Hidden from users until a message SysOp validates it (miunvalidated).
    pub unvalidated: bool,
}

/// A file in a file base (ulfrec + verbrec).
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct FileRec {
    pub name: String,
    pub desc: String,
    /// Extended description lines.
    pub ext: Vec<String>,
    pub points: i32,
    pub downloads: u32,
    pub size: u64,
    pub owner: String,
    pub owner_num: usize,
    /// YYYY-MM-DD
    pub date: String,
    pub unvalidated: bool,
}

#[derive(Default)]
pub struct Online {
    pub name: String,
    pub doing: String,
    pub unum: usize,
    /// Set when the SysOp edits this user; the session reloads the record.
    pub reload: bool,
    /// Node messages and chat lines waiting to be shown.
    pub inbox: Vec<String>,
    /// Multinode chat forum this node is in, 0 = none.
    pub forum: u8,
    /// Forum the console wants this node to join (a SysOp chat), 0 = none.
    pub pull: u8,
    /// The caller's connection, so the console can hang them up.
    pub sock: Option<crate::serial::Link>,
    pub tap: std::sync::Arc<std::sync::Mutex<crate::term::Tap>>,
}

pub struct Db {
    dir: PathBuf,
    /// Set: documents, line files and the SysOp log live in PostgreSQL;
    /// uploads, traps and script files stay under `dir`.
    pg: Option<Pg>,
    /// User number N is `users[N - 1]`, as in USER.LST.
    pub users: Vec<User>,
    pub sys: SysState,
    pub online: BTreeMap<usize, Online>,
    /// Days of old SysOp logs kept as sysop1.log, sysop2.log... (backsysoplogs; 0 = one log).
    pub back_logs: u32,
}

/// A PostgreSQL connection, reopened if the server drops it.
pub struct Pg {
    url: String,
    client: std::sync::Mutex<postgres::Client>,
}

const SCHEMA: &str = "create table if not exists kv (name text primary key, body text not null);
create table if not exists log (id bigserial primary key, at timestamptz not null default now(), line text not null);
create index if not exists log_at on log (at);";

impl Pg {
    pub fn connect(url: &str) -> io::Result<Pg> {
        let mut c = postgres::Client::connect(url, postgres::NoTls).map_err(io::Error::other)?;
        c.batch_execute(SCHEMA).map_err(io::Error::other)?;
        Ok(Pg { url: url.into(), client: c.into() })
    }

    // ponytail: no TLS, add postgres-native-tls when the server isn't local
    fn run<T>(&self, f: impl Fn(&mut postgres::Client) -> Result<T, postgres::Error>) -> io::Result<T> {
        let mut c = self.client.lock().unwrap_or_else(|e| e.into_inner());
        if c.is_closed() {
            *c = postgres::Client::connect(&self.url, postgres::NoTls).map_err(io::Error::other)?;
        }
        f(&mut c).map_err(io::Error::other)
    }

    pub fn read(&self, name: &str) -> io::Result<Option<String>> {
        self.run(|c| c.query_opt("select body from kv where name = $1", &[&name]).map(|r| r.map(|r| r.get(0))))
    }

    pub fn write(&self, name: &str, body: &str) -> io::Result<()> {
        self.run(|c| c.execute("insert into kv (name, body) values ($1, $2) on conflict (name) do update set body = excluded.body", &[&name, &body])).map(drop)
    }
}

/// Open the configuration and data. With `IMPULSE_DATABASE_URL` set, both
/// come from PostgreSQL; the first run copies the config file in.
pub fn open(cfg_path: &str) -> Result<(crate::config::Config, Db), String> {
    let Ok(url) = std::env::var("IMPULSE_DATABASE_URL") else {
        let cfg = crate::config::load(cfg_path)?;
        let db = Db::open(&cfg.data_path).map_err(|e| format!("Unable to open data in {}: {e}", cfg.data_path.display()))?;
        return Ok((cfg, db));
    };
    let pg = Pg::connect(&url).map_err(|e| format!("Unable to connect to PostgreSQL: {e}"))?;
    let cfg = match pg.read(CONFIG).map_err(|e| e.to_string())? {
        Some(s) => toml::from_str(&s).map_err(|e| format!("{CONFIG} in PostgreSQL: {e}"))?,
        None => {
            let c = crate::config::load(cfg_path)?;
            pg.write(CONFIG, &toml::to_string_pretty(&c).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            c
        }
    };
    let mut db = Db { dir: cfg.data_path.clone(), pg: Some(pg), users: vec![], sys: Default::default(), online: BTreeMap::new(), back_logs: 0 };
    fs::create_dir_all(&db.dir).map_err(|e| e.to_string())?;
    db.users = db.load(USERS).map_err(|e| e.to_string())?;
    db.sys = db.load(SYSTEM).map_err(|e| e.to_string())?;
    Ok((cfg, db))
}

const CONFIG: &str = "impulse.toml";
const USERS: &str = "users.json";
const SYSTEM: &str = "system.json";

impl Db {
    pub fn open(dir: &Path) -> io::Result<Db> {
        fs::create_dir_all(dir.join("msgs"))?;
        fs::create_dir_all(dir.join("files"))?;
        let mut db = Db { dir: dir.to_path_buf(), pg: None, users: vec![], sys: Default::default(), online: BTreeMap::new(), back_logs: 0 };
        db.users = db.load(USERS)?;
        db.sys = db.load(SYSTEM)?;
        Ok(db)
    }

    /// A document or line file under `name`, None if missing.
    fn read(&self, name: &str) -> io::Result<Option<String>> {
        if let Some(pg) = &self.pg {
            return pg.read(name);
        }
        match fs::read_to_string(self.dir.join(name)) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Files go via a temp file + rename so a crash never leaves a half-written one.
    fn write(&self, name: &str, body: &str) -> io::Result<()> {
        if let Some(pg) = &self.pg {
            return pg.write(name, body);
        }
        let p = self.dir.join(name);
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, body)?;
        fs::rename(tmp, p)
    }

    fn load<T: DeserializeOwned + Default>(&self, name: &str) -> io::Result<T> {
        Ok(self.read(name)?.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or_default())
    }

    fn store<T: Serialize + ?Sized>(&self, name: &str, v: &T) -> io::Result<()> {
        self.write(name, &serde_json::to_string_pretty(v)?)
    }

    pub fn remove(&self, name: &str) -> io::Result<()> {
        match &self.pg {
            Some(pg) => pg.run(|c| c.execute("delete from kv where name = $1", &[&name])).map(drop),
            None => fs::remove_file(self.dir.join(name)).or_else(|e| if e.kind() == io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
        }
    }

    /// Write the configuration back where it came from (comments in a
    /// hand-edited file are not kept).
    pub fn save_config(&self, c: &crate::config::Config, path: &str) -> io::Result<()> {
        match &self.pg {
            Some(pg) => pg.write(CONFIG, &toml::to_string_pretty(c).map_err(io::Error::other)?),
            None => c.save(path),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        self.store(USERS, &self.users)?;
        self.store(SYSTEM, &self.sys)
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

    fn msg_name(file: &str) -> String {
        format!("msgs/{file}.json")
    }

    pub fn msgs(&self, file: &str) -> io::Result<Vec<Msg>> {
        self.load(&Self::msg_name(file))
    }

    /// Add a message, then drop the oldest non-permanent ones past `max` (0 = no limit).
    pub fn post(&mut self, file: &str, mut m: Msg, max: usize) -> io::Result<()> {
        self.sys.next_msg_id += 1;
        m.id = self.sys.next_msg_id;
        let mut v = self.msgs(file)?;
        v.push(m);
        self.store(&Self::msg_name(file), &v)?;
        self.prune(file, max)?;
        self.save()
    }

    pub fn put_msgs(&self, file: &str, v: &[Msg]) -> io::Result<()> {
        self.store(&Self::msg_name(file), &v)
    }

    /// Drop the oldest non-permanent messages beyond `max`; returns how many.
    pub fn prune(&mut self, file: &str, max: usize) -> io::Result<usize> {
        let mut v = self.msgs(file)?;
        let n = v.len();
        while max > 0 && v.len() > max {
            match v.iter().position(|m| !m.permanent) {
                Some(i) => v.remove(i),
                None => break,
            };
        }
        if v.len() != n {
            self.store(&Self::msg_name(file), &v)?;
        }
        Ok(n - v.len())
    }

    pub fn update_msg(&mut self, file: &str, id: u64, f: impl FnOnce(&mut Msg)) -> io::Result<()> {
        let mut v = self.msgs(file)?;
        if let Some(m) = v.iter_mut().find(|m| m.id == id) {
            f(m);
        }
        self.store(&Self::msg_name(file), &v)
    }

    pub fn delete_msg(&mut self, file: &str, id: u64) -> io::Result<()> {
        let mut v = self.msgs(file)?;
        v.retain(|m| m.id != id);
        self.store(&Self::msg_name(file), &v)
    }

    pub fn files(&self, base: &str) -> io::Result<Vec<FileRec>> {
        self.load(&format!("files/{base}.json"))
    }

    pub fn put_files(&self, base: &str, v: &[FileRec]) -> io::Result<()> {
        self.store(&format!("files/{base}.json"), v)
    }

    /// Any JSON data file under the data directory (default if missing).
    pub fn get<T: DeserializeOwned + Default>(&self, name: &str) -> io::Result<T> {
        self.load(name)
    }

    pub fn put<T: Serialize + ?Sized>(&self, name: &str, v: &T) -> io::Result<()> {
        self.store(name, v)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Plain text line files (rumors, oneliners, auto-message).
    pub fn lines(&self, name: &str) -> Vec<String> {
        self.read(name).ok().flatten().map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
    }

    pub fn put_lines(&self, name: &str, lines: &[String]) -> io::Result<()> {
        self.write(name, &(lines.join("\n") + "\n"))
    }

    pub fn log(&self, line: &str) {
        use std::io::Write;
        eprintln!("{line}");
        if let Some(pg) = &self.pg {
            // ponytail: old days are deleted, not kept as totals; the zlog history has those
            let keep = self.back_logs as i32;
            let _ = pg.run(|c| {
                c.execute("insert into log (line) values ($1)", &[&line])?;
                if keep > 0 {
                    c.execute("delete from log where at < $1", &[&(today_start() - chrono::Duration::days(keep as i64))])?;
                }
                Ok(())
            });
            return;
        }
        let p = self.dir.join("sysop.log");
        if self.back_logs > 0 {
            self.rotate_log(&p);
        }
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{line}");
        }
    }

    /// The current SysOp log: today's when logs rotate, else all of it.
    pub fn log_text(&self) -> String {
        let Some(pg) = &self.pg else { return fs::read_to_string(self.dir.join("sysop.log")).unwrap_or_default() };
        let from = (self.back_logs > 0).then(today_start);
        pg.run(|c| c.query("select line from log where $1::timestamptz is null or at >= $1 order by id", &[&from]))
            .map(|rows| rows.iter().map(|r| r.get::<_, String>(0) + "\n").collect())
            .unwrap_or_default()
    }

    /// The first line of a new day starts a new SysOp log: the last one gets
    /// that day's totals and becomes sysop1.log, older ones move up a number.
    fn rotate_log(&self, p: &Path) {
        use std::io::Write;
        let Ok(t) = fs::metadata(p).and_then(|m| m.modified()) else { return };
        let day = chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if day == today {
            return;
        }
        if let (Some(z), Ok(mut f)) =
            (std::iter::once(&self.sys.today).chain(&self.sys.history).find(|z| z.date == day), fs::OpenOptions::new().append(true).open(p))
        {
            let _ = write!(
                f,
                "\nTotal time on........: {}\nNumber of calls......: {}\nNew users............: {}\nPublic posts.........: {}\n\
                 Private mail sent....: {}\nFeedback sent........: {}\nDownloads today......: {}-{}k\nUploads today........: {}-{}k\n",
                z.active, z.calls, z.new_users, z.pub_posts, z.priv_posts, z.feedback, z.downloads, z.dk, z.uploads, z.uk
            );
        }
        let old = |i: u32| self.dir.join(format!("sysop{i}.log"));
        let _ = fs::remove_file(old(self.back_logs));
        for i in (1..self.back_logs).rev() {
            let _ = fs::rename(old(i), old(i + 1));
        }
        let _ = fs::rename(p, old(1));
        let _ = fs::write(p, format!("\nImpulse SysOp log for {today}\n\n"));
    }
}

fn today_start() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now().date_naive().and_hms_opt(0, 0, 0).and_then(|t| t.and_local_timezone(chrono::Local).earliest()).unwrap_or_else(chrono::Local::now)
}

pub fn hash_pw(pw: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(pw.as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
}

/// Logon check; also tries the password in capitals, as Impulse kept them.
pub fn check_login(hash: &str, pw: &str) -> bool {
    check_pw(hash, pw) || (pw.to_uppercase() != pw && check_pw(hash, &pw.to_uppercase()))
}

pub fn check_pw(hash: &str, pw: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_rotation() {
        let dir = std::env::temp_dir().join(format!("impulse-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = super::Db::open(&dir).unwrap();
        db.back_logs = 2;
        let yesterday = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
        let day = chrono::DateTime::<chrono::Local>::from(yesterday).format("%Y-%m-%d").to_string();
        db.sys.today.date = day;
        db.sys.today.calls = 7;
        for (n, old) in [(2, "older"), (1, "old")] {
            std::fs::write(dir.join(format!("sysop{n}.log")), old).unwrap();
        }
        std::fs::write(dir.join("sysop.log"), "yday\n").unwrap();
        std::fs::File::options().write(true).open(dir.join("sysop.log")).unwrap().set_modified(yesterday).unwrap();
        db.log("today");
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        assert!(read("sysop1.log").starts_with("yday\n") && read("sysop1.log").contains("Number of calls......: 7"));
        assert_eq!(read("sysop2.log"), "old");
        assert!(read("sysop.log").ends_with("today\n"));
        db.log("again"); // same day: no rotation
        assert_eq!(read("sysop2.log"), "old");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs against a scratch database: IMPULSE_TEST_DATABASE_URL=postgres://... cargo test
    #[test]
    fn postgres_store() {
        let Ok(url) = std::env::var("IMPULSE_TEST_DATABASE_URL") else { return };
        let pg = super::Pg::connect(&url).unwrap();
        pg.run(|c| c.batch_execute("delete from kv where name like 'test/%'; delete from log where line like 'test %'")).unwrap();
        let mut db = super::Db { dir: std::env::temp_dir(), pg: Some(pg), users: vec![], sys: Default::default(), online: Default::default(), back_logs: 1 };
        assert!(db.lines("test/rumors").is_empty());
        db.put_lines("test/rumors", &["a".into(), "b".into()]).unwrap();
        assert_eq!(db.lines("test/rumors"), ["a", "b"]);
        db.put("test/x.json", &vec![1, 2]).unwrap();
        db.put("test/x.json", &vec![3]).unwrap();
        assert_eq!(db.get::<Vec<i32>>("test/x.json").unwrap(), [3]);
        db.remove("test/x.json").unwrap();
        assert!(db.get::<Vec<i32>>("test/x.json").unwrap().is_empty());
        db.log("test one");
        db.log("test two");
        assert!(db.log_text().ends_with("test one\ntest two\n"));
        db.back_logs = 0;
        assert!(db.log_text().contains("test one\n"));
    }

    #[test]
    fn passwords() {
        let h = super::hash_pw("hunter2");
        assert!(super::check_pw(&h, "hunter2"));
        assert!(!super::check_pw(&h, "HUNTER2"));
        assert!(!super::check_pw("", ""));
    }
}

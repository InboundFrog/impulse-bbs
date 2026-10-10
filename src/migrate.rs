//! `impulse migrate to-pg|to-files [config]`: move the config, data and
//! SysOp log between `data_path` and PostgreSQL (`IMPULSE_DATABASE_URL`).

use crate::db::{CONFIG, Pg};
use chrono::{DateTime, Local, NaiveDateTime};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Line files kept in the store besides the `*.json` documents.
const LINE_FILES: [&str; 3] = ["auto.msg", "oneliners.txt", "rumors.txt"];

/// A store document under `data_path`: top-level `*.json` and line files,
/// and `msgs/*.json`, `files/*.json` (not the upload directories under files/).
fn is_doc(rel: &str) -> bool {
    match rel.split('/').collect::<Vec<_>>()[..] {
        [f] => f.ends_with(".json") || LINE_FILES.contains(&f),
        ["msgs" | "files", f] => f.ends_with(".json"),
        _ => false,
    }
}

/// `sysop.log` is 0, `sysopN.log` is N.
fn log_no(f: &str) -> Option<u32> {
    let n = f.strip_prefix("sysop")?.strip_suffix(".log")?;
    if n.is_empty() { Some(0) } else { n.parse().ok() }
}

pub fn run(dir: &str, cfg_path: &str) -> Result<String, String> {
    let url = std::env::var("IMPULSE_DATABASE_URL").map_err(|_| "Set IMPULSE_DATABASE_URL to the PostgreSQL database first.".to_string())?;
    let pg = Pg::connect(&url).map_err(|e| format!("Unable to connect to PostgreSQL: {e}"))?;
    match dir {
        "to-pg" => to_pg(&pg, cfg_path),
        "to-files" => to_files(&pg, cfg_path),
        _ => Err("usage: impulse migrate to-pg|to-files [config]".into()),
    }
}

fn to_pg(pg: &Pg, cfg_path: &str) -> Result<String, String> {
    let e = |e: std::io::Error| e.to_string();
    if pg.run(|c| c.query_one("select (select count(*) from kv) + (select count(*) from log)", &[])).map_err(e)?.get::<_, i64>(0) > 0 {
        return Err("PostgreSQL already has data; migrate it to-files first, or use an empty database.".into());
    }
    let text = fs::read_to_string(cfg_path).map_err(|e| format!("{cfg_path}: {e}"))?;
    let cfg: crate::config::Config = toml::from_str(&text).map_err(|e| format!("{cfg_path}: {e}"))?;
    let data = &cfg.data_path;
    let mut docs = Vec::new();
    let mut logs = Vec::new();
    for sub in ["", "msgs", "files"] {
        let Ok(rd) = fs::read_dir(data.join(sub)) else { continue };
        for f in rd.flatten().filter(|f| f.path().is_file()) {
            let name = f.file_name().to_string_lossy().to_string();
            let rel = if sub.is_empty() { name.clone() } else { format!("{sub}/{name}") };
            if is_doc(&rel) {
                docs.push(rel);
            } else if let Some(n) = log_no(&name).filter(|_| sub.is_empty()) {
                logs.push((n, rel));
            }
        }
    }
    docs.sort();
    logs.sort_by_key(|l| std::cmp::Reverse(l.0)); // oldest first
    // ponytail: non-UTF-8 bytes become U+FFFD; the BBS can't read such files anyway
    let read = |rel: &str| fs::read(data.join(rel)).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("{rel}: {e}"));
    let bodies = docs.iter().map(|d| read(d)).collect::<Result<Vec<_>, _>>()?;
    let mut lines: Vec<(DateTime<Local>, String)> = Vec::new();
    for (_, rel) in &logs {
        // lines without a "[yyyy-mm-dd hh:mm]" stamp take the last one seen, or the file's date
        let mut at: DateTime<Local> = fs::metadata(data.join(rel)).and_then(|m| m.modified()).map(Into::into).unwrap_or_else(|_| Local::now());
        for l in read(rel)?.lines() {
            if let Some(t) =
                l.get(1..17).and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").ok()).and_then(|t| t.and_local_timezone(Local).earliest())
            {
                at = t;
            }
            lines.push((at, l.to_string()));
        }
    }
    // archive first: if the insert fails the files are still in place
    let mut moved: Vec<PathBuf> = docs.iter().chain(logs.iter().map(|(_, r)| r)).map(|r| data.join(r)).collect();
    moved.insert(0, cfg_path.into());
    let tar = archive(&moved)?;
    pg.run(|c| {
        let mut t = c.transaction()?;
        t.execute("insert into kv (name, body) values ($1, $2)", &[&CONFIG, &text])?;
        for (d, b) in docs.iter().zip(&bodies) {
            t.execute("insert into kv (name, body) values ($1, $2)", &[d, b])?;
        }
        for (at, l) in &lines {
            t.execute("insert into log (at, line) values ($1, $2)", &[at, l])?;
        }
        t.commit()
    })
    .map_err(e)?;
    for p in &moved {
        fs::remove_file(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(format!(
        "Moved {cfg_path}, {} data file(s) and {} log line(s) into PostgreSQL; the files are in {tar} and were removed. \
         Run impulse with IMPULSE_DATABASE_URL set from now on.",
        docs.len(),
        lines.len()
    ))
}

/// tar + gzip the files into archive/yyyymmdd-data.tar.gz (-2, -3... if taken),
/// keeping their paths relative to the current directory.
fn archive(files: &[PathBuf]) -> Result<String, String> {
    fs::create_dir_all("archive").map_err(|e| format!("archive: {e}"))?;
    let day = Local::now().format("%Y%m%d");
    let out = (1..)
        .map(|n| if n == 1 { format!("archive/{day}-data.tar.gz") } else { format!("archive/{day}-data-{n}.tar.gz") })
        .find(|p| !Path::new(p).exists())
        .unwrap();
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut tar = Command::new("tar");
    tar.arg("-czf").arg(&out);
    for f in files {
        // files outside the current directory keep their absolute path, minus the leading /
        let abs = fs::canonicalize(f).map_err(|e| format!("{}: {e}", f.display()))?;
        match abs.strip_prefix(&cwd) {
            Ok(rel) => tar.arg("-C").arg(&cwd).arg(rel),
            Err(_) => tar.arg("-C").arg("/").arg(abs.strip_prefix("/").unwrap_or(&abs)),
        };
    }
    match tar.status() {
        Ok(s) if s.success() => Ok(out),
        r => Err(format!("tar failed ({r:?}); nothing was moved.")),
    }
}

fn to_files(pg: &Pg, cfg_path: &str) -> Result<String, String> {
    let e = |e: std::io::Error| e.to_string();
    let text = pg.read(CONFIG).map_err(e)?.ok_or("PostgreSQL has no configuration; nothing to migrate.")?;
    let cfg: crate::config::Config = toml::from_str(&text).map_err(|e| format!("{CONFIG} in PostgreSQL: {e}"))?;
    let data = &cfg.data_path;
    let rows: Vec<(String, String)> =
        pg.run(|c| c.query("select name, body from kv where name <> $1 order by name", &[&CONFIG])).map_err(e)?.iter().map(|r| (r.get(0), r.get(1))).collect();
    let (docs, odd): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(n, _)| is_doc(n));
    let log: String = pg.run(|c| c.query("select line from log order by id", &[])).map_err(e)?.iter().map(|r| r.get::<_, String>(0) + "\n").collect();
    // nothing is overwritten: check every target before writing any
    let targets: Vec<PathBuf> =
        std::iter::once(PathBuf::from(cfg_path)).chain(docs.iter().map(|(n, _)| data.join(n))).chain(std::iter::once(data.join("sysop.log"))).collect();
    if let Some(p) = targets.iter().find(|p| p.exists()) {
        return Err(format!("{} already exists; move it away first.", p.display()));
    }
    fs::create_dir_all(data.join("msgs")).and_then(|_| fs::create_dir_all(data.join("files"))).map_err(e)?;
    fs::write(cfg_path, &text).map_err(|e| format!("{cfg_path}: {e}"))?;
    for (n, b) in &docs {
        fs::write(data.join(n), b).map_err(|e| format!("{n}: {e}"))?;
    }
    if !log.is_empty() {
        fs::write(data.join("sysop.log"), &log).map_err(e)?;
    }
    let names: Vec<&String> = docs.iter().map(|(n, _)| n).collect();
    pg.run(|c| {
        let mut t = c.transaction()?;
        t.execute("delete from kv where name = $1 or name = any($2)", &[&CONFIG, &names])?;
        t.execute("delete from log", &[])?;
        t.commit()
    })
    .map_err(e)?;
    let kept = if odd.is_empty() {
        String::new()
    } else {
        format!(" Left in PostgreSQL (not BBS data): {}.", odd.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", "))
    };
    Ok(format!(
        "Wrote {cfg_path}, {} data file(s) and {} log line(s) under {}, and removed them from PostgreSQL.{kept} Run impulse without IMPULSE_DATABASE_URL from now on.",
        docs.len(),
        log.lines().count(),
        data.display()
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn which_files_move() {
        use super::{is_doc, log_no};
        assert!(is_doc("users.json") && is_doc("rumors.txt") && is_doc("msgs/general.json") && is_doc("files/uploads.json"));
        assert!(!is_doc("files/uploads/GAME.ZIP") && !is_doc("trap/trap.msg") && !is_doc("SURVEY.ASW") && !is_doc("users.tmp") && !is_doc("../x.json"));
        assert_eq!((log_no("sysop.log"), log_no("sysop3.log"), log_no("sysopx.log")), (Some(0), Some(3), None));
    }
}

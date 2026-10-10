//! `impulse init` (INSTALL.PAS) and `impulse clean` (MAKECLEA.PAS).

use std::path::Path;
use std::{fs, io};

/// The stock config, menus and display files, built into the binary.
const FILES: &[(&str, &[u8])] = &[
    ("impulse.toml", include_bytes!("../impulse.toml")),
    ("menus/MAIN.MNU", include_bytes!("../menus/MAIN.MNU")),
    ("menus/MSG.MNU", include_bytes!("../menus/MSG.MNU")),
    ("menus/FILE.MNU", include_bytes!("../menus/FILE.MNU")),
    ("menus/SYSOP.MNU", include_bytes!("../menus/SYSOP.MNU")),
    ("text/WELCOME.ANS", include_bytes!("../text/WELCOME.ANS")),
    ("text/LOGON.ANS", include_bytes!("../text/LOGON.ANS")),
    ("text/LOGOFF.ANS", include_bytes!("../text/LOGOFF.ANS")),
];

/// Lay out a new system in `dir`; existing files are left alone.
pub fn init(dir: &Path) -> io::Result<String> {
    let mut made = 0;
    for (name, bytes) in FILES {
        let p = dir.join(name);
        fs::create_dir_all(p.parent().unwrap_or(dir))?;
        if !p.exists() {
            fs::write(&p, bytes)?;
            made += 1;
        }
    }
    Ok(format!("Installed {made} file(s) in {}. cd there and run `impulse`; the first new user is the SysOp.", dir.display()))
}

/// Delete every user but #1 and the new-user voting queue.
pub fn clean(db: &mut crate::db::Db) -> io::Result<String> {
    let n = db.users.len().saturating_sub(1);
    db.users.truncate(1);
    db.save()?;
    db.remove("nuv.json")?;
    Ok(format!("Removed {n} user(s) and the NUV queue; user #1 kept."))
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_then_load() {
        let d = std::env::temp_dir().join(format!("impulse-init-{}", std::process::id()));
        super::init(&d).unwrap();
        assert!(d.join("menus/MAIN.MNU").exists());
        assert!(crate::config::load(d.join("impulse.toml").to_str().unwrap()).is_ok());
        std::fs::remove_dir_all(d).unwrap();
    }
}

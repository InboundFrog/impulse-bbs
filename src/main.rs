//! Impulse BBS, re-implemented in Rust from the Impulse 7.1 Pascal sources.

mod acs;
mod cmds;
mod config;
mod db;
mod doors;
mod edit;
mod events;
mod files;
mod fse;
mod import;
mod menu;
mod misc;
mod msgs;
mod multinode;
mod nuv;
mod qwk;
mod script;
mod session;
mod sysop;
mod term;
mod wfc;

use session::Shared;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, RwLock};
use std::{io::Write, process, thread};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "import") {
        let Some(dir) = args.get(2) else {
            eprintln!("usage: impulse import <impulse directory> [config]");
            process::exit(2);
        };
        let cfg = args.get(3).map_or("impulse.toml", |s| s.as_str());
        match import::run(std::path::Path::new(dir), cfg) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("{e}");
                process::exit(1);
            }
        }
        return;
    }
    let path = args.get(1).cloned().unwrap_or_else(|| "impulse.toml".into());
    let cfg = config::load(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        process::exit(1);
    });
    let db = db::Db::open(&cfg.data_path).unwrap_or_else(|e| {
        eprintln!("Unable to open data in {}: {e}", cfg.data_path.display());
        process::exit(1);
    });
    let listener = TcpListener::bind(&cfg.listen).unwrap_or_else(|e| {
        eprintln!("Unable to listen on {}: {e}", cfg.listen);
        process::exit(1);
    });
    println!("Impulse v{} - {} - listening on {}", env!("CARGO_PKG_VERSION"), cfg.bbs_name, cfg.listen);
    let sh = Arc::new(Shared { cfg: RwLock::new(Arc::new(cfg)), cfg_path: path, db: Mutex::new(db) });
    events::spawn(sh.clone());
    wfc::spawn(sh.clone());
    for stream in listener.incoming().flatten() {
        // Claim the lowest free node number, as MULTINOD.PAS assigns nodes.
        let node = {
            let mut db = sh.db();
            let node = (1..=sh.cfg().max_nodes).find(|n| !db.online.contains_key(n));
            if let Some(n) = node {
                db.online.insert(n, db::Online { name: "Logging in".into(), doing: "Logon".into(), sock: stream.try_clone().ok(), ..Default::default() });
            }
            node
        };
        let Some(node) = node else {
            let mut s = stream;
            let _ = s.write_all(b"All nodes are busy, please call back later.\r\n");
            continue;
        };
        let sh = sh.clone();
        thread::spawn(move || session::run(sh, stream, node));
    }
}

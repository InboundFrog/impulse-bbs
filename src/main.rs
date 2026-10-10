//! Impulse BBS, re-implemented in Rust from the Impulse 7.1 Pascal sources.

mod acs;
mod arcview;
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
mod serial;
mod session;
mod setup;
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
    if let Some(cmd @ ("init" | "clean")) = args.get(1).map(String::as_str) {
        let r = if cmd == "init" {
            setup::init(std::path::Path::new(args.get(2).map_or(".", |s| s.as_str())))
        } else {
            db::open(args.get(2).map_or("impulse.toml", |s| s.as_str())).map_err(std::io::Error::other).and_then(|(_, mut db)| setup::clean(&mut db))
        };
        match r {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("{e}");
                process::exit(1);
            }
        }
        return;
    }
    let path = args.get(1).cloned().unwrap_or_else(|| "impulse.toml".into());
    let (cfg, mut db) = db::open(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        process::exit(1);
    });
    db.back_logs = cfg.back_sysop_logs; // ponytail: read at startup only
    if !cfg.telnet && cfg.serial.is_empty() {
        eprintln!("Telnet is off and no [[serial]] ports are set: nobody can call.");
        process::exit(1);
    }
    let listener = cfg.telnet.then(|| {
        TcpListener::bind(&cfg.listen).unwrap_or_else(|e| {
            eprintln!("Unable to listen on {}: {e}", cfg.listen);
            process::exit(1);
        })
    });
    let mut on: Vec<String> = cfg.serial.iter().map(|p| format!("{} at {}", p.device.display(), p.baud)).collect();
    if cfg.telnet {
        on.insert(0, cfg.listen.clone());
    }
    println!("Impulse v{} - {} - answering {}", env!("CARGO_PKG_VERSION"), cfg.bbs_name, on.join(", "));
    let sh = Arc::new(Shared {
        cfg: RwLock::new(Arc::new(cfg)),
        cfg_path: path,
        db: Mutex::new(db),
        console_forum: Default::default(),
        console_avail: std::io::IsTerminal::is_terminal(&std::io::stdin()).into(),
        offhook: Default::default(),
    });
    events::spawn(sh.clone());
    wfc::spawn(sh.clone());
    for p in sh.cfg().serial.clone() {
        serial::spawn(sh.clone(), p);
    }
    let Some(listener) = listener else {
        loop {
            thread::park(); // serial threads answer; the console's Q exits
        }
    };
    for stream in listener.incoming().flatten() {
        let link = serial::Link::Tcp(stream);
        let Some(node) = claim_node(&sh, &link) else {
            let mut s = link;
            let _ = s.write_all(b"All nodes are busy, please call back later.\r\n");
            continue;
        };
        let sh = sh.clone();
        let peer = match &link {
            serial::Link::Tcp(s) => s.peer_addr().map(|a| a.to_string()).unwrap_or_default(),
            _ => String::new(),
        };
        thread::spawn(move || session::run(sh, link, peer, node));
    }
}

/// Claim the lowest free node number, as MULTINOD.PAS assigns nodes.
fn claim_node(sh: &Shared, link: &serial::Link) -> Option<usize> {
    let mut db = sh.db();
    let node = (1..=sh.cfg().max_nodes).find(|n| !db.online.contains_key(n))?;
    db.online.insert(node, db::Online { name: "Logging in".into(), doing: "Logon".into(), sock: link.try_clone().ok(), ..Default::default() });
    Some(node)
}

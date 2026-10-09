//! The waiting-for-caller console (WFCMENU.PAS), as commands typed on the
//! server's stdin. The SysOp editors themselves run in a session (`*`, `%`
//! commands), so log on over telnet for those.

use crate::session::Shared;
use std::io::BufRead;
use std::sync::Arc;

const HELP: &str = "  W           who's online
  S           today's statistics
  M node text send a node message (node 0 = everyone)
  K node      hang up a node
  L [n]       last n lines of the SysOp log (default 20)
  Q           shut down";

pub fn spawn(sh: Arc<Shared>) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match run(&sh, line.trim()) {
                Some(out) => println!("{out}"),
                None => {
                    // let the hung-up sessions save their users
                    for _ in 0..30 {
                        if sh.db().online.is_empty() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    sh.db().log("Shut down from the console");
                    std::process::exit(0);
                }
            }
        }
    });
}

/// One console command; None means shut down.
pub fn run(sh: &Shared, line: &str) -> Option<String> {
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    let (node, text) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let node: usize = node.parse().unwrap_or(usize::MAX);
    let mut db = sh.db();
    Some(match cmd.to_ascii_uppercase().as_str() {
        "" => String::new(),
        "W" if db.online.is_empty() => "No one is online.".into(),
        "W" => db.online.iter().map(|(n, o)| format!("  {n:>3}  {:<30} {}", o.name, o.doing)).collect::<Vec<_>>().join("\n"),
        "S" => {
            let callers = db.sys.callers;
            let z = db.sys.zlog();
            format!(
                "  {}: {} calls ({callers} total), {} new users, {} posts, {} e-mail, {} feedback\n  {} uploads ({}k), {} downloads ({}k), {} min active",
                z.date, z.calls, z.new_users, z.pub_posts, z.priv_posts, z.feedback, z.uploads, z.uk, z.downloads, z.dk, z.active
            )
        }
        "M" if !text.is_empty() => {
            let msg = format!("|NL|U5Message from the SysOp: |U3{text}");
            let mut sent = 0;
            for (_, o) in db.online.iter_mut().filter(|(n, _)| node == 0 || **n == node) {
                o.inbox.push(msg.clone());
                sent += 1;
            }
            format!("Sent to {sent} node(s).")
        }
        "K" => match db.online.get(&node).and_then(|o| o.sock.as_ref()) {
            Some(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
                format!("Node {node} hung up.")
            }
            None => format!("Node {node} isn't online."),
        },
        "L" => {
            let n = if node == usize::MAX { 20 } else { node };
            let text = std::fs::read_to_string(db.path("sysop.log")).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(n)..].join("\n")
        }
        "Q" => {
            for o in db.online.values() {
                if let Some(s) = &o.sock {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
            }
            return None;
        }
        _ => HELP.into(),
    })
}

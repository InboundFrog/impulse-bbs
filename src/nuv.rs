//! New user voting (NUV.PAS): validated users vote new users in or out.

use crate::session::Session;
use serde::{Deserialize, Serialize};
use std::io;

const FILE: &str = "nuv.json";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Nuv {
    pub unum: usize,
    pub handle: String,
    pub yes: u32,
    pub no: u32,
    pub voted_by: Vec<usize>,
    /// (who, comment)
    pub comments: Vec<(String, String)>,
}

impl Session {
    /// Put a new user up for voting (newuser.pas, usenuv).
    pub fn nuv_add(&mut self) -> io::Result<()> {
        self.printf("nuvalert")?;
        let db = self.sh.db();
        let mut v: Vec<Nuv> = db.get(FILE)?;
        v.push(Nuv { unum: self.unum, handle: self.user.name.clone(), ..Nuv::default() });
        db.put(FILE, &v)
    }

    /// Any users this caller hasn't voted on yet (logon check).
    pub fn nuv_pending(&self) -> bool {
        self.user.sl > self.cfg().new_sl && self.sh.db().get::<Vec<Nuv>>(FILE).unwrap_or_default().iter().any(|n| !n.voted_by.contains(&self.unum) && n.unum != self.unum)
    }

    /// `-N` (newUserVote).
    pub fn new_user_vote(&mut self) -> io::Result<()> {
        if self.user.sl <= self.cfg().new_sl || self.user.ac.contains('K') {
            return Ok(());
        }
        let list: Vec<Nuv> = self.sh.db().get(FILE)?;
        if list.is_empty() {
            self.write_hdr("New User Voting")?;
            return self.sprint("|U1No users to vote on.");
        }
        let fso = self.is_sysop();
        for n in list {
            if n.voted_by.contains(&self.unum) || n.unum == self.unum {
                continue;
            }
            let cfg = self.cfg();
            let vars = [
                ("NM", n.handle.clone()),
                ("YV", n.yes.to_string()),
                ("NV", n.no.to_string()),
                ("VY", cfg.nuv_yes.to_string()),
                ("VN", cfg.nuv_no.to_string()),
            ];
            self.cls()?;
            self.show_answers("NEWUSER", n.unum)?;
            self.tmpl("nuvote", &vars, "|NL|U1Voting on |U3%NM|U1: |U3%YV|U1 yes (|U3%VY|U1 validates), |U3%NV|U1 no (|U3%VN|U1 deletes)")?;
            for (who, c) in &n.comments {
                self.sprompt("|U1")?;
                self.out(who)?;
                self.sprompt("|U9: |U2")?;
                self.out(c)?;
                self.nl()?;
            }
            let keys = if fso { "YNAVDQ" } else { "YNAQ" };
            let k = self.sub_prompt("NUVP", &cfg.strings.nuv_prompt, keys)?;
            let vote = match k {
                'Y' | 'N' => Some(k == 'Y'),
                'V' | 'D' if fso => None,
                'Q' => break,
                _ => continue,
            };
            let comment = match vote {
                Some(_) if self.pynq("|NL|U1Would you like to add a comment? ", false)? => {
                    self.sprint("|U1Enter your comment on this user")?;
                    self.sprompt("|U2> |U3")?;
                    Some(self.input(77)?).filter(|c| !c.is_empty())
                }
                _ => None,
            };
            // Re-read: someone on another node may have voted meanwhile.
            let outcome = {
                let db = self.sh.db();
                let mut all: Vec<Nuv> = db.get(FILE)?;
                let Some(e) = all.iter_mut().find(|e| e.unum == n.unum) else { continue };
                match vote {
                    Some(true) => e.yes += 1,
                    Some(false) => e.no += 1,
                    None => {}
                }
                if vote.is_some() {
                    e.voted_by.push(self.unum);
                }
                if let Some(c) = comment {
                    e.comments.push((self.user.name.clone(), c));
                }
                let outcome = match (vote, k) {
                    (None, 'V') => Some(true),
                    (None, _) => Some(false),
                    _ if e.yes >= cfg.nuv_yes => Some(true),
                    _ if e.no >= cfg.nuv_no => Some(false),
                    _ => None,
                };
                if outcome.is_some() {
                    all.retain(|e| e.unum != n.unum);
                }
                db.put(FILE, &all)?;
                outcome
            };
            match outcome {
                Some(true) => {
                    let u = self.sh.db().users.get(n.unum - 1).cloned();
                    if let Some(mut u) = u {
                        self.auto_validate(&mut u);
                        self.store_user(n.unum, u)?;
                    }
                    self.log(&format!("NUV: validated {}", n.handle));
                    self.sprint(&cfg.strings.nuv_validated)?;
                }
                Some(false) => {
                    self.nuke_user(n.unum)?;
                    self.log(&format!("NUV: deleted {}", n.handle));
                    self.sprint(&cfg.strings.nuv_deleted)?;
                }
                None => {}
            }
        }
        Ok(())
    }
}

//! Node messages and multinode chat forums (MULTINOD.PAS), kept in memory.

use crate::session::Session;
use std::io;
use std::time::Duration;

impl Session {
    /// Queue `msg` for `node` (sendnodemessage); false if nobody's there.
    pub fn send_node(&self, node: usize, msg: String) -> bool {
        match self.sh.db().online.get_mut(&node) {
            Some(o) => {
                o.inbox.push(msg);
                true
            }
            None => false,
        }
    }

    fn take_inbox(&self) -> Vec<String> {
        self.sh.db().online.get_mut(&self.node).map(|o| std::mem::take(&mut o.inbox)).unwrap_or_default()
    }

    /// Print waiting node messages (readnodemessages); done at each menu prompt.
    pub fn show_node_msgs(&mut self) -> io::Result<()> {
        for m in self.take_inbox() {
            self.nl()?;
            self.sprint(&m)?;
        }
        Ok(())
    }

    /// `NS` (enternodemessage)
    pub fn node_message(&mut self) -> io::Result<()> {
        self.whos_online()?;
        self.sprompt("|NL|U1Send to which node? |U3")?;
        let Ok(n) = self.input(3)?.parse::<usize>() else { return Ok(()) };
        if n == self.node || !self.sh.db().online.contains_key(&n) {
            return self.sprint("|12Nobody is on that node.");
        }
        self.sprompt("|U1Message: |U3")?;
        let m = self.input(70)?;
        if m.is_empty() {
            return Ok(());
        }
        let from = format!("|U5{} #{}|U4 on node #|U3{}|U5 says,|NL|U4\"|U3{m}|U4\"", self.user.name, self.unum, self.node);
        if self.send_node(n, from) { self.sprint("|U1Message sent.") } else { self.sprint("|12Nobody is on that node.") }
    }

    /// Send `msg` to everyone else in `forum`.
    fn forum_say(&self, forum: u8, msg: &str) {
        let mut db = self.sh.db();
        for (n, o) in db.online.iter_mut() {
            if *n != self.node && o.forum == forum {
                o.inbox.push(msg.to_string());
            }
        }
    }

    /// `NC` (entermultichat / multichat): line-mode chat between nodes.
    pub fn multi_chat(&mut self, data: &str) -> io::Result<()> {
        // 1-10 are open forums; 11 and up are OC pages, numbered 10 + node.
        let forum = match data.trim().parse::<u8>() {
            Ok(f @ 1..=10) => f,
            _ => {
                self.sprompt("|NL|U1Enter which forum |U2(|U31|U2-|U310)|U1? |U3")?;
                match self.input(3)?.parse::<u8>() {
                    Ok(f @ 1..=10) => f,
                    Ok(f @ 11..) if self.is_sysop() => f,
                    _ => return Ok(()),
                }
            }
        };
        self.chat_forum(forum)
    }

    pub fn chat_forum(&mut self, forum: u8) -> io::Result<()> {
        self.cls()?;
        self.sprint(&format!("|U1Entering forum #|U3{forum}|U1..."))?;
        self.sprint("|U3/Q|U1 = Quit  -  |U3/W|U1 = Who's on  -  |U3/S|U1 = Send node message|NL")?;
        let here: Vec<String> = self.sh.db().online.values().filter(|o| o.forum == forum).map(|o| o.name.clone()).collect();
        if !here.is_empty() {
            self.sprint(&format!("|U1Here now: |U3{}", here.join(", ")))?;
        }
        if let Some(o) = self.sh.db().online.get_mut(&self.node) {
            o.forum = forum;
        }
        self.set_doing(&format!("Chat forum #{forum}"));
        let name = self.user.name.clone();
        self.forum_say(forum, &format!("|U4* |U3{name}|U1 has joined the forum."));
        let r = self.chat_loop(forum, &name);
        if let Some(o) = self.sh.db().online.get_mut(&self.node) {
            o.forum = 0;
        }
        self.forum_say(forum, &format!("|U4* |U3{name}|U1 has left the forum."));
        r
    }

    fn chat_loop(&mut self, forum: u8, name: &str) -> io::Result<()> {
        let mut line = String::new();
        let prompt = "|U2> |U3";
        self.sprompt(prompt)?;
        loop {
            // ponytail: polls every 200ms; a per-node channel would wake instantly
            if !self.t.key_ready(Duration::from_millis(200))? {
                let msgs = self.take_inbox();
                if !msgs.is_empty() {
                    self.out(&format!("\r{}\r", " ".repeat(line.len() + 2)))?;
                    for m in msgs {
                        self.sprint(&m)?;
                    }
                    self.sprompt(prompt)?;
                    self.out(&line)?;
                }
                continue;
            }
            match self.getkey()? {
                '\r' => {
                    self.nl()?;
                    let said = std::mem::take(&mut line);
                    match said.to_uppercase().as_str() {
                        "/Q" => return Ok(()),
                        "/W" => self.whos_online()?,
                        "/S" => self.node_message()?,
                        "" => {}
                        _ => self.forum_say(forum, &format!("|U3{name}|U1: {}", crate::session::strip_pipes(&said))),
                    }
                    self.sprompt(prompt)?;
                }
                '\x08' | '\x7f' => {
                    if line.pop().is_some() {
                        self.out("\x08 \x08")?;
                    }
                }
                c if !c.is_control() && line.len() < 75 => {
                    line.push(c);
                    self.out(&c.to_string())?;
                }
                _ => {}
            }
        }
    }
}

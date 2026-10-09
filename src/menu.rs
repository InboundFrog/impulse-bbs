//! `.MNU` menu files in the line-oriented Impulse format (the text format
//! MNUCONV.PAS reads; one field per line). Header is 15 lines, then 9 per command.

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Command {
    /// Long description, shown in generated menu listings.
    pub ldesc: String,
    /// Short (highlighted) description, used by pulldown menus.
    pub sdesc: String,
    /// Pulldown position.
    pub x: String,
    pub y: String,
    /// Keys the user types to run it, or FIRSTCMD / EVERYTIME / ENTER.
    pub keys: String,
    pub acs: String,
    /// Two-letter command type ("cmdkeys"), e.g. `-/`, `MP`, `HC`.
    pub cmd: String,
    /// Command data ("MString").
    pub data: String,
    /// H = always hidden, U = always visible, P = pulldown.
    pub flags: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Menu {
    pub title: [String; 3],
    /// Help file shown at novice help level ("directive").
    pub help: String,
    pub tutorial: String,
    pub prompt: String,
    pub acs: String,
    pub password: String,
    pub fallback: String,
    /// Forced help level, generic columns and colours: kept for round trips.
    pub help_level: String,
    pub gen_cols: String,
    pub gcols: [String; 3],
    /// C = clear screen, D = don't center, N = no prompt, F = force pause,
    /// P = pulldown, T = show time left.
    pub flags: String,
    pub cmds: Vec<Command>,
}

pub fn parse(text: &str) -> Menu {
    let lines: Vec<&str> = text.lines().collect();
    let get = |i: usize| lines.get(i).copied().unwrap_or("").to_string();
    let mut m = Menu {
        title: [get(0), get(1), get(2)],
        help: get(3).trim().to_string(),
        tutorial: get(4).trim().to_string(),
        prompt: get(5),
        acs: get(6).trim().to_string(),
        password: get(7).trim().to_uppercase(),
        fallback: get(8).trim().to_string(),
        help_level: get(9).trim().to_string(),
        gen_cols: get(10).trim().to_string(),
        gcols: [get(11), get(12), get(13)].map(|s| s.trim().to_string()),
        flags: get(14).to_uppercase(),
        cmds: Vec::new(),
    };
    let mut i = 15;
    while i < lines.len() {
        m.cmds.push(Command {
            ldesc: get(i),
            sdesc: get(i + 1),
            x: get(i + 2).trim().to_string(),
            y: get(i + 3).trim().to_string(),
            keys: get(i + 4).trim().to_uppercase(),
            acs: get(i + 5).trim().to_string(),
            cmd: get(i + 6).trim().to_uppercase(),
            data: get(i + 7),
            flags: get(i + 8).to_uppercase(),
        });
        i += 9;
    }
    m
}

/// The menu in .MNU text form, CRLF line endings.
pub fn to_text(m: &Menu) -> String {
    let mut l: Vec<&str> = m.title.iter().map(String::as_str).collect();
    l.extend([&m.help, &m.tutorial, &m.prompt, &m.acs, &m.password, &m.fallback, &m.help_level, &m.gen_cols].map(String::as_str));
    l.extend(m.gcols.iter().map(String::as_str));
    l.push(&m.flags);
    for c in &m.cmds {
        l.extend([&c.ldesc, &c.sdesc, &c.x, &c.y, &c.keys, &c.acs, &c.cmd, &c.data, &c.flags].map(String::as_str));
    }
    l.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_original_main_mnu() {
        let text = include_str!("../tests/legacy-main.mnu");
        let m = super::parse(text);
        assert_eq!(m.title[0], "Main Menu");
        assert_eq!(m.help, "MAIN");
        assert_eq!(m.prompt, "Main:");
        assert_eq!(m.flags, "C");
        assert_eq!(m.cmds.len(), 21);
        let c = &m.cmds[0];
        assert_eq!((c.keys.as_str(), c.acs.as_str(), c.cmd.as_str(), c.data.as_str()), ("A", "s10", "-/", "auto"));
        let c = &m.cmds[3];
        assert_eq!((c.keys.as_str(), c.cmd.as_str(), c.data.as_str()), ("G", "HC", "|03leave?"));
        // Writing it back gives the same menu.
        let again = super::parse(&super::to_text(&m));
        assert_eq!(serde_json::to_value(&again).unwrap(), serde_json::to_value(&m).unwrap());
    }
}

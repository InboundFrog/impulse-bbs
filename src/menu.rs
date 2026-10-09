//! `.MNU` menu files in the line-oriented Impulse format (the text format
//! MNUCONV.PAS reads; one field per line). Header is 15 lines, then 9 per command.

#[derive(Clone, Default)]
pub struct Command {
    /// Long description, shown in generated menu listings.
    pub ldesc: String,
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

#[derive(Clone, Default)]
pub struct Menu {
    pub title: [String; 3],
    /// Help file shown at novice help level ("directive").
    pub help: String,
    pub prompt: String,
    pub acs: String,
    pub password: String,
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
        // 4: tutorial file, unused since 7.0
        prompt: get(5),
        acs: get(6).trim().to_string(),
        password: get(7).trim().to_uppercase(),
        // 8: fallback menu (unused: missing menus just leave you where you are)
        // 9..=13: forced help level, generic columns, generic colours (gone in 7.1)
        flags: get(14).to_uppercase(),
        cmds: Vec::new(),
    };
    let mut i = 15;
    while i < lines.len() {
        m.cmds.push(Command {
            ldesc: get(i),
            // i+1: short description, i+2/i+3: pulldown x/y (pulldowns unsupported)
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

#[cfg(test)]
mod tests {
    #[test]
    fn parses_original_main_mnu() {
        let m = super::parse(include_str!("../tests/legacy-main.mnu"));
        assert_eq!(m.title[0], "Main Menu");
        assert_eq!(m.help, "MAIN");
        assert_eq!(m.prompt, "Main:");
        assert_eq!(m.flags, "C");
        assert_eq!(m.cmds.len(), 21);
        let c = &m.cmds[0];
        assert_eq!((c.keys.as_str(), c.acs.as_str(), c.cmd.as_str(), c.data.as_str()), ("A", "s10", "-/", "auto"));
        let c = &m.cmds[3];
        assert_eq!((c.keys.as_str(), c.cmd.as_str(), c.data.as_str()), ("G", "HC", "|03leave?"));
    }
}

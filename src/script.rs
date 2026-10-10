//! Impulse Script Language (SCRIPT.PAS, ISLU.PAS): `IS` runs a script from
//! the text path. `.ISL` files are plain text; anything else is taken to be
//! ISLC output, where every byte is stored as 255 minus itself.

use crate::session::{Flow, Session, find_ci};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Word `n` of `s`, splitting on single spaces (Parm).
fn parm(s: &str, n: usize) -> &str {
    s.split(' ').nth(n).unwrap_or("")
}

/// Everything from word `n` on (ParmLine).
fn parm_line(s: &str, n: usize) -> &str {
    s.splitn(n + 1, ' ').nth(n).unwrap_or("")
}

/// Pascal `Val` on an integer.
fn num(s: &str) -> Option<i64> {
    s.parse().ok()
}

fn value(s: &str) -> i64 {
    num(s).unwrap_or(0)
}

/// Undo ISLC's byte inversion (decomp_line).
fn decomp(s: &[u8]) -> String {
    s.iter().map(|&b| (255 - b) as char).collect()
}

/// Control characters as `[#n]` (CtrlStr).
fn ctrl_str(s: &str) -> String {
    s.chars().map(|c| if (c as u32) < 32 { format!("[#{}]", c as u32) } else { c.to_string() }).collect()
}

struct Isl {
    lines: Vec<String>,
    vars: BTreeMap<String, String>,
    stack: Vec<usize>,
    cwd: PathBuf,
    fin: Option<io::Lines<io::BufReader<std::fs::File>>>,
    fout: Option<std::fs::File>,
    error: String,
}

impl Isl {
    fn load(path: &Path, cwd: PathBuf) -> io::Result<Isl> {
        let bytes = std::fs::read(path)?;
        let plain = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("isl"));
        let lines = bytes
            .split(|&b| b == b'\n')
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
            .map(|l| if plain { l.iter().map(|&b| b as char).collect() } else { decomp(l) })
            .map(|l: String| l.split(';').next().unwrap_or("").trim().to_string())
            .collect();
        let mut vars = BTreeMap::new();
        for (k, v) in [("ISLVER", "1"), ("ISLREV", "1"), ("SPACE", " "), ("CR", "\r"), ("LF", "\n"), ("NULL", "\0"), ("ESC", "\x1b")] {
            vars.insert(k.to_string(), v.to_string());
        }
        Ok(Isl { lines, vars, stack: Vec::new(), cwd, fin: None, fout: None, error: String::new() })
    }

    fn get(&self, n: &str) -> String {
        self.vars.get(&n.to_uppercase()).cloned().unwrap_or_default()
    }

    /// Setting a variable to "" removes it (SetVar).
    fn set(&mut self, n: &str, v: impl Into<String>) {
        let v = v.into();
        if n.is_empty() {
            self.error = "Variable name missing".into();
        } else if v.is_empty() {
            self.vars.remove(&n.to_uppercase());
        } else {
            self.vars.insert(n.to_uppercase(), v);
        }
    }

    /// `&NAME&` or `&NAME ` becomes the variable's value (Expand).
    fn expand(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('&') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            let end = after.find(['&', ' ']).unwrap_or(after.len());
            out.push_str(&self.get(&after[..end]));
            rest = if after[end..].starts_with('&') { &after[end + 1..] } else { &after[end..] };
        }
        out + rest
    }

    /// Index of the line after `:label`, as the next line to run (Search).
    fn search(&mut self, label: &str) -> Option<usize> {
        if label.is_empty() {
            self.error = "Label missing".into();
            return None;
        }
        let i = self.lines.iter().position(|l| l.strip_prefix(':').is_some_and(|x| x.eq_ignore_ascii_case(label)));
        if i.is_none() {
            self.error = format!("Label not found: {label}");
        }
        i.map(|i| i + 1)
    }

    fn path(&self, p: &str) -> PathBuf {
        self.cwd.join(p.replace('\\', "/"))
    }

    fn compare(&mut self, a: &str, b: &str) {
        let o = match (num(a), num(b)) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => a.cmp(b),
        };
        let sym = match o {
            std::cmp::Ordering::Less => "<",
            std::cmp::Ordering::Equal => "=",
            std::cmp::Ordering::Greater => ">",
        };
        self.set("COMPARE", sym);
    }

    fn io_err(&mut self, r: io::Result<()>) {
        let code = match r {
            Ok(()) => 0,
            Err(e) => e.raw_os_error().unwrap_or(1),
        };
        self.set("IOERR", code.to_string());
    }
}

/// strftime-ish for GETTIME (TimeStr): H M S X Y O D W.
fn time_str(fmt: &str) -> String {
    let n = chrono::Local::now();
    fmt.chars()
        .map(|c| match c.to_ascii_uppercase() {
            'H' => n.format("%H").to_string(),
            'M' => n.format("%M").to_string(),
            'S' => n.format("%S").to_string(),
            'X' => format!("{:02}", n.timestamp_subsec_millis() / 10),
            'Y' => n.format("%Y").to_string(),
            'O' => n.format("%m").to_string(),
            'D' => n.format("%d").to_string(),
            'W' => n.format("%w").to_string(),
            _ => c.to_string(),
        })
        .collect()
}

impl Session {
    /// `IS` (doscript)
    pub fn script(&mut self, name: &str) -> io::Result<Flow> {
        self.run_script(name, 0)
    }

    fn run_script(&mut self, name: &str, depth: u8) -> io::Result<Flow> {
        let c = self.cfg();
        let Some(path) = find_ci(&c.text_path, name.trim()) else {
            self.sprint(&format!("|12Script {} not found.", name.trim()))?;
            return Ok(Flow::Next);
        };
        let mut s = Isl::load(&path, c.text_path.clone())?;
        let (flow, line) = self.isl_run(&mut s, depth)?;
        if !s.error.is_empty() {
            self.sprint(&format!("|12Script execution error in {}", path.display()))?;
            self.sprint(&format!("|12{} on line {line}", s.error))?;
            self.sprint("|12Please notify Sysop")?;
        }
        Ok(flow)
    }

    /// Run until END, an error or a menu change; returns the last line number.
    fn isl_run(&mut self, s: &mut Isl, depth: u8) -> io::Result<(Flow, usize)> {
        let mut pc = 0;
        // ponytail: a step cap stands in for Impulse's "it's the SysOp's script" trust
        for _ in 0..1_000_000 {
            let line = s.expand(s.lines.get(pc).map_or("END", |l| l.as_str()));
            pc += 1;
            let cmd = parm(&line, 0).to_uppercase();
            let (p1, p2, p3) = (parm(&line, 1).to_string(), parm(&line, 2).to_string(), parm(&line, 3).to_string());
            let (l1, l2) = (parm_line(&line, 1).to_string(), parm_line(&line, 2).to_string());
            let jump = |s: &mut Isl, pc: &mut usize, label: &str| {
                if let Some(i) = s.search(label) {
                    *pc = i;
                }
            };
            match cmd.as_str() {
                "END" => return Ok((Flow::Next, pc)),
                "LWRITELN" | "LWRITE" => self.log(&format!("ISL: {l1}")),
                "WRITELN" => self.sprint(&l1)?,
                "WRITE" => self.sprompt(&l1)?,
                "EXEC" | "DEXEC" => {
                    let cl = if cmd == "EXEC" { l1.clone() } else { format!("{p1} {l2}") };
                    let r = self.t.run_external(std::process::Command::new("sh").arg("-c").arg(&cl).current_dir(&s.cwd), true);
                    match r {
                        Ok(st) => {
                            s.set("EL", st.code().unwrap_or(-1).to_string());
                            s.set("DOSERR", "0");
                        }
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Err(e),
                        Err(_) => s.set("DOSERR", "2"),
                    }
                }
                "GOTO" => jump(s, &mut pc, &p1),
                "GOSUB" => {
                    s.stack.push(pc);
                    jump(s, &mut pc, &p1);
                }
                "RETURN" => match s.stack.pop() {
                    Some(r) => pc = r,
                    None => s.error = "Unable to pop from stack".into(),
                },
                "POP" => {
                    if s.stack.pop().is_none() {
                        s.error = "Unable to pop from stack".into();
                    }
                }
                "PUSH" => {
                    let at = if p1.is_empty() { Some(pc) } else { s.search(&p1) };
                    s.stack.extend(at);
                }
                "CLSTACK" => s.stack.clear(),
                "SET" => s.set(&p1, l2),
                "ASK" => {
                    self.sprompt(&l2)?;
                    let v = self.input_caps(80usize.saturating_sub(l2.len() + 1).max(1))?;
                    s.set(&p1, v);
                }
                "UPCASE" => {
                    let v = s.get(&p1).to_uppercase();
                    s.set(&p1, v);
                }
                "LOCATE" if self.user.ansi => self.out(&format!("\x1b[{p2};{p1}H"))?,
                "CLS" => self.cls()?,
                "CLEOL" if self.user.ansi => self.out("\x1b[K")?,
                "PAUSE" => self.pause()?,
                "INKEY" => {
                    if self.t.key_ready(Duration::ZERO)? && (self.getkey()? as u32).to_string() == p1 {
                        jump(s, &mut pc, &p2);
                    }
                }
                "READKEY" => {
                    let k = loop {
                        let k = self.getkey()?;
                        if k != '&' || !s.get("AMP").is_empty() {
                            break k;
                        }
                    };
                    s.set(&p1, k.to_string());
                }
                "LEN" => s.set(&p1, l2.chars().count().to_string()),
                "CHR" => s.set(&p1, char::from(value(&p2) as u8).to_string()),
                "CENTER" | "SCENTER" => {
                    let w = crate::session::strip_pipes(&l1).chars().count();
                    self.sprint(&format!("{}{l1}", " ".repeat(40usize.saturating_sub(w / 2))))?;
                }
                "COMPARE" => s.compare(&p1, &p2),
                "IF" | "IFEXIST" | "IFNUM" => {
                    let yes = match cmd.as_str() {
                        "IF" => {
                            let c = s.get("COMPARE");
                            !c.is_empty() && p1.contains(&c)
                        }
                        "IFEXIST" => s.path(&p1).exists(),
                        _ => num(&p1).is_some(),
                    };
                    if yes {
                        jump(s, &mut pc, &p2);
                    } else if !p3.is_empty() {
                        jump(s, &mut pc, &p3);
                    }
                }
                "REMOVE" => {
                    let ch = p2.chars().next().unwrap_or(' ');
                    let v = s.get(&p1).replace(ch, "");
                    s.set(&p1, v);
                }
                "LTRIM" => {
                    let v = s.get(&p1).trim_start().to_string();
                    s.set(&p1, v);
                }
                "FWOPEN" | "FWAPPEND" => {
                    let r = std::fs::OpenOptions::new().create(true).write(true).append(cmd == "FWAPPEND").truncate(cmd == "FWOPEN").open(s.path(&p1));
                    let r = r.map(|f| s.fout = Some(f));
                    s.io_err(r);
                }
                "FWRITE" => {
                    let r = match s.fout.as_mut() {
                        Some(f) => writeln!(f, "{l1}"),
                        None => Err(io::Error::other("not open")),
                    };
                    s.io_err(r);
                }
                "FWCLOSE" => {
                    s.fout = None;
                    s.io_err(Ok(()));
                }
                "FROPEN" => {
                    let r = std::fs::File::open(s.path(&p1)).map(|f| s.fin = Some(io::BufReader::new(f).lines()));
                    s.io_err(r);
                    if !p2.is_empty() {
                        s.set("FEOF", p2);
                    }
                }
                "FREAD" => match s.fin.as_mut().and_then(|f| f.next()) {
                    Some(Ok(mut l)) => {
                        if s.get("AMP").is_empty() {
                            l.retain(|c| c != '&');
                        }
                        s.set(&p1, l);
                        s.io_err(Ok(()));
                    }
                    _ => {
                        let l = s.get("FEOF");
                        jump(s, &mut pc, &l);
                    }
                },
                "FRCLOSE" => {
                    s.fin = None;
                    s.io_err(Ok(()));
                }
                "CD" => {
                    let p = s.path(&p1);
                    let r = if p.is_dir() {
                        s.cwd = p;
                        Ok(())
                    } else {
                        Err(io::Error::other("no such directory"))
                    };
                    s.io_err(r);
                }
                "MD" => {
                    let r = std::fs::create_dir(s.path(&p1));
                    s.io_err(r);
                }
                "RD" => {
                    let r = std::fs::remove_dir(s.path(&p1));
                    s.io_err(r);
                }
                "INC" | "DEC" | "MULT" | "DIV" => {
                    let Some(v) = num(&s.get(&p1)) else { continue };
                    let n = if p2.is_empty() && cmd != "MULT" && cmd != "DIV" { 1 } else { value(&p2) };
                    let r = match cmd.as_str() {
                        "INC" => v + n,
                        "DEC" => v - n,
                        "MULT" => v * n,
                        _ if n == 0 => {
                            s.error = "Division by zero".into();
                            continue;
                        }
                        _ => v / n,
                    };
                    s.set(&p1, r.to_string());
                }
                "STRING" => s.set(&p1, p3.chars().next().map(|c| c.to_string().repeat(value(&p2).max(0) as usize)).unwrap_or_default()),
                "CMD" => s.set(&p1, p2),
                "ARG" => s.set(&p1, parm(parm_line(&line, 3), value(&p2) as usize)),
                "ARGLINE" => s.set(&p1, parm_line(parm_line(&line, 3), value(&p2) as usize)),
                "INSTR" => {
                    let at = p2.find(&p1).map_or(0, |i| i + 1);
                    s.set("INSTR", at.to_string());
                    s.set("COMPARE", if at > 0 { "=" } else { "<" });
                }
                "SHOWVAR" => {
                    let v: Vec<String> = s.vars.iter().map(|(k, v)| format!("{k:<8} = {}", ctrl_str(v))).collect();
                    for l in v {
                        self.out(&l)?;
                        self.nl()?;
                    }
                }
                "SHOWSTACK" => {
                    for (i, v) in s.stack.clone().iter().enumerate() {
                        self.out(&format!("{i:2} = {v:4}"))?;
                        self.nl()?;
                    }
                }
                "GETDSIZE" | "GETDFREE" => {
                    let (size, free) = disk(&s.cwd).unwrap_or_default();
                    s.set(&p1, if cmd == "GETDSIZE" { size } else { free }.to_string())
                }
                "GETX" | "GETY" => {
                    let (x, y) = self.cursor_pos()?.unwrap_or((0, 0));
                    s.set(&p1, if cmd == "GETX" { x } else { y }.to_string())
                }
                "GETVER" => s.set(&p1, "7.1"),
                "GETDIR" => s.set(&p1, s.cwd.display().to_string()),
                "GETTIME" => s.set(&p1, time_str(&l2)),
                "CUT" => {
                    let start = (value(&p2).max(1) - 1) as usize;
                    let v: String = parm_line(&line, 4).chars().skip(start).take(value(&p3).max(0) as usize).collect();
                    s.set(&p1, v);
                }
                "CTRLSTR" => {
                    let v = ctrl_str(&s.get(&p1));
                    s.set(&p1, v);
                }
                "DELAY" => std::thread::sleep(Duration::from_millis(value(&p1).clamp(0, 10_000) as u64)),
                "SOUND" => std::thread::sleep(Duration::from_millis(value(&p2).clamp(0, 10_000) as u64)),
                "BEEP" => self.out(&"\x07".repeat(value(&p1).clamp(0, 10) as usize))?,
                "ERROR" => s.error = l1,
                "RANDOM" => s.set(&p1, crate::session::rand(value(&p2).max(1) as usize).to_string()),
                "EXPAND" => {
                    let v = s.path(&s.get(&p1)).display().to_string();
                    s.set(&p1, v);
                }
                "SPLIT" => {
                    let p = PathBuf::from(s.get(&p1));
                    let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                    let name = p.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                    s.set("SPLDIR", dir);
                    s.set("SPLNAME", name);
                    s.set("SPLEXT", ext);
                }
                "COPYVAR" => {
                    let v = s.get(&p1);
                    s.set(&p2, v);
                }
                "LJUST" | "RJUST" | "CJUST" => {
                    let v = s.get(&p1);
                    let pad = (value(&p2).max(0) as usize).saturating_sub(v.chars().count());
                    let r = match cmd.as_str() {
                        "LJUST" => format!("{v}{}", " ".repeat(pad)),
                        "RJUST" => format!("{}{v}", " ".repeat(pad)),
                        _ => format!("{}{v}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2)),
                    };
                    s.set(&p1, r);
                }
                "GETENV" => s.set(&p1, std::env::var(&p2).unwrap_or_default()),
                "CHAIN" if depth < 8 => {
                    let f = self.run_script(&p1, depth + 1)?;
                    if !matches!(f, Flow::Next) {
                        return Ok((f, pc));
                    }
                }
                "LOADMENU" => return Ok((self.goto_menu(&p1)?, pc)),
                "MENUCMD" => {
                    let f = self.do_cmd(&p1.to_uppercase(), &l2)?;
                    if !matches!(f, Flow::Next) {
                        return Ok((f, pc));
                    }
                }
                // labels, blank lines, SHOWTRAP (no command sets traps), SETTIME, SETDATE, RANDOMIZE
                _ => {}
            }
            if !s.error.is_empty() {
                return Ok((Flow::Next, pc));
            }
        }
        s.error = "Script ran too long".into();
        Ok((Flow::Next, pc))
    }
}

/// Size and free bytes of the filesystem holding `dir` (DiskSize/DiskFree), from df(1).
fn disk(dir: &std::path::Path) -> Option<(u64, u64)> {
    let out = std::process::Command::new("df").arg("-kP").arg(dir).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let f: Vec<u64> = text.lines().nth(1)?.split_whitespace().skip(1).take(3).filter_map(|n| n.parse().ok()).collect();
    Some((*f.first()? * 1024, *f.get(2)? * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isl_basics() {
        assert_eq!(parm("SET X hello world", 1), "X");
        assert_eq!(parm_line("SET X hello world", 2), "hello world");
        assert_eq!(parm("A", 3), "");
        assert_eq!(decomp(&[255 - b'H', 255 - b'i']), "Hi");
        let mut s = Isl { lines: vec![], vars: BTreeMap::new(), stack: vec![], cwd: PathBuf::new(), fin: None, fout: None, error: String::new() };
        s.set("name", "Bob");
        assert_eq!(s.expand("hi &NAME& and &name ok &nope&!"), "hi Bob and Bob ok !");
        s.compare("10", "9");
        assert_eq!(s.get("COMPARE"), ">");
        s.compare("abc", "abd");
        assert_eq!(s.get("COMPARE"), "<");
        s.set("name", "");
        assert_eq!(s.get("NAME"), "");
    }
}

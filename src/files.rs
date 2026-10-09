//! File bases, listings and transfers (FILE*.PAS).

use crate::config::{FBase, Protocol};
use crate::db::FileRec;
use crate::session::{Session, find_ci, today};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// DOS-style wildcard match, case-insensitive. `FOO.*` matches `FOO`.
pub fn wild(pat: &str, name: &str) -> bool {
    fn m(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => m(&p[1..], s) || (!s.is_empty() && m(p, &s[1..])),
            (Some(b'?'), Some(_)) => m(&p[1..], &s[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && m(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let pat = if pat.is_empty() || pat == "*.*" { "*" } else { pat };
    m(pat.as_bytes(), name.as_bytes()) || (!name.contains('.') && m(pat.as_bytes(), format!("{name}.").as_bytes()))
}

fn size_str(b: u64) -> String {
    if b < 10_000 { b.to_string() } else { format!("{}k", b / 1024) }
}

fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Where a base's file lives: the download path, else the upload path.
pub fn file_path(fb: &FBase, name: &str) -> PathBuf {
    let p = fb.dl_path.join(name);
    if p.exists() || fb.ul_path.as_os_str().is_empty() { p } else { fb.ul_path.join(name) }
}

pub fn ul_dir(fb: &FBase) -> &Path {
    if fb.ul_path.as_os_str().is_empty() { &fb.dl_path } else { &fb.ul_path }
}

/// Files in a listing: (base index, record).
type Items = Vec<(usize, FileRec)>;

const LIST_HELP: &[&str] = &[
    "|NL|U1(|U3CR|U1) Next page  (|U3F|U1)lag or (|U3+|U1) to tag  (|U3D|U1)ownload  (|U3V|U1)iew interior",
    "|U1(|U3T|U1)ype file  (|U3I|U1)nfo  (|U3J|U1)ump to file  (|U3U|U1)pload  (|U3Q|U1)uit listing",
];

impl Session {
    // ---- bases --------------------------------------------------------------

    /// File base indexes this user may use, in the current conference.
    pub fn fbases(&self) -> Vec<usize> {
        let b = &self.cfg().fbases;
        (0..b.len()).filter(|&i| self.acs(&b[i].acs) && (self.fconf == 0 || b[i].conf == 0 || b[i].conf == self.fconf)).collect()
    }

    fn fbase_ok(&mut self, b: usize) -> io::Result<bool> {
        let fb = self.cfg().fbases[b].clone();
        if fb.password.is_empty() || self.fbase_pw.contains(&fb.file) {
            return Ok(true);
        }
        self.sprompt(&format!("|NL|U3{}|U1 password: |U3", fb.name))?;
        if self.input_pw(20)?.eq_ignore_ascii_case(&fb.password) {
            self.fbase_pw.push(fb.file);
            return Ok(true);
        }
        self.sprint("|12Wrong password.")?;
        Ok(false)
    }

    fn list_fbases(&mut self) -> io::Result<Vec<usize>> {
        let list = self.fbases();
        self.tmpl("filebase.top", &[], "|NL|U3File bases")?;
        for (i, &b) in list.iter().enumerate() {
            let fb = self.cfg().fbases[b].clone();
            let scan = if self.user.no_fscan.contains(&fb.file) { " " } else { "*" };
            let n = self.sh.db().files(&fb.file)?.len();
            let vars = [("FE", fb.name.clone()), ("FI", (i + 1).to_string()), ("FN", n.to_string()), ("FZ", scan.into())];
            self.tmpl("filebase.mid", &vars, "|U2%FI. |U3%FZ|U1%FE |U2(%FN files)")?;
        }
        self.tmpl("filebase.bot", &[], "")?;
        Ok(list)
    }

    pub fn fbase_by_number(&mut self, n: &str) -> io::Result<()> {
        let list = self.fbases();
        match n.parse::<usize>() {
            Ok(n @ 1..) if n <= list.len() => {
                if !self.fbase_ok(list[n - 1])? {
                    return Ok(());
                }
                self.fbase = list[n - 1];
                self.user.last_fbase = self.fbase;
                let name = self.cfg().fbases[self.fbase].name.clone();
                self.sprompt("|U1Joined |U3")?;
                self.out(&name)?;
                self.nl()
            }
            _ => self.sprint("|12No such file base."),
        }
    }

    /// `FA` (fbasechange).
    pub fn change_fbase(&mut self, data: &str) -> io::Result<()> {
        let list = self.fbases();
        if list.is_empty() {
            return self.sprint("|NL|12No file bases available.");
        }
        let pos = list.iter().position(|&b| b == self.fbase).unwrap_or(0);
        let n = match data.trim() {
            "+" => (pos + 1) % list.len() + 1,
            "-" => (pos + list.len() - 1) % list.len() + 1,
            _ => {
                self.list_fbases()?;
                self.sprompt("|NL|U1Change to which file base? |U3")?;
                match self.input(3)?.parse() {
                    Ok(n) => n,
                    Err(_) => return Ok(()),
                }
            }
        };
        self.fbase_by_number(&n.to_string())
    }

    /// `FZ` (setdirs): choose which bases new file scans cover.
    pub fn fscan_bases(&mut self) -> io::Result<()> {
        loop {
            let list = self.list_fbases()?;
            self.sprompt("|NL|U1Toggle new file scan of which base |U2(|U3*|U2=scanned, Enter quits)|U1? |U3")?;
            let Ok(n @ 1..) = self.input(3)?.parse::<usize>() else { return Ok(()) };
            let Some(&b) = list.get(n - 1) else { return Ok(()) };
            let file = self.cfg().fbases[b].file.clone();
            match self.user.no_fscan.iter().position(|f| *f == file) {
                Some(i) => {
                    self.user.no_fscan.remove(i);
                }
                None => self.user.no_fscan.push(file),
            }
        }
    }

    /// `FK`: which columns file listings show.
    pub fn file_list_fields(&mut self) -> io::Result<()> {
        const F: &[(char, &str)] = &[
            ('N', "File name"),
            ('S', "Size"),
            ('P', "File points"),
            ('D', "Description"),
            ('E', "Extended description"),
            ('W', "Who uploaded it"),
            ('T', "Date uploaded"),
            ('C', "Number of downloads"),
        ];
        loop {
            self.nl()?;
            for (k, name) in F {
                let on = if self.user.list_fields.contains(*k) { "|U3On " } else { "|U2Off" };
                self.sprint(&format!("|U2[|U3{k}|U2] |U1{name:<22}{on}"))?;
            }
            self.sprompt("|NL|U1Toggle which |U2(|U3Enter|U2=done)|U1? |U3")?;
            let c = self.onek("NSPDEWTC\r")?;
            if c == '\r' {
                return Ok(());
            }
            let f = &mut self.user.list_fields;
            if f.contains(c) { f.retain(|x| x != c) } else { f.push(c) }
        }
    }

    // ---- listing ------------------------------------------------------------

    fn can_see_file(&self, f: &FileRec) -> bool {
        !f.unvalidated || self.is_sysop() || f.owner_num == self.unum
    }

    /// One listing entry (pfndd); returns the lines printed.
    fn print_file(&mut self, n: usize, f: &FileRec, fb: &FBase) -> io::Result<u16> {
        let fl = self.user.list_fields.clone();
        self.sprompt(&format!("|U2{n:<3} "))?;
        if fl.contains('N') {
            self.sprompt("|U3")?;
            self.out(&format!("{:<12} ", f.name))?;
        }
        if fl.contains('S') {
            self.sprompt(&format!("|U1{:>6} ", size_str(f.size)))?;
        }
        if fl.contains('P') {
            self.sprompt(&format!("|U2{:>3} ", f.points))?;
        }
        if fl.contains('D') {
            self.sprompt("|U1")?;
            let d: String = crate::session::strip_pipes(&f.desc).chars().take(45).collect();
            self.out(&d)?;
        }
        if f.unvalidated {
            self.sprompt(" |12(unvalidated)")?;
        }
        self.nl()?;
        let mut lines = 1;
        let pad = " ".repeat(10);
        if fl.contains('E') {
            for l in f.ext.clone() {
                self.sprompt(&format!("{pad}|U1"))?;
                self.out(&l)?;
                self.nl()?;
                lines += 1;
            }
        }
        if fl.contains('W') && self.acs(&fb.name_acs) {
            self.sprompt(&format!("{pad}|U0Uploaded by|U9: |U3"))?;
            self.out(&f.owner)?;
            self.nl()?;
            lines += 1;
        }
        if fl.contains('T') {
            self.sprint(&format!("{pad}|U0Uploaded on|U9: |U3{}", f.date))?;
            lines += 1;
        }
        if fl.contains('C') {
            self.sprint(&format!("{pad}|U0Number of D/L's|U9: |U3{}", f.downloads))?;
            lines += 1;
        }
        Ok(lines)
    }

    fn file_header(&mut self, fb: &FBase) -> io::Result<()> {
        self.vars.insert("FE".into(), fb.name.clone());
        let shown = self.printf("filehdr")?;
        self.vars.clear();
        if !shown {
            self.sprompt("|NL|U3")?;
            self.out(&fb.name)?;
            self.nl()?;
            self.sprint("|U9-------------------------------------------------------------------------")?;
        }
        Ok(())
    }

    /// Page through files with the FILEP prompt. Returns true if the user quit.
    fn list_files(&mut self, items: &Items) -> io::Result<bool> {
        if items.is_empty() {
            return Ok(false);
        }
        let prompt = self.cfg().strings.file_prompt.clone();
        let page = self.user.page_len.saturating_sub(4).max(5);
        let (mut i, mut base) = (0, usize::MAX);
        loop {
            let mut lines = 0;
            let mut first = true;
            while i < items.len() && lines < page {
                let (b, f) = &items[i];
                let fb = self.cfg().fbases[*b].clone();
                if *b != base || first {
                    self.file_header(&fb)?;
                    base = *b;
                    first = false;
                    lines += 3;
                }
                lines += self.print_file(i + 1, f, &fb)?;
                i += 1;
            }
            loop {
                let k = self.sub_prompt("FILEP", &prompt, "NLQF+DRVTIJSU?")?;
                match k {
                    'N' | 'L' if i >= items.len() => return Ok(false),
                    'N' | 'L' => break,
                    'Q' => return Ok(true),
                    'J' => {
                        if let Some(n) = self.pick(items, "Jump to which file")? {
                            i = n;
                            break;
                        }
                    }
                    'F' | '+' => {
                        if let Some(n) = self.pick(items, "Flag which file")? {
                            self.batch_add(&items[n])?;
                        }
                    }
                    'D' | 'R' => {
                        if let Some(n) = self.pick(items, "Download which file")? {
                            self.download(vec![items[n].clone()])?;
                        }
                    }
                    'V' => {
                        if let Some(n) = self.pick(items, "View which file")? {
                            self.view_file(&items[n])?;
                        }
                    }
                    'T' => {
                        if let Some(n) = self.pick(items, "Type which file")? {
                            self.type_file(&items[n])?;
                        }
                    }
                    'I' => {
                        if let Some(n) = self.pick(items, "Info on which file")? {
                            self.file_info(&items[n])?;
                        }
                    }
                    'S' | 'U' => self.upload()?,
                    '?' => {
                        for l in LIST_HELP {
                            self.sprint(l)?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn pick(&mut self, items: &Items, what: &str) -> io::Result<Option<usize>> {
        self.sprompt(&format!("|NL|U0{what} |U9(|U01|U9-|U0{}|U9)? |U3", items.len()))?;
        Ok(match self.input(4)?.parse::<usize>() {
            Ok(n @ 1..) if n <= items.len() => Some(n - 1),
            _ => None,
        })
    }

    /// Files in base `b` this user can see, matching `keep`.
    fn base_files(&self, b: usize, keep: impl Fn(&FileRec) -> bool) -> io::Result<Items> {
        let file = self.cfg().fbases[b].file.clone();
        Ok(self.sh.db().files(&file)?.into_iter().filter(|f| self.can_see_file(f) && keep(f)).map(|f| (b, f)).collect())
    }

    fn ask_spec(&mut self, data: &str) -> io::Result<String> {
        if !data.trim().is_empty() {
            return Ok(data.trim().to_string());
        }
        self.sprompt("|NL|U1File name |U2(|U3Enter|U2=*.*)|U1: |U3")?;
        let s = self.input_caps(12)?;
        Ok(if s.is_empty() { "*.*".into() } else { s })
    }

    /// `FL` (listfiles): the current base.
    pub fn list_cur(&mut self, data: &str) -> io::Result<()> {
        let b = self.fbase;
        if self.cfg().fbases.get(b).is_none() || !self.fbase_ok(b)? {
            return self.sprint("|NL|12No file base selected.");
        }
        let spec = self.ask_spec(data)?;
        let items = self.base_files(b, |f| wild(&spec, &f.name))?;
        if items.is_empty() {
            return self.sprint("|NL|U1No files found.");
        }
        self.set_doing("Listing files");
        self.list_files(&items).map(|_| ())
    }

    /// Search every base (search / searchd / nf), skipping locked ones.
    fn search_all(&mut self, keep: impl Fn(&Self, usize, &FileRec) -> bool) -> io::Result<()> {
        let mut items = Items::new();
        for b in self.fbases() {
            let fb = self.cfg().fbases[b].clone();
            if !fb.password.is_empty() && !self.fbase_pw.contains(&fb.file) {
                continue;
            }
            items.extend(self.base_files(b, |f| keep(self, b, f))?);
        }
        if items.is_empty() {
            return self.sprint("|NL|U1No files found.");
        }
        self.sprint(&format!("|NL|U1Found |U3{}|U1 files.", items.len()))?;
        self.list_files(&items).map(|_| ())
    }

    /// `FS` (search): file names across all bases.
    pub fn search_names(&mut self, data: &str) -> io::Result<()> {
        let spec = self.ask_spec(data)?;
        self.search_all(|_, _, f| wild(&spec, &f.name))
    }

    /// `FF`: search by name or description.
    pub fn find_files(&mut self) -> io::Result<()> {
        let p = self.cfg().strings.file_search.clone();
        self.sprompt(&format!("|NL{p}"))?;
        match self.onek("WSQ\r")? {
            'W' => self.search_names(""),
            'S' => {
                self.sprompt("|NL|U1Search descriptions for: |U3")?;
                let s = self.input(40)?.to_lowercase();
                if s.is_empty() {
                    return Ok(());
                }
                self.search_all(|_, _, f| f.desc.to_lowercase().contains(&s) || f.ext.iter().any(|l| l.to_lowercase().contains(&s)))
            }
            _ => Ok(()),
        }
    }

    /// `FN` / `CN F` (nf): files uploaded since the new-scan date.
    pub fn file_newscan(&mut self) -> io::Result<()> {
        let since = self.new_date.clone();
        let shown = if since.is_empty() { "your first call" } else { &since };
        self.sprint(&format!("|NL|U1Scanning for files uploaded since |U3{shown}|U1..."))?;
        self.search_all(|s, b, f| f.date >= since && !s.user.no_fscan.contains(&s.cfg().fbases[b].file))
    }

    /// `FP` (pointdate): set the new-scan date for this call.
    pub fn new_scan_date(&mut self) -> io::Result<()> {
        self.sprint(&format!("|NL|U1Current new-scan date is |U3{}", self.new_date))?;
        self.sprompt("|U1New date (YYYY-MM-DD or MM/DD/YYYY, Enter keeps): |U3")?;
        let s = self.input(10)?;
        let d = chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").or_else(|_| chrono::NaiveDate::parse_from_str(&s, "%m/%d/%Y"));
        match d {
            Ok(d) => {
                self.new_date = d.format("%Y-%m-%d").to_string();
                self.sprint(&format!("|U1New-scan date set to |U3{}", self.new_date))
            }
            Err(_) if s.is_empty() => Ok(()),
            Err(_) => self.sprint("|12Bad date."),
        }
    }

    /// Ask for a file name in the current base, then the other bases.
    fn ask_file(&mut self, prompt: &str, any_base: bool) -> io::Result<Items> {
        self.sprompt(&format!("|NL|U1{prompt}: |U3"))?;
        let spec = self.input_caps(12)?;
        if spec.is_empty() {
            return Ok(Items::new());
        }
        let mut found = self.base_files(self.fbase, |f| wild(&spec, &f.name))?;
        if found.is_empty() && any_base {
            for b in self.fbases() {
                if b != self.fbase {
                    found.extend(self.base_files(b, |f| wild(&spec, &f.name))?);
                }
            }
        }
        if found.is_empty() {
            self.sprint("|U1File not found.")?;
        }
        Ok(found)
    }

    // ---- looking at files ---------------------------------------------------

    /// `I` in a listing (fileinfo).
    fn file_info(&mut self, (b, f): &(usize, FileRec)) -> io::Result<()> {
        let fb = self.cfg().fbases[*b].clone();
        self.nl()?;
        self.out_field("Name", &f.name)?;
        self.out_field("Desc", &f.desc)?;
        for l in &f.ext {
            self.out_field("", l)?;
        }
        self.out_field("Size", &format!("{} bytes", f.size))?;
        self.out_field("Pts", &f.points.to_string())?;
        self.out_field("D/Ls", &f.downloads.to_string())?;
        if self.acs(&fb.name_acs) {
            self.out_field("ULer", &f.owner)?;
        }
        self.out_field("Date", &f.date)?;
        let on_disk = file_path(&fb, &f.name).exists();
        self.out_field("Stat", if f.unvalidated { "Not validated" } else if on_disk { "Available" } else { "Offline" })?;
        Ok(())
    }

    /// `T` in a listing: show a text file.
    fn type_file(&mut self, (b, f): &(usize, FileRec)) -> io::Result<()> {
        let fb = self.cfg().fbases[*b].clone();
        let Ok(bytes) = std::fs::read(file_path(&fb, &f.name)) else { return self.sprint("|12File is not available.") };
        let text: String = bytes.iter().map(|&b| b as char).collect();
        self.nl()?;
        for l in text.lines() {
            self.out(l)?;
            self.nl()?;
        }
        Ok(())
    }

    /// `FV` (lfii) / `V` in a listing: an archive's contents.
    fn view_file(&mut self, (b, f): &(usize, FileRec)) -> io::Result<()> {
        let fb = self.cfg().fbases[*b].clone();
        let p = file_path(&fb, &f.name);
        if !p.exists() {
            return self.sprint("|12File is not available.");
        }
        let lower = f.name.to_lowercase();
        // ponytail: zip via unzip(1) only; LZH/ARJ viewers (LZHVIEW.PAS) not ported
        if !lower.ends_with(".zip") {
            return self.sprint("|U1Can't view the inside of that kind of file.");
        }
        let out = Command::new("unzip").arg("-l").arg(&p).output();
        let Ok(out) = out else { return self.sprint("|12unzip is not available.") };
        self.nl()?;
        for l in String::from_utf8_lossy(&out.stdout).lines() {
            self.out(l)?;
            self.nl()?;
        }
        Ok(())
    }

    pub fn view_cmd(&mut self) -> io::Result<()> {
        if let Some(it) = self.ask_file("View which file", true)?.into_iter().next() {
            self.view_file(&it)?;
        }
        Ok(())
    }

    /// `FY`: your file statistics.
    pub fn file_stats(&mut self) -> io::Result<()> {
        if self.printf("filestat")? {
            return Ok(());
        }
        let u = &self.user;
        let s = format!(
            "|NL|U1Uploads: |U3{}|U1 files, |U3{}|U1k   Downloads: |U3{}|U1 files, |U3{}|U1k   File points: |U3{}",
            u.uploads, u.uk, u.downloads, u.dk, u.file_points
        );
        self.sprint(&s)
    }

    // ---- downloads ----------------------------------------------------------

    fn free_dl(&self, fb: &FBase) -> bool {
        self.is_sysop() || fb.no_ratio
    }

    /// Ratio check for `n` more files of `k` kilobytes (checkfileratio).
    // ponytail: one ratio for everyone, not Impulse's per-SL tables
    fn ratio_ok(&self, n: u32, k: u64) -> bool {
        let c = self.cfg();
        let u = &self.user;
        if self.is_sysop() || u.ac.contains('1') {
            return true;
        }
        let files = c.dl_ratio == 0 || u.downloads + n <= (u.uploads + 1) * c.dl_ratio;
        let kb = c.dlk_ratio == 0 || u.dk + k <= (u.uk + 1) * c.dlk_ratio as u64;
        files && kb
    }

    /// Can this file be downloaded at all (okdl)?
    fn ok_dl(&mut self, (b, f): &(usize, FileRec)) -> io::Result<bool> {
        let fb = self.cfg().fbases[*b].clone();
        let why = if f.unvalidated && !self.is_sysop() {
            "That file has not been validated yet."
        } else if !file_path(&fb, &f.name).exists() {
            "That file is not available."
        } else if self.cfg().use_file_points && !self.free_dl(&fb) && !self.user.ac.contains('3') && self.user.file_points < f.points {
            "You don't have enough file points."
        } else {
            return Ok(true);
        };
        self.sprompt("|12")?;
        self.out(&format!("{}: {why}", f.name))?;
        self.nl()?;
        Ok(false)
    }

    pub fn choose_protocol(&mut self, upload: bool, batch: bool) -> io::Result<Option<Protocol>> {
        let list: Vec<Protocol> = self
            .cfg()
            .protocols
            .iter()
            .filter(|p| self.acs(&p.acs) && !(if upload { &p.ul_cmd } else { &p.dl_cmd }).is_empty() && (p.batch || !batch))
            .cloned()
            .collect();
        if list.is_empty() {
            self.sprint("|NL|12No transfer protocols are available.")?;
            return Ok(None);
        }
        self.nl()?;
        let mut keys = String::from("Q\r");
        for p in &list {
            let k = p.key.chars().next().unwrap_or(' ').to_ascii_uppercase();
            keys.push(k);
            self.sprint(&format!("|U2[|U3{k}|U2] |U1{}", p.name))?;
        }
        self.sprint("|U2[|U3Q|U2] |U1Quit")?;
        self.sprompt("|NL|U1Protocol: |U3")?;
        let k = self.onek(&keys)?;
        Ok(list.into_iter().find(|p| p.key.chars().next().map(|c| c.to_ascii_uppercase()) == Some(k)))
    }

    /// Run a protocol command in `cwd`; true if it exited cleanly.
    pub fn run_protocol(&mut self, cmd: &str, files: &[PathBuf], cwd: &Path) -> io::Result<bool> {
        let list: Vec<String> = files.iter().map(|p| shell_quote(&std::path::absolute(p).unwrap_or(p.clone()))).collect();
        let line = cmd.replace("%F", &list.join(" "));
        self.log(&format!("Transfer: {line}"));
        self.sprint("|NL|U1Start your transfer now. |U2(Ctrl-X several times aborts)")?;
        let st = self.t.run_external(Command::new("sh").arg("-c").arg(&line).current_dir(cwd), false);
        match st {
            Ok(s) => Ok(s.success()),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(e),
            Err(e) => {
                self.log(&format!("Transfer failed: {e}"));
                Ok(false)
            }
        }
    }

    /// Send files and charge for them (dlx / batchdl).
    pub fn download(&mut self, items: Items) -> io::Result<()> {
        let mut ok = Items::new();
        for it in items {
            if self.ok_dl(&it)? {
                ok.push(it);
            }
        }
        if ok.is_empty() {
            return Ok(());
        }
        let cfg = self.cfg();
        let charged: Vec<&(usize, FileRec)> = ok.iter().filter(|(b, _)| !self.free_dl(&cfg.fbases[*b])).collect();
        let k: u64 = charged.iter().map(|(_, f)| f.size / 1024).sum();
        if !self.ratio_ok(charged.len() as u32, k) {
            return self.sprint("|NL|12Your upload/download ratio is out of balance. Upload something first!");
        }
        let total: u64 = ok.iter().map(|(_, f)| f.size).sum();
        self.sprint(&format!("|NL|U1Sending |U3{}|U1 file(s), |U3{}|U1 bytes.", ok.len(), total))?;
        let Some(p) = self.choose_protocol(false, ok.len() > 1)? else { return Ok(()) };
        let paths: Vec<PathBuf> = ok.iter().map(|(b, f)| file_path(&cfg.fbases[*b], &f.name)).collect();
        let dir = cfg.data_path.clone();
        if !self.run_protocol(&p.dl_cmd, &paths, &dir)? {
            self.log("Download aborted");
            return self.sprint("|NL|12Transfer aborted.");
        }
        for (b, f) in &ok {
            let fb = &cfg.fbases[*b];
            if !self.free_dl(fb) {
                self.user.downloads += 1;
                self.user.dk += f.size / 1024;
                if cfg.use_file_points && !self.user.ac.contains('3') {
                    self.user.file_points -= f.points;
                }
            }
            let mut db = self.sh.db();
            let z = db.sys.zlog();
            z.downloads += 1;
            z.dk += f.size / 1024;
            let mut v = db.files(&fb.file)?;
            if let Some(x) = v.iter_mut().find(|x| x.name == f.name) {
                x.downloads += 1;
            }
            db.put_files(&fb.file, &v)?;
            db.save()?;
            drop(db);
            self.log(&format!("Downloaded {} from {}", f.name, fb.name));
        }
        self.batch.retain(|(b, n)| !ok.iter().any(|(ob, of)| ob == b && of.name == *n));
        self.save_user();
        self.sprint("|NL|U1Transfer complete.")
    }

    /// `FD` (idl): download by name, or the batch queue if it has files.
    pub fn download_cmd(&mut self, data: &str) -> io::Result<()> {
        if !self.batch.is_empty() {
            return self.batch_download();
        }
        let items = if data.trim().is_empty() {
            self.ask_file("Download which file", true)?
        } else {
            let spec = data.trim().to_string();
            self.base_files(self.fbase, |f| wild(&spec, &f.name))?
        };
        if items.is_empty() {
            return Ok(());
        }
        self.download(items)
    }

    // ---- batch queue (FILE6/FILE12.PAS) ---------------------------------------

    fn batch_add(&mut self, it: &(usize, FileRec)) -> io::Result<()> {
        if self.batch.iter().any(|(b, n)| *b == it.0 && *n == it.1.name) {
            return self.sprint("|U1Already in the batch queue.");
        }
        if self.batch.len() >= 50 {
            return self.sprint("|12The batch queue is full.");
        }
        if !self.ok_dl(it)? {
            return Ok(());
        }
        self.batch.push((it.0, it.1.name.clone()));
        let s = self.cfg().strings.file_added.clone();
        self.sprint(&s)
    }

    fn batch_items(&self) -> io::Result<Items> {
        let mut v = Items::new();
        for (b, name) in &self.batch {
            let file = self.cfg().fbases[*b].file.clone();
            if let Some(f) = self.sh.db().files(&file)?.into_iter().find(|f| f.name == *name) {
                v.push((*b, f));
            }
        }
        Ok(v)
    }

    /// `BL`
    pub fn batch_list(&mut self) -> io::Result<()> {
        let items = self.batch_items()?;
        if items.is_empty() {
            return self.sprint("|NL|U1The batch queue is empty.");
        }
        self.sprint("|NL|U3  # |U1File name     Size    Base")?;
        for (i, (b, f)) in items.iter().enumerate() {
            let base = self.cfg().fbases[*b].name.clone();
            self.sprompt(&format!("|U2{:>3} |U3", i + 1))?;
            self.out(&format!("{:<12} {:>7}  {base}", f.name, size_str(f.size)))?;
            self.nl()?;
        }
        let total: u64 = items.iter().map(|(_, f)| f.size).sum();
        self.sprint(&format!("|U1Total: |U3{}|U1 files, |U3{}|U1 bytes.", items.len(), total))
    }

    /// `BR`
    pub fn batch_remove(&mut self) -> io::Result<()> {
        self.batch_list()?;
        if self.batch.is_empty() {
            return Ok(());
        }
        self.sprompt("|NL|U1Remove which file? |U3")?;
        if let Ok(n @ 1..) = self.input(3)?.parse::<usize>()
            && n <= self.batch.len() {
                self.batch.remove(n - 1);
                self.sprint("|U1Removed.")?;
            }
        Ok(())
    }

    /// `BC`
    pub fn batch_clear(&mut self) -> io::Result<()> {
        self.batch.clear();
        self.sprint("|NL|U1Batch queue cleared.")
    }

    /// `BD`
    pub fn batch_download(&mut self) -> io::Result<()> {
        let items = self.batch_items()?;
        if items.is_empty() {
            return self.sprint("|NL|U1The batch queue is empty.");
        }
        self.download(items)
    }

    /// `B?`
    pub fn batch_info(&mut self) -> io::Result<()> {
        let items = self.batch_items()?;
        let total: u64 = items.iter().map(|(_, f)| f.size).sum();
        self.sprint(&format!("|NL|U1Batch queue: |U3{}|U1 files, |U3{}|U1 bytes.", items.len(), total))
    }

    // ---- uploads --------------------------------------------------------------

    /// Is `name` already in any file base (searchfordups)?
    fn is_dup(&self, name: &str) -> bool {
        let c = self.cfg();
        let db = self.sh.db();
        c.fbases.iter().any(|fb| db.files(&fb.file).unwrap_or_default().iter().any(|f| f.name.eq_ignore_ascii_case(name)))
    }

    /// FILE_ID.DIZ from a zip: (description, extended lines).
    fn zip_diz(p: &Path) -> Option<(String, Vec<String>)> {
        let out = Command::new("unzip").args(["-p", "-C"]).arg(p).arg("FILE_ID.DIZ").output().ok()?;
        let text: String = out.stdout.iter().map(|&b| b as char).collect();
        let mut lines = text.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.trim().is_empty());
        let first = lines.next()?;
        Some((first, lines.take(19).collect()))
    }

    /// `FU` / `BU` (batchul): receive files into the current base.
    pub fn upload(&mut self) -> io::Result<()> {
        let b = self.fbase;
        let Some(fb) = self.cfg().fbases.get(b).cloned() else { return self.sprint("|NL|12No file base selected.") };
        if !self.acs(&fb.ul_acs) || !self.fbase_ok(b)? {
            return self.sprint("|NL|12You can't upload to this base.");
        }
        if fb.max_files > 0 && self.sh.db().files(&fb.file)?.len() >= fb.max_files {
            return self.sprint("|NL|12This file base is full.");
        }
        let Some(p) = self.choose_protocol(true, false)? else { return Ok(()) };
        let tmp = self.cfg().data_path.join("upload").join(self.node.to_string());
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        self.set_doing("Uploading");
        let ok = self.run_protocol(&p.ul_cmd, &[], &tmp)?;
        let mut got: Vec<PathBuf> = std::fs::read_dir(&tmp)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
        got.sort();
        if got.is_empty() {
            return self.sprint(if ok { "|NL|U1No files received." } else { "|NL|12Transfer aborted." });
        }
        std::fs::create_dir_all(ul_dir(&fb))?;
        for path in got {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if self.is_dup(&name) || find_ci(ul_dir(&fb), &name).is_some() {
                self.sprint(&format!("|12{name} is already on the system; removed."))?;
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let size = std::fs::metadata(&path)?.len();
            let (desc, ext) = match Self::zip_diz(&path) {
                Some(d) => d,
                None => {
                    self.sprompt("|NL|U1Description for |U3")?;
                    self.out(&name)?;
                    self.sprompt("|U1: |U3")?;
                    (self.input(60)?, Vec::new())
                }
            };
            let dest = ul_dir(&fb).join(&name);
            if std::fs::rename(&path, &dest).is_err() {
                std::fs::copy(&path, &dest)?;
                std::fs::remove_file(&path)?;
            }
            let cfg = self.cfg();
            let f = FileRec {
                name: name.clone(),
                desc: if desc.is_empty() { "(No description)".into() } else { desc },
                ext,
                points: i32::from(cfg.use_file_points),
                size,
                owner: self.user.name.clone(),
                owner_num: self.unum,
                date: today(),
                unvalidated: cfg.validate_uploads && !self.is_sysop(),
                ..FileRec::default()
            };
            {
                let mut db = self.sh.db();
                let mut v = db.files(&fb.file)?;
                v.push(f);
                db.put_files(&fb.file, &v)?;
                let z = db.sys.zlog();
                z.uploads += 1;
                z.uk += size / 1024;
                db.save()?;
            }
            self.user.uploads += 1;
            self.user.uk += size / 1024;
            if cfg.use_file_points {
                self.user.file_points += cfg.ul_points;
            }
            self.log(&format!("Uploaded {name} to {}", fb.name));
            self.sprompt("|U1Thanks for uploading |U3")?;
            self.out(&name)?;
            self.sprint("|U1!")?;
        }
        self.save_user();
        Ok(())
    }

    /// `FR` (remove): take one of your files out of the current base.
    pub fn remove_file(&mut self) -> io::Result<()> {
        let b = self.fbase;
        let Some(fb) = self.cfg().fbases.get(b).cloned() else { return Ok(()) };
        self.sprompt("|NL|U1Remove which file: |U3")?;
        let name = self.input_caps(12)?;
        let mut v = self.sh.db().files(&fb.file)?;
        let Some(i) = v.iter().position(|f| f.name.eq_ignore_ascii_case(&name)) else { return self.sprint("|U1File not found.") };
        let f = v[i].clone();
        if f.owner_num != self.unum && !self.is_sysop() {
            return self.sprint("|12You can't remove that file - you didn't upload it.");
        }
        if !self.pynq(&format!("|U1Remove |U3{}|U1? ", f.name), false)? {
            return Ok(());
        }
        v.remove(i);
        self.sh.db().put_files(&fb.file, &v)?;
        if self.is_sysop() && self.pynq("|U1Delete the file from disk too? ", false)? {
            let _ = std::fs::remove_file(file_path(&fb, &f.name));
        }
        // the uploader loses the credit
        let owner = self.sh.db().users.get(f.owner_num.wrapping_sub(1)).cloned();
        if f.owner_num == self.unum {
            self.user.uploads = self.user.uploads.saturating_sub(1);
            self.user.uk = self.user.uk.saturating_sub(f.size / 1024);
            self.save_user();
        } else if let Some(mut u) = owner {
            u.uploads = u.uploads.saturating_sub(1);
            u.uk = u.uk.saturating_sub(f.size / 1024);
            self.store_user(f.owner_num, u)?;
        }
        self.log(&format!("Removed {} from {}", f.name, fb.name));
        self.sprint("|U1Removed.")
    }

    // ---- SysOp (FILE5/9/10/13/14.PAS) ----------------------------------------

    /// `%F` (dlboardedit)
    pub fn fbase_editor(&mut self) -> io::Result<()> {
        self.log("* File base edit");
        let fb = self.cfg().fbases.clone();
        if let Some(new) = self.edit_vec("File bases", &fb, &[])? {
            self.sh.edit_cfg(|c| c.fbases = new)?;
            self.fbase = self.fbase.min(self.cfg().fbases.len().saturating_sub(1));
        }
        Ok(())
    }

    fn cur_fbase(&mut self) -> io::Result<Option<FBase>> {
        let fb = self.cfg().fbases.get(self.fbase).cloned();
        if fb.is_none() {
            self.sprint("|NL|12No file base selected.")?;
        }
        Ok(fb)
    }

    /// `%1` (editfiles): the current base's file records.
    pub fn edit_files(&mut self) -> io::Result<()> {
        let Some(fb) = self.cur_fbase()? else { return Ok(()) };
        self.log("* Edited files");
        let v = self.sh.db().files(&fb.file)?;
        if let Some(new) = self.edit_vec(&format!("Files in {}", fb.name), &v, &[])? {
            self.sh.db().put_files(&fb.file, &new)?;
        }
        Ok(())
    }

    /// `%2` (sort): by name, this base or all of them.
    pub fn sort_files(&mut self) -> io::Result<()> {
        self.log("* Sorted files");
        let all = self.pynq("|NL|U1Sort all file bases? ", false)?;
        let c = self.cfg();
        let bases: Vec<&FBase> = if all { c.fbases.iter().collect() } else { c.fbases.get(self.fbase).into_iter().collect() };
        for fb in bases {
            let db = self.sh.db();
            let mut v = db.files(&fb.file)?;
            v.sort_by_key(|f| f.name.to_uppercase());
            db.put_files(&fb.file, &v)?;
            drop(db);
            self.sprint(&format!("|U1Sorted |U3{}|U1 ({} files).", fb.name, v.len()))?;
        }
        Ok(())
    }

    /// `%4` (unlisted_download): send any file on disk.
    pub fn unlisted_download(&mut self, data: &str) -> io::Result<()> {
        let path = if data.trim().is_empty() {
            self.sprompt("|NL|U1Path of file to send: |U3")?;
            self.input(78)?
        } else {
            data.trim().to_string()
        };
        let p = PathBuf::from(path);
        if !p.is_file() {
            return self.sprint("|12File not found.");
        }
        self.log(&format!("* Unlisted download of {}", p.display()));
        let Some(pr) = self.choose_protocol(false, false)? else { return Ok(()) };
        let dir = self.cfg().data_path.clone();
        let ok = self.run_protocol(&pr.dl_cmd, &[p], &dir)?;
        self.sprint(if ok { "|NL|U1Transfer complete." } else { "|NL|12Transfer aborted." })
    }

    /// `%5` (move): a file to another base, record and all.
    pub fn move_file(&mut self) -> io::Result<()> {
        let Some(fb) = self.cur_fbase()? else { return Ok(()) };
        let Some((_, f)) = self.ask_file("Move which file", false)?.into_iter().next() else { return Ok(()) };
        let list = self.list_fbases()?;
        self.sprompt("|NL|U1Move to which base? |U3")?;
        let Ok(n @ 1..) = self.input(3)?.parse::<usize>() else { return Ok(()) };
        let Some(to) = list.get(n - 1).map(|&b| self.cfg().fbases[b].clone()) else { return Ok(()) };
        if to.file == fb.file {
            return Ok(());
        }
        let src = file_path(&fb, &f.name);
        if src.exists() {
            std::fs::create_dir_all(&to.dl_path)?;
            let dst = to.dl_path.join(&f.name);
            if std::fs::rename(&src, &dst).is_err() {
                std::fs::copy(&src, &dst)?;
                std::fs::remove_file(&src)?;
            }
        }
        let db = self.sh.db();
        let mut v = db.files(&fb.file)?;
        v.retain(|x| x.name != f.name);
        db.put_files(&fb.file, &v)?;
        let mut w = db.files(&to.file)?;
        w.push(f.clone());
        db.put_files(&to.file, &w)?;
        drop(db);
        self.log(&format!("* Moved {} from {} to {}", f.name, fb.name, to.name));
        self.sprint(&format!("|U1Moved |U3{}|U1 to |U3{}|U1.", f.name, to.name))
    }

    /// `%6` (uploadall): add files sitting in a base's directory unlisted.
    pub fn upload_all(&mut self) -> io::Result<()> {
        let all = self.pynq("|NL|U1Scan all file bases? ", false)?;
        let c = self.cfg();
        let bases: Vec<FBase> = if all { c.fbases.clone() } else { c.fbases.get(self.fbase).cloned().into_iter().collect() };
        let mut add_all = false;
        'bases: for fb in bases {
            self.sprint(&format!("|NL|U1Scanning |U3{}|U1 ({})", fb.name, fb.dl_path.display()))?;
            let Ok(rd) = std::fs::read_dir(&fb.dl_path) else { continue };
            let mut names: Vec<String> = rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
            names.sort();
            for name in names {
                let mut v = self.sh.db().files(&fb.file)?;
                if v.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
                    continue;
                }
                let path = fb.dl_path.join(&name);
                let size = std::fs::metadata(&path)?.len();
                self.sprompt("|NL|U3")?;
                self.out(&format!("{name} {}k", size / 1024))?;
                self.nl()?;
                let k = if add_all {
                    'A'
                } else {
                    self.sprompt("|U3Add file? |U1[A/Y/N/D/J/Q] |U2")?;
                    self.onek("YNDJQA\r")?
                };
                let (desc, ext) = match k {
                    'Y' | '\r' | 'A' => {
                        add_all |= k == 'A';
                        match Self::zip_diz(&path) {
                            Some(d) => d,
                            None if add_all => ("(No description)".into(), Vec::new()),
                            None => {
                                self.sprompt("|U2Enter description: |U3")?;
                                (self.input(55)?, Vec::new())
                            }
                        }
                    }
                    'D' => {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    'J' => continue 'bases,
                    'Q' => return Ok(()),
                    _ => continue,
                };
                v.push(FileRec {
                    name,
                    desc,
                    ext,
                    size,
                    owner: self.user.name.clone(),
                    owner_num: self.unum,
                    date: today(),
                    ..FileRec::default()
                });
                self.sh.db().put_files(&fb.file, &v)?;
            }
        }
        Ok(())
    }

    /// `%7` (validatefiles)
    pub fn validate_files(&mut self) -> io::Result<()> {
        let c = self.cfg();
        let mut any = false;
        for fb in &c.fbases {
            let mut v = self.sh.db().files(&fb.file)?;
            let mut changed = false;
            let mut i = 0;
            while i < v.len() {
                if !v[i].unvalidated {
                    i += 1;
                    continue;
                }
                any = true;
                let f = v[i].clone();
                self.print_file(i + 1, &f, fb)?;
                self.sprompt("|U1Validate? |U2(|U3Y|U2)es (|U3N|U2)o (|U3D|U2)elete (|U3Q|U2)uit: |U3")?;
                match self.onek("YNDQ")? {
                    'Y' => {
                        v[i].unvalidated = false;
                        changed = true;
                    }
                    'D' => {
                        let _ = std::fs::remove_file(file_path(fb, &f.name));
                        v.remove(i);
                        changed = true;
                        continue;
                    }
                    'Q' => {
                        if changed {
                            self.sh.db().put_files(&fb.file, &v)?;
                        }
                        return Ok(());
                    }
                    _ => {}
                }
                i += 1;
            }
            if changed {
                self.sh.db().put_files(&fb.file, &v)?;
            }
        }
        if !any {
            self.sprint("|NL|U1No unvalidated files.")?;
        }
        Ok(())
    }

    /// `%8` (addgifspecs): prefix GIF descriptions with "(WxH,Nc)".
    pub fn add_gif_specs(&mut self) -> io::Result<()> {
        let Some(fb) = self.cur_fbase()? else { return Ok(()) };
        let mut v = self.sh.db().files(&fb.file)?;
        let mut n = 0;
        for f in v.iter_mut().filter(|f| f.name.to_lowercase().ends_with(".gif") && !f.desc.starts_with('(')) {
            let Ok(b) = std::fs::read(file_path(&fb, &f.name)) else { continue };
            if let Some((x, y, c)) = gif_specs(&b) {
                f.desc = format!("({x}x{y},{c}c) {}", f.desc).chars().take(54).collect();
                n += 1;
            }
        }
        self.sh.db().put_files(&fb.file, &v)?;
        self.sprint(&format!("|NL|U1Added GifSpecs to |U3{n}|U1 file(s)."))
    }

    /// `%9` (packmessagebases): prune bases to their message limits.
    pub fn pack_boards(&mut self) -> io::Result<()> {
        self.sprint("|NL|U1Packing message bases...")?;
        for b in self.cfg().boards.iter() {
            let n = self.sh.db().prune(&b.file, b.max_msgs)?;
            self.sprint(&format!("|U3{}|U1: removed |U3{n}|U1 message(s).", b.name))?;
        }
        Ok(())
    }

    /// `%$` / `%%` (dirf): raw directory of the current base.
    pub fn raw_dir(&mut self, expanded: bool) -> io::Result<()> {
        let Some(fb) = self.cur_fbase()? else { return Ok(()) };
        self.sprint("|NL|U1Raw directory.")?;
        let spec = self.ask_spec("")?;
        let Ok(rd) = std::fs::read_dir(&fb.dl_path) else { return self.sprint("|12Bad download path.") };
        let mut ents: Vec<_> = rd.flatten().filter(|e| wild(&spec, &e.file_name().to_string_lossy())).collect();
        ents.sort_by_key(|e| e.file_name());
        self.nl()?;
        let mut line = String::new();
        for e in &ents {
            let name = e.file_name().to_string_lossy().to_string();
            let md = e.metadata().ok();
            if expanded {
                let size = match &md {
                    Some(m) if m.is_dir() => "<DIR>".to_string(),
                    Some(m) => m.len().to_string(),
                    None => "?".into(),
                };
                let when = md.and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%m-%d-%y  %H:%M").to_string()).unwrap_or_default();
                self.out(&format!("{name:<20} {size:>10}  {when}"))?;
                self.nl()?;
            } else {
                line.push_str(&format!("{name:<16}"));
                if line.len() >= 64 {
                    self.out(&line)?;
                    self.nl()?;
                    line.clear();
                }
            }
        }
        if !line.is_empty() {
            self.out(&line)?;
            self.nl()?;
        }
        self.sprint(&format!("|U1{} file(s).", ents.len()))
    }
}

/// Width, height and colour count from a GIF header (getgifspecs).
fn gif_specs(b: &[u8]) -> Option<(u16, u16, u32)> {
    if b.len() < 11 || !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return None;
    }
    Some((u16::from_le_bytes([b[6], b[7]]), u16::from_le_bytes([b[8], b[9]]), 1 << ((b[10] & 7) + 1)))
}

#[cfg(test)]
mod tests {
    use super::wild;

    #[test]
    fn wildcards() {
        assert!(wild("*.*", "README"));
        assert!(wild("", "x.zip"));
        assert!(wild("*.ZIP", "foo.zip"));
        assert!(!wild("*.ZIP", "foo.arj"));
        assert!(wild("F?O.*", "FOO"));
        assert!(wild("IMP*", "IMP71.ZIP"));
        assert!(!wild("IMP?", "IMP71"));
    }

    #[test]
    fn gif() {
        let mut h = b"GIF89a".to_vec();
        h.extend_from_slice(&[0x40, 0x01, 0xc8, 0x00, 0xf7]);
        assert_eq!(super::gif_specs(&h), Some((320, 200, 256)));
        assert_eq!(super::gif_specs(b"PK\x03\x04......."), None);
    }
}

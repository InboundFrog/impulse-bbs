//! `impulse.toml`: the parts of STATUS.DAT (systatrec) and the string table
//! (fstringrec) that this port uses. Every field has a default.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub bbs_name: String,
    pub sysop_name: String,
    pub listen: String,
    pub max_nodes: usize,
    pub data_path: PathBuf,
    pub menu_path: PathBuf,
    pub text_path: PathBuf,
    pub start_menu: String,
    pub closed_system: bool,
    pub new_user_password: String,
    pub new_sl: u8,
    pub new_dsl: u8,
    pub new_ar: String,
    pub max_logon_tries: u32,
    pub idle_timeout_minutes: u64,
    pub minutes_per_day: i64,
    /// Users matching this ACS are SysOps: no time limit, sysop-only commands.
    pub sysop_acs: String,
    pub max_message_lines: usize,
    pub boards: Vec<Board>,
    pub strings: Strings,
}

#[derive(Deserialize, Clone)]
pub struct Board {
    pub name: String,
    /// Storage name: `<data_path>/msgs/<file>.json`.
    pub file: String,
    #[serde(default)]
    pub acs: String,
    #[serde(default)]
    pub post_acs: String,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Strings {
    pub enter_handle: String,
    pub enter_pw: String,
    pub name_not_found: String,
    pub logon_new: String,
    pub wrong_password: String,
    pub new_user_pw: String,
    pub pause: String,
    pub aborted: String,
    pub msg_title: String,
    pub msg_send_to: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bbs_name: "Impulse BBS".into(),
            sysop_name: "SysOp".into(),
            listen: "0.0.0.0:2323".into(),
            max_nodes: 8,
            data_path: "data".into(),
            menu_path: "menus".into(),
            text_path: "text".into(),
            start_menu: "MAIN".into(),
            closed_system: false,
            new_user_password: String::new(),
            new_sl: 20,
            new_dsl: 20,
            new_ar: String::new(),
            max_logon_tries: 4,
            idle_timeout_minutes: 10,
            minutes_per_day: 60,
            sysop_acs: "s255".into(),
            max_message_lines: 100,
            boards: vec![Board { name: "General".into(), file: "general".into(), acs: String::new(), post_acs: "s20".into() }],
            strings: Strings::default(),
        }
    }
}

impl Default for Strings {
    fn default() -> Self {
        Strings {
            enter_handle: "|NL|U1Handle or user number |U2(|U3new|U2)|U1: |U3".into(),
            enter_pw: "|U1Password: |U3".into(),
            name_not_found: "|NL|12Name not found in user list.".into(),
            logon_new: "|U1Log on as a new user? ".into(),
            wrong_password: "|12Wrong password.".into(),
            new_user_pw: "|U1New user password: |U3".into(),
            pause: "|U2[|U3pause|U2]".into(),
            aborted: "|12Aborted.".into(),
            msg_title: "|U1Subject: |U3".into(),
            msg_send_to: "|U1Send to: |U3".into(),
        }
    }
}

pub fn load(path: &str) -> Result<Config, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).map_err(|e| format!("{path}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("{path} not found, using defaults");
            Ok(Config::default())
        }
        Err(e) => Err(format!("{path}: {e}")),
    }
}

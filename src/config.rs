//! `impulse.toml`: the parts of STATUS.DAT (systatrec) and the string table
//! (fstringrec) that this port uses. Every field has a default.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub bbs_name: String,
    pub sysop_name: String,
    /// Accept telnet callers on `listen`.
    pub telnet: bool,
    pub listen: String,
    /// Serial lines answered, one node each while a caller is on.
    pub serial: Vec<SerialPort>,
    pub max_nodes: usize,
    pub data_path: PathBuf,
    pub menu_path: PathBuf,
    pub text_path: PathBuf,
    pub start_menu: String,
    pub closed_system: bool,
    pub new_user_password: String,
    /// Matrix logon: callers start at `shuttle_menu` and use O1/O2/O3 to
    /// log on, apply or check their validation.
    pub shuttle_log: bool,
    pub shuttle_menu: String,
    /// Asked by O1 when set; O3 reveals it to validated users.
    pub shuttle_pw: String,
    pub new_sl: u8,
    pub new_dsl: u8,
    pub new_ar: String,
    pub max_logon_tries: u32,
    /// Let users at or below `new_sl` log on; 7.1 hangs up on them until a
    /// SysOp (or auto-validation / NUV) raises them.
    pub unvalidated_logon: bool,
    /// After the password, ask for the last 4 digits of the phone number (phonepw).
    pub phone_pw: bool,
    /// Users matching this ACS must also give `sysop_password` at logon (spw).
    pub logon_pw_acs: String,
    /// Serial callers below this rate are slow (lock300/b300 times; 0 = off):
    /// `slow_lockout` refuses them, `slow_hours` ("HH:MM-HH:MM") limits when
    /// they may call. Display files NO9600, NO9600H, YES9600H.
    pub slow_baud: u32,
    pub slow_lockout: bool,
    pub slow_hours: String,
    pub idle_timeout_minutes: u64,
    pub minutes_per_day: i64,
    /// Users matching this ACS are SysOps: no time limit, sysop-only commands.
    pub sysop_acs: String,
    /// Trap every caller's keystrokes (globaltrap).
    pub global_trap: bool,
    /// Record every SysOp chat (autochatopen).
    pub auto_chat_capture: bool,
    /// Asked before every `%` SysOp command when set (checkpw).
    pub sysop_password: String,
    pub max_message_lines: usize,
    /// Message SysOps: validate, move, extract and make messages permanent.
    pub msop_acs: String,
    /// See who posted anonymously, in public and in e-mail.
    pub anon_pub_read: String,
    pub anon_priv_read: String,
    /// Post anonymously on any base, and send anonymous e-mail.
    pub anon_pub_post: String,
    pub anon_priv_post: String,
    /// Most minutes a user may keep in the time bank (0 disables it).
    pub max_time_bank: i64,
    /// Bulletin files are `<prefix><n>` (BULLETIN lists them).
    pub bulletin_prefix: String,
    /// Levels given by auto-validation (`OA`) and the user editor.
    pub auto_sl: u8,
    pub auto_dsl: u8,
    pub auto_ar: String,
    pub auto_ac: String,
    /// New users are validated straight away (autov).
    pub auto_validate_new: bool,
    /// Otherwise put them up for new user voting, decided by this many votes.
    pub use_nuv: bool,
    pub nuv_yes: u32,
    pub nuv_no: u32,
    /// New-user questions in the order asked (NUORDER.DAT, MAKENUO.PAS):
    /// real_name city phone birthday sex ansi.
    pub new_user_order: Vec<String>,
    /// Up to four prompt choices; empty means everyone gets the menu prompts.
    pub prompts: Vec<Prompt>,
    pub boards: Vec<Board>,
    pub fbases: Vec<FBase>,
    /// External transfer protocols (PROTOCOL.DAT).
    pub protocols: Vec<Protocol>,
    pub events: Vec<Event>,
    /// QWK packet name (BBSID, up to 8 chars) and message limit per packet.
    pub qwk_id: String,
    pub qwk_max: usize,
    /// Files and K a user may download per file / K uploaded (0 = no ratio).
    pub dl_ratio: u32,
    pub dlk_ratio: u32,
    /// Per-SL limits (callallow/timeallow/dlratio/dlkratio[sl]): the entry
    /// with the highest `min_sl` at or below the user's SL applies.
    #[serde(alias = "sl_ratios")]
    pub sl_limits: Vec<SlLimit>,
    /// Users matching this ACS skip ratios (nodlratio).
    pub no_ratio_acs: String,
    /// Charge file points for downloads, and credit this many per upload.
    pub use_file_points: bool,
    pub ul_points: i32,
    /// Uploads stay hidden until a SysOp validates them (`%7`).
    pub validate_uploads: bool,
    /// Upload checks on .ZIP files (ULCHECK.PAS): a comment file put on with
    /// `zip -z` and a file (an ad, say) added to each; empty = skip.
    /// Names listed in `<text_path>/REMOVE.TXT` are deleted from uploads.
    pub ul_comment: String,
    pub ul_add: String,
    /// Message and file conferences (MCONF.DAT / FCONF.DAT), numbered from 1.
    pub msg_confs: Vec<Conf>,
    pub file_confs: Vec<Conf>,
    pub strings: Strings,
}

/// A file base (ulrec in UPLOADS.DAT).
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct FBase {
    pub name: String,
    /// Storage name: `<data_path>/files/<file>.json`.
    pub file: String,
    pub dl_path: PathBuf,
    /// Where uploads go; empty means `dl_path`.
    pub ul_path: PathBuf,
    pub acs: String,
    pub ul_acs: String,
    /// Needed to see who uploaded a file.
    pub name_acs: String,
    pub password: String,
    /// 0 = no limit.
    pub max_files: usize,
    /// Downloads here don't count against ratios or points.
    pub no_ratio: bool,
    pub conf: usize,
}

/// External protocol (protrec). In the commands `%F` is the files to send,
/// quoted and space-separated. Receives run in an empty directory.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Protocol {
    pub key: String,
    pub name: String,
    pub acs: String,
    pub dl_cmd: String,
    pub ul_cmd: String,
    /// Can send several files at once.
    pub batch: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Event {
    pub active: bool,
    pub description: String,
    /// A: only `data` ACS may log on during the window; D/E: run `data` as a command.
    pub kind: String,
    pub data: String,
    /// "HH:MM"
    pub time: String,
    /// Minutes; how long an `A` window lasts.
    pub duration: u32,
    /// Weekdays, 0 = Sunday; empty = every day.
    pub days: Vec<u8>,
    /// Run monthly on this day instead, 0 = not monthly.
    pub day_of_month: u8,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct SlLimit {
    pub min_sl: u8,
    /// Download ratios; 0 = none.
    pub files: u32,
    pub kb: u32,
    /// Calls a day; 0 = no limit.
    pub calls: u32,
    /// Minutes a day; 0 = `minutes_per_day`.
    pub minutes: i64,
}

impl Config {
    /// The `sl_limits` entry for `sl`, if any applies.
    pub fn sl_limit(&self, sl: u8) -> Option<&SlLimit> {
        self.sl_limits.iter().filter(|r| r.min_sl <= sl).max_by_key(|r| r.min_sl)
    }

    /// Minutes a day for `sl` (timeallow).
    pub fn minutes_for(&self, sl: u8) -> i64 {
        match self.sl_limit(sl) {
            Some(l) if l.minutes > 0 => l.minutes,
            _ => self.minutes_per_day,
        }
    }
}

/// A serial port callers may use: a direct-connect terminal, or a dial-in
/// modem (MODEM.DAT's strings; `~` pauses, `|` is a CR).
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SerialPort {
    pub device: PathBuf,
    /// Port (DTE) rate; with a modem, the caller's rate comes from CONNECT.
    pub baud: u32,
    /// RTS/CTS hardware flow control.
    pub rts_cts: bool,
    pub modem: bool,
    pub init: String,
    pub answer: String,
    pub hangup: String,
    /// Sent by the console's `O` so callers get a busy signal.
    pub offhook: String,
    /// RINGs before answering; 2 lets caller ID arrive first.
    pub rings: u8,
    /// Re-init the modem after this many minutes without a call, 0 = never.
    pub init_minutes: u64,
}

impl Default for SerialPort {
    fn default() -> Self {
        SerialPort {
            device: "/dev/ttyUSB0".into(),
            baud: 9600,
            rts_cts: false,
            modem: false,
            init: "ATZ|~ATE0V1Q0S0=0".into(),
            answer: "ATA".into(),
            hangup: "~~~+++~~~ATH0".into(),
            offhook: "ATH1M0".into(),
            rings: 1,
            init_minutes: 30,
        }
    }
}

/// A menu prompt users can pick instead of each menu's own (PROMPTS.DAT).
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Prompt {
    pub name: String,
    pub prompt: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Conf {
    pub name: String,
    pub acs: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Board {
    pub name: String,
    /// Storage name: `<data_path>/msgs/<file>.json`.
    pub file: String,
    pub acs: String,
    pub post_acs: String,
    /// Needed to use pipe colour/MCI codes in posts.
    pub mci_acs: String,
    /// Oldest messages are dropped past this many (0 = no limit).
    pub max_msgs: usize,
    pub anonymous: Anon,
    pub password: String,
    /// Posts are made under real names.
    pub real_names: bool,
    /// Conference number; 0 shows the base in every conference.
    pub conf: usize,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Anon {
    #[default]
    No,
    Yes,
    Forced,
    DearAbby,
    AnyName,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Strings {
    pub enter_handle: String,
    pub enter_pw: String,
    pub name_not_found: String,
    pub logon_new: String,
    pub wrong_password: String,
    pub phone_num: String,
    pub wrong_phone: String,
    pub new_user_pw: String,
    pub pause: String,
    pub aborted: String,
    pub msg_title: String,
    pub msg_send_to: String,
    pub bulletin_line: String,
    pub time_bank_deposit: String,
    pub time_bank_withdraw: String,
    pub post_anon: String,
    /// Message reader prompt when there's no READP menu (scanmessage).
    pub read_prompt: String,
    /// E-mail reader prompt when there's no EMAILP menu.
    pub mail_prompt: String,
    pub delete_read_mail: String,
    pub post_here: String,
    /// NUV vote prompt when there's no NUVP menu.
    pub nuv_prompt: String,
    pub nuv_validated: String,
    pub nuv_deleted: String,
    /// File listing prompt when there's no FILEP menu.
    pub file_prompt: String,
    /// `FF`: search by (W)ildcard or (S)tring.
    pub file_search: String,
    pub file_added: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bbs_name: "Impulse BBS".into(),
            sysop_name: "SysOp".into(),
            telnet: true,
            listen: "0.0.0.0:2323".into(),
            serial: Vec::new(),
            max_nodes: 8,
            data_path: "data".into(),
            menu_path: "menus".into(),
            text_path: "text".into(),
            start_menu: "MAIN".into(),
            closed_system: false,
            new_user_password: String::new(),
            shuttle_log: false,
            shuttle_menu: "MATRIX".into(),
            shuttle_pw: String::new(),
            new_sl: 20,
            new_dsl: 20,
            new_ar: String::new(),
            max_logon_tries: 4,
            unvalidated_logon: false,
            phone_pw: false,
            logon_pw_acs: "s255".into(),
            slow_baud: 0,
            slow_lockout: false,
            slow_hours: String::new(),
            idle_timeout_minutes: 10,
            minutes_per_day: 60,
            sysop_acs: "s255".into(),
            global_trap: false,
            auto_chat_capture: false,
            sysop_password: String::new(),
            max_message_lines: 100,
            max_time_bank: 60,
            bulletin_prefix: "bullet".into(),
            auto_sl: 50,
            auto_dsl: 50,
            auto_ar: String::new(),
            auto_ac: String::new(),
            auto_validate_new: false,
            use_nuv: false,
            nuv_yes: 5,
            nuv_no: 3,
            new_user_order: ["real_name", "city", "phone", "birthday", "sex", "ansi"].map(String::from).to_vec(),
            msop_acs: "s250".into(),
            anon_pub_read: "s250".into(),
            anon_priv_read: "s250".into(),
            anon_pub_post: "s250".into(),
            anon_priv_post: "s250".into(),
            boards: vec![Board { name: "General".into(), file: "general".into(), post_acs: "s20".into(), ..Board::default() }],
            fbases: vec![FBase { name: "Uploads".into(), file: "uploads".into(), dl_path: "data/files/uploads".into(), ..FBase::default() }],
            protocols: vec![
                Protocol {
                    key: "Z".into(),
                    name: "Zmodem".into(),
                    dl_cmd: "sz -b -e %F".into(),
                    ul_cmd: "rz -b -e".into(),
                    batch: true,
                    ..Protocol::default()
                },
                Protocol {
                    key: "Y".into(),
                    name: "Ymodem".into(),
                    dl_cmd: "sz -b --ymodem %F".into(),
                    ul_cmd: "rz -b --ymodem".into(),
                    batch: true,
                    ..Protocol::default()
                },
                Protocol { key: "X".into(), name: "Xmodem".into(), dl_cmd: "sz -b --xmodem %F".into(), ul_cmd: String::new(), ..Protocol::default() },
            ],
            events: Vec::new(),
            qwk_id: "IMPULSE".into(),
            qwk_max: 500,
            dl_ratio: 0,
            dlk_ratio: 0,
            sl_limits: Vec::new(),
            no_ratio_acs: String::new(),
            use_file_points: false,
            ul_points: 1,
            validate_uploads: false,
            ul_comment: String::new(),
            ul_add: String::new(),
            msg_confs: Vec::new(),
            prompts: Vec::new(),
            file_confs: Vec::new(),
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
            phone_num: "|U1Last 4 digits of your phone number: |U3".into(),
            wrong_phone: "|12Wrong phone number.".into(),
            new_user_pw: "|U1New user password: |U3".into(),
            pause: "|U2[|U3pause|U2]".into(),
            aborted: "|12Aborted.".into(),
            msg_title: "|U1Subject: |U3".into(),
            msg_send_to: "|U1Send to: |U3".into(),
            bulletin_line: "|NL|U1Bulletin to read |U2(|U3?|U2=list, |U3Q|U2=quit)|U1: |U3".into(),
            time_bank_deposit: "|NL|U1Deposit how many minutes? |U3".into(),
            time_bank_withdraw: "|NL|U1Withdraw how many minutes? |U3".into(),
            post_anon: "|U1Post anonymously? ".into(),
            read_prompt: "|NL|U1Read |U2(|U3?|U2=help)|U1: |U3".into(),
            mail_prompt: "|NL|U1Mail |U2(|U3?|U2=help)|U1: |U3".into(),
            delete_read_mail: "|U1Delete this letter? ".into(),
            post_here: "|NL|U1Post a message on this base? ".into(),
            nuv_prompt: "|NL|U1Vote |U2(|U3Y|U2)es (|U3N|U2)o (|U3A|U2)bstain (|U3Q|U2)uit|U1: |U3".into(),
            nuv_validated: "|NL|U1The user has been validated.".into(),
            nuv_deleted: "|NL|U1The user has been deleted.".into(),
            file_prompt: "|NL|U1Files |U2(|U3?|U2=help, |U3Enter|U2=more)|U1: |U3".into(),
            file_search: "|U1Search by (|U3W|U1)ildcard, (|U3S|U1)tring or (|U3Q|U1)uit: |U3".into(),
            file_added: "|U1Added to the batch queue.".into(),
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

impl Config {
    /// Write back to `path` (comments in a hand-edited file are not kept).
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let s = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(tmp, path)
    }
}

/// Entry templates for the list settings, for the configuration editor.
pub fn templates() -> Vec<(String, serde_json::Value)> {
    use crate::edit::tmpl;
    vec![
        tmpl::<Board>("boards"),
        tmpl::<FBase>("fbases"),
        tmpl::<Protocol>("protocols"),
        tmpl::<Conf>("msg_confs"),
        tmpl::<Conf>("file_confs"),
        tmpl::<Event>("events"),
        tmpl::<Prompt>("prompts"),
        tmpl::<SlLimit>("sl_limits"),
        tmpl::<SerialPort>("serial"),
    ]
}

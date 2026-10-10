# Impulse BBS (Rust)

A re-implementation of **Impulse 7.1**, Brandon Sneed's 1993-98 DOS BBS
written in Turbo Pascal, as a telnet and serial-line BBS in Rust (Unix). The original source
(`imp71src`, about 39k lines of Pascal) was the reference.

```sh
cargo run              # reads ./impulse.toml, listens on 0.0.0.0:2323
telnet localhost 2323  # type NEW; the first account created is the SysOp (SL 255)
```

Use a terminal that renders CP437 + ANSI (SyncTERM, NetRunner, or
`telnet` in a UTF-8 terminal for a rough look). Bytes are passed through
untouched, so original `.ANS` art displays as intended.

External tools, when present: `sz`/`rz` (lrzsz) for the default transfer
protocols, `zip`/`unzip` for QWK packets, FILE_ID.DIZ and
upload checks.

## What carries over from Impulse

| Impulse (Pascal)                                     | Here                          |
|------------------------------------------------------|-------------------------------|
| `IMP.PAS` main loop, `MENUS*.PAS` command dispatch   | `session.rs`, `cmds.rs`       |
| `.MNU` menus (text format read by `MNUCONV.PAS`)     | `menu.rs`, `menus/`           |
| `aacs1` ACS strings (`s20fA\|!rP`, `()`, `!`, `\|`)    | `acs.rs`                      |
| `OUTPUT.PAS` pipe codes, `printf` display files      | `session.rs`                  |
| `getc`/`setc`, FOSSIL/COM layer                      | `term.rs`, `serial.rs`        |
| `STATUS.DAT` settings + string table                 | `impulse.toml`, `config.rs`   |
| `USER.LST`, message bases, `*.DIR` file lists        | `data/*.json`, `db.rs`        |
| `MAIL*.PAS` messages and e-mail, `CONF.PAS`          | `msgs.rs`                     |
| `FILE*.PAS` file bases, transfers, batch, `ULCHECK`  | `files.rs`                    |
| `ZIPVIEWU.PAS`, `LZHVIEW.PAS` (plus ARJ)             | `arcview.rs`                  |
| `ANSIEDIT.PAS` full-screen editor                    | `fse.rs`                      |
| `DOORS.PAS`, `EXEC.PAS`                              | `doors.rs`                    |
| `MULTINOD.PAS` node messages and multinode chat      | `multinode.rs`                |
| `SCRIPT.PAS` / `ISL*.PAS` scripting                  | `script.rs`                   |
| `SYSOP*.PAS` editors, `IMPDOS.PAS`                   | `sysop.rs`, `edit.rs`         |
| `NUV.PAS`, `RUMORS.PAS`, `TIMEBANK.PAS`, `MISC*.PAS` | `nuv.rs`, `misc.rs`           |
| events (`EVENTS.DAT`), time chop                     | `events.rs`                   |
| QWK (`MQ`/`MR`)                                      | `qwk.rs`                      |
| `WFCMENU.PAS`                                        | `wfc.rs` (stdin console)      |

Menus behave the way they did: hot keys, `/X` and `//LONG` commands,
`FIRSTCMD`/`EVERYTIME`/`ENTER`, several commands chained on one key, the
`-^` / `-/` / `-\` goto/gosub/return stack with `MENU;C;CMD` data, and
pulldown (`P` flag) menus drawn as a lightbar from each command's short
description and X/Y. The original sample menu, `tests/legacy-main.mnu`,
parses unchanged.

Default menus `MAIN`, `MSG`, `FILE` and `SYSOP` reach every command.

### Commands

- **Menus/display:** `-^ -/ -\` `-F -L -P -S -C -$ -; -W -T` `XY` `SP SC`
- **Logoff/info:** `HC HI HM` `OI OY OC OL OK/OP` (OBV/2-style from `NEWINF.ANS` if present) `OA OB OS` `O1-O3` (matrix logon)
  `XC XU XS XH XL XA`
- **Messages:** `MA MP MS MN M# MZ M$ MJ MK ML ME MM` `CM CF CE` `MQ MR` (QWK)
- **Files:** `FA FL FS FF FN FP FD FU FV FR FY FK FZ F#` and batch `BL BR BC BD BU B?`
- **Doors/scripts:** `DC DD DG DS DW D-` (CHAIN.TXT, DORINFO1.DEF, DOOR.SYS…),
  `ID` (IMPULSE.IDF), `IS` (ISL scripts, plain or compiled)
- **Multinode:** `NW NS NC`, SysOp paging across nodes
- **Misc:** automessage `UR UW UA`, rumors `RR RA RL RD`, time bank `UD UL`,
  new-user voting `-N`, bulletins, BBS list `-V -A`, questionnaires `-Q -R`
- **SysOp (`%x`):** user, board, file base, config, menu and conference
  editors, become-user, logs, daily stats, mini-DOS (with `LADLE` SAUCE → FILE_ID.DIZ), read all mail, protocol
  editor `%P`, file maintenance `%1-%9 %$ %%`

## Running a system

- **Lines:** `telnet = false` turns off the telnet listener. Each `[[serial]]`
  entry (`device`, `baud` 300-115200, `rts_cts`) answers at 8N1 and gives
  the caller the lowest free node. Ports not listed are never opened.
  A direct-connect terminal wakes on a key. With `modem = true` the port
  is a dial-in modem (MODEM.DAT): `init` must get `OK`, `RING` is
  answered with `answer`, the caller's rate comes from `CONNECT`, a dropped
  carrier ends the call, and afterwards DTR is dropped and `hangup` sent.
  The modem is re-initialised after `init_minutes` without a call.
  `rings = 2` waits for caller ID (`NMBR`/`NAME`, logged with the
  connect). Drop files and `|LP` carry the caller's rate. ACS
  `B<n>` checks a serial caller's rate (n × 100); telnet callers always pass.
- **Setting up:** `impulse init [dir]` writes the stock config, menus and
  display files (INSTALL). `impulse clean [config]` deletes every user but
  #1 and the NUV queue (MAKECLEA). `new_user_order` sets which new-user
  questions are asked, in order (NUORDER/MAKENUO).
- **Console:** the server reads commands on stdin: `W` who's online, `S`
  today's stats, `M node text` node message (0 = all), `K node` hang up,
  `V node` watch a node live (the local screen), `B node [n]` its
  scrollback, `C node` chat with a caller (`/Q` ends), `A` toggle SysOp
  available (an `OC` page beeps here), `O` take idle modems off hook
  (busy) and back, `L [n]` log tail, `Q` shut down.
  The WFC editors, mail and pack run in a session, so log on over telnet
  (`telnet localhost 2323` is the local logon).
- **Callers:** menu prompts from `[[prompts]]` (PROMPTS.DAT, picked at
  logon or `OK` M), auto-signatures (`OK` G). `[[sl_limits]]` sets calls
  and minutes a day and download ratios per SL (`no_ratio_acs` is exempt).
- **Logon checks:** users not yet validated are hung up unless
  `unvalidated_logon`; `phone_pw` asks the phone number's last 4 digits;
  `logon_pw_acs` users also give the SysOp password; failed passwords count
  in the user's `illegal`. `slow_baud` with `slow_lockout`/`slow_hours`
  turns away slow serial callers (NO9600, NO9600H, YES9600H).
- **Limits:** posts, e-mail and feedback per call, mailbox size (bigger for
  `cosysop_acs`), `OC` pages per call (then GOAWAY and a letter), quoted
  lines, and free disk before a message. Message SysOps are exempt from
  the per-call limits. A user's `forward_to` sends their mail on to
  another user, and `mail_closed` closes their mailbox to all but
  Co-SysOps. `board_sysop` lists bases a user is message SysOp of. A board
  with `filter` strips colour and 8-bit characters from posts, and
  `unhidden` bases are listed to users who can't enter them.
- **Uploads:** `ul_points` per upload, or per `ul_points_kb` K. Uploaders
  outside `ul_credit_acs` get credit when `%7` validates the file.
  `see_unval_acs` users see unvalidated files.
- **Logs:** `data/sysop.log` rotates daily into `sysop1.log`… with the day's
  totals, keeping `back_sysop_logs` days. `screen_pause = false` turns
  pauses off for everyone.
- **PostgreSQL:** set `IMPULSE_DATABASE_URL` (e.g.
  `postgres://impulse:impulse@localhost:5432/impulse`) and the config,
  users, system state, message and file bases, line files and the SysOp log
  live in PostgreSQL instead of `data_path`. `docker compose up -d` runs one
  locally (`compose.yaml`). The first start copies the config file in; after
  that the database copy is used and SysOp edits save there. Documents are
  in table `kv` (`name`, `body` as text, so `body::jsonb` queries work),
  the log in `log`, rows older than `back_sysop_logs` days are deleted.
  Uploads, traps, ISL files and QWK packets stay on disk under `data_path`.
  `import` and `clean` use the database too. Existing `data/*.json` is not
  migrated: import or start fresh.
- **Importing a 7.1 system:** `impulse import <dir with USER.LST etc.> [config]`
  into a fresh `data_path`. Users (numbers, stats, flags, colours), message
  and file bases and `*.DIR` file lists come across; the config is backed up
  to `<config>.bak`. Check the file paths afterwards: they came from DOS.
- **Auditing:** `global_trap` or a user's `trap_activity` appends their
  keystrokes to `data/trap/trap.msg` (`trapN.msg` with `trap_separate`);
  `auto_chat_capture` or `chat_auto` records SysOp chats to `chat.msg`
  (`chatN.msg`); `slog_separate` copies a user's log lines to
  `slogN.node`. The user flags are in the `%` user editor and import from
  USER.LST.
- **Events:** `[[events]]` in the config: ACS-restricted logon windows, timed
  shell commands, and time chop before an event.

## Deliberate differences

- Telnet and Unix serial ports replace FOSSIL; modems answer but never
  dial out. Doors talk to the caller over pipes, so drop files name
  `COM0` (local) with the real rate. There's one thread per node.
- Passwords are stored as Argon2 hashes and are case-sensitive; logons also
  try the password in capitals, which is how Impulse stored them, so
  imported users keep working.
- Data is JSON, not Pascal binary records.
- Menus without a help file get a generated command list. Impulse 7.1 had
  dropped its generic menus. Menu flag `G` leaves out `GLOBAL.MNU`.
- The pulldown look is ours: 7.1's `DoPulls` isn't in the released source.
- Base numbers are always compressed: only the bases you can enter are
  numbered (7.1's `compressbases` on).
- Screens are 80 columns; the user's `linelen` isn't used.
- Per-call limits allow exactly the configured number (7.1 allowed one
  more `OC` page than `maxchat`); 0 means no limit.

## Not carried over

`MAKEWFC`: it builds `WFC.BIN`, which no 7.1 code reads. ISL `SETTIME`
and `SETDATE` are ignored: a script shouldn't set the host clock.
Echomail and netmail (board types, origin and tear lines, addresses, their
colours) aren't carried over: there's no mail tosser. Neither is modem dial-out.
`tfiledate` is left out because 7.1 never sets it.

## Layout

```
impulse.toml   configuration
menus/         *.MNU menus (MAIN, MSG, FILE, SYSOP)
text/          display files (WELCOME, LOGON, LOGOFF); add your own .ANS/.ASC
data/          created at runtime: users.json, system.json, msgs/, files/, sysop.log
compose.yaml   local services (PostgreSQL)
```

Tests: `cargo test` covers the ACS evaluator, colours and telnet escaping,
the legacy menu, passwords, wildcards, door macros, ISL, events, QWK,
the editor buffer and the binary import layouts.
`IMPULSE_TEST_DATABASE_URL=postgres://... cargo test` also runs the
PostgreSQL store test (it writes `test/*` rows and `test ...` log lines).

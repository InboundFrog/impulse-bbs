# Impulse BBS (Rust)

A re-implementation of **Impulse 7.1**, Brandon Sneed's 1993-98 DOS BBS
written in Turbo Pascal, as a telnet BBS in Rust. The original source
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
| `getc`/`setc`, FOSSIL/COM layer                      | `term.rs`                     |
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
- **Logoff/info:** `HC HI HM` `OI OY OC OL OK/OP OA OB OS` `O1-O3` (matrix logon)
  `XC XU XS XH XL XA`
- **Messages:** `MA MP MS MN M# MZ M$ MJ MK ML ME MM` `CM CF CE` `MQ MR` (QWK)
- **Files:** `FA FL FS FF FN FP FD FU FV FR FY FK FZ F#` and batch `BL BR BC BD BU B?`
- **Doors/scripts:** `DC DD DG DS DW D-` (CHAIN.TXT, DORINFO1.DEF, DOOR.SYS…),
  `ID` (IMPULSE.IDF), `IS` (ISL scripts, plain or compiled)
- **Multinode:** `NW NS NC`, SysOp paging across nodes
- **Misc:** automessage `UR UW UA`, rumors `RR RA RL RD`, time bank `UD UL`,
  new-user voting `-N`, bulletins, BBS list `-V -A`, questionnaires `-Q -R`
- **SysOp (`%x`):** user, board, file base, config, menu and conference
  editors, become-user, logs, daily stats, mini-DOS (with `LADLE` SAUCE → FILE_ID.DIZ), read all mail, file
  maintenance `%1-%9 %$ %%`

## Running a system

- **Console:** the server reads commands on stdin: `W` who's online, `S`
  today's stats, `M node text` node message (0 = all), `K node` hang up,
  `L [n]` log tail, `Q` shut down. The editors run in a session, so log on
  over telnet for those.
- **Importing a 7.1 system:** `impulse import <dir with USER.LST etc.> [config]`
  into a fresh `data_path`. Users (numbers, stats, flags, colours), message
  and file bases and `*.DIR` file lists come across; the config is backed up
  to `<config>.bak`. Check the file paths afterwards: they came from DOS.
- **Events:** `[[events]]` in the config: ACS-restricted logon windows, timed
  shell commands, and time chop before an event.

## Deliberate differences

- Telnet replaces modems and FOSSIL, and there's one thread per node.
- Passwords are stored as Argon2 hashes and are case-sensitive; logons also
  try the password in capitals, which is how Impulse stored them, so
  imported users keep working.
- Data is JSON, not Pascal binary records.
- Menus without a help file get a generated command list. Impulse 7.1 had
  dropped its generic menus. Menu flag `G` leaves out `GLOBAL.MNU`.
- The pulldown look is ours: 7.1's `DoPulls` isn't in the released source.
- The full-screen editor has a built-in header; `FSEHEAD`/`FSEHELP` aren't read.

## Not carried over

The local sysop screen and scrollback, the installer and the DOS
utilities (`MAKE*.PAS`).

## Layout

```
impulse.toml   configuration
menus/         *.MNU menus (MAIN, MSG, FILE, SYSOP)
text/          display files (WELCOME, LOGON, LOGOFF); add your own .ANS/.ASC
data/          created at runtime: users.json, system.json, msgs/, files/, sysop.log
```

Tests: `cargo test` covers the ACS evaluator, colours and telnet escaping,
the legacy menu, passwords, wildcards, door macros, ISL, events, QWK,
the editor buffer and the binary import layouts.

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

## What carries over from Impulse

| Impulse (Pascal)                                     | Here                     |
|------------------------------------------------------|--------------------------|
| `IMP.PAS` main loop, `MENUS*.PAS` command dispatch   | `session.rs`, `cmds.rs`  |
| `.MNU` menus (text format read by `MNUCONV.PAS`)     | `menu.rs`, `menus/`      |
| `aacs1` ACS strings (`s20fA\|!rP`, `()`, `!`, `\|`)    | `acs.rs`                 |
| `OUTPUT.PAS` pipe codes `\|00-\|15 \|B0 \|U0-9 \|UN \|TL`… | `Session::sprompt`       |
| `printf` display files, `NAME.1-9` random screens    | `Session::printf`        |
| top/mid/bot templates with `%XX` vars (`sysMciSet`)  | `Session::tmpl`          |
| `getc`/`setc` DOS attribute → ANSI                   | `term.rs`                |
| `STATUS.DAT` settings + string table                 | `impulse.toml`           |
| `USER.LST`, `LASTON.DAT`, message bases              | `data/*.json`            |

Menus behave the way they did: hot keys, `/X` and `//LONG` commands,
`FIRSTCMD`/`EVERYTIME`/`ENTER`, `global.mnu` merged in, several commands
chained on one key, and the `-^` / `-/` / `-\` goto/gosub/return stack with
`MENU;C;CMD` data. The original sample menu, `tests/legacy-main.mnu`, parses
unchanged.

**Commands so far:** menu navigation (`-^ -/ -\`), `-F -L -S -C -$`, logoff
(`HC HI HM`), `OI OY OC OL OK/OP`, `XC XU XS XH XL XA`, automessage (`UR UW`),
rumors (`RR RA RL RD`), `SP SC`, messages (`MA MP MS MN M#`), e-mail (`ME MM`),
who's online (`NW`). Logon, new-user signup, daily time limits, idle timeout
and multiple nodes all work.

Any other command prints "not implemented yet" and is noted in
`data/sysop.log`.

## Deliberate differences

- Telnet replaces modems and FOSSIL, and there's one thread per node.
  There's no local WFC/sysop console, so log in over telnet.
- Passwords are stored as Argon2 hashes and are case-sensitive. Impulse
  stored them in plain text, upper-cased.
- Data is JSON, not Pascal binary records. Original `USER.LST`/`*.BRD`
  files can't be imported yet.
- Menus without a help file get a generated command list. Impulse 7.1 had
  dropped its generic menus.

## Not done yet (the original units to port)

File areas and transfers (`FILE*.PAS`, needs zmodem), doors (`DOORS.PAS`,
DOOR.SYS/DORINFO), the sysop editors (`SYSOP*.PAS`: user/menu/board editors),
new user voting (`NUV.PAS`), the time bank, conferences, QWK, the full-screen
editor and ANSI editor, the ISL scripting language (`SCRIPT.PAS`), multinode
chat and node messages, bulletins, BBS list, pulldown menus, and importing the
original binary data files.

## Layout

```
impulse.toml   configuration
menus/         *.MNU menus (MAIN, MSG)
text/          display files (WELCOME, LOGON, LOGOFF); add your own .ANS/.ASC
data/          created at runtime: users.json, system.json, msgs/, sysop.log
```

Tests: `cargo test` covers the ACS evaluator, the colour translation, parsing
the legacy menu and password hashing.

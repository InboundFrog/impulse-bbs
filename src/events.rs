//! Scheduled events (EVENTS.DAT): `A` events limit logons to an ACS during
//! their window, `D`/`E` events run a command at their time. Callers' time
//! is cut short so they're off before a command event starts (findchoptime).

use crate::config::Event;
use crate::session::Shared;
use chrono::{Datelike, Local, NaiveDateTime, Timelike};
use std::sync::Arc;
use std::time::Duration;

/// Minutes past midnight for "HH:MM".
pub fn hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    Some(h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?)
}

/// Does `e` run on the day of `t`?
fn runs_on(e: &Event, t: NaiveDateTime) -> bool {
    if e.day_of_month > 0 {
        return t.day() == e.day_of_month as u32;
    }
    e.days.is_empty() || e.days.contains(&(t.weekday().num_days_from_sunday() as u8))
}

/// Minutes from `t` until `e` next starts, looking `within` minutes ahead.
pub fn minutes_until(e: &Event, t: NaiveDateTime, within: i64) -> Option<i64> {
    let start = hhmm(&e.time)?;
    let now = (t.hour() * 60 + t.minute()) as i64;
    (0..=within.min(2 * 1440)).find(|&d| {
        let at = now + d;
        at % 1440 == start && runs_on(e, t + chrono::Duration::minutes(d))
    })
}

/// Is `t` inside the event's window [time, time + duration)?
pub fn in_window(e: &Event, t: NaiveDateTime) -> bool {
    let Some(start) = hhmm(&e.time) else { return false };
    let now = (t.hour() * 60 + t.minute()) as i64;
    let since = (now - start).rem_euclid(1440);
    let day = t - chrono::Duration::minutes(since);
    e.active && since < e.duration.max(1) as i64 && runs_on(e, day)
}

/// Background thread: run due `D`/`E` events, once a minute.
pub fn spawn(sh: Arc<Shared>) {
    std::thread::spawn(move || {
        loop {
            let secs = Local::now().second() as u64;
            std::thread::sleep(Duration::from_secs(60 - secs.min(59)));
            let now = Local::now().naive_local();
            for e in sh.cfg().events.iter().filter(|e| e.active && e.kind != "A") {
                if minutes_until(e, now, 0) == Some(0) {
                    sh.db().log(&format!("[{}] Event \"{}\": {}", crate::session::now(), e.description, e.data));
                    let st = std::process::Command::new("sh").arg("-c").arg(&e.data).status();
                    sh.db().log(&format!("[{}] Event \"{}\" finished: {st:?}", crate::session::now(), e.description));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule() {
        let t = |s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        // 2026-10-09 is a Friday (5)
        let e = Event { active: true, kind: "D".into(), time: "02:00".into(), duration: 30, ..Event::default() };
        assert_eq!(minutes_until(&e, t("2026-10-09 01:30"), 60), Some(30));
        assert_eq!(minutes_until(&e, t("2026-10-09 23:30"), 60), None);
        assert_eq!(minutes_until(&e, t("2026-10-09 23:30"), 180), Some(150));
        assert!(in_window(&e, t("2026-10-09 02:10")));
        assert!(!in_window(&e, t("2026-10-09 02:30")));
        let sat = Event { days: vec![6], ..e.clone() };
        assert_eq!(minutes_until(&sat, t("2026-10-09 01:30"), 60), None);
        assert!(!in_window(&Event { time: "23:50".into(), ..sat }, t("2026-10-10 00:05")));
    }
}

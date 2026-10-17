//! Reminders: a thread that looks as each wall-clock minute starts (the
//! clock's way, so it is on time after a suspend) for every reminder whose
//! moment came since it last looked, and says each in a desktop
//! notification. Where it had looked up to is kept beside the events
//! (`calendar-reminded`), so a reminder due while the bar wasn't running
//! or the machine slept is said when it is back, if its event isn't over;
//! none is said twice. The mark is written when one is said and when the
//! events change, never every minute: a restart then looks back no
//! further than the last of those, and an event made after its reminder's
//! moment never says it late.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_core::log_warn;

use super::date::{self, Date};
use super::store::{self, Event, Shared};
use crate::clock::{ClockOptions, Local};
use crate::settings::Settings;

/// The mark's file, beside the events'.
const MARK: &str = "calendar-reminded";
/// How many days on a reminder can be for: a week, and the day it ends.
const AHEAD: i64 = 8;

/// Start the thread: it looks at once (for what came due while it wasn't
/// running), then every minute.
pub fn start(book: Shared) {
    let mark = lntrn_sys::dirs::lantern_config().map(|d| d.join("control-center").join(MARK));
    let mut look = Look { since: mark.as_deref().and_then(read_mark), mark, revision: None };
    look.once(&book);
    if let Err(e) = crate::clock::each_minute("reminders", move || look.once(&book)) {
        log_warn!("calendar: no reminders thread: {e}; no reminders until the bar starts again");
    }
}

struct Look {
    /// Up to when reminders were said; `None` before the first look,
    /// when none was ever kept.
    since: Option<i64>,
    mark: Option<PathBuf>,
    /// The events' revision the mark was last written for.
    revision: Option<u64>,
}

impl Look {
    fn once(&mut self, book: &Shared) {
        let (Some(now), Some(today)) = (now_secs(), Local::now().map(|t| Date::of(&t))) else { return };
        // The clock set back: start again from now.
        let since = self.since.filter(|s| *s <= now).unwrap_or(now);
        let (events, revision) = {
            let b = store::lock(book);
            (b.events().to_vec(), b.revision())
        };
        let due = due(&events, today, since, now, |d, m| Local::epoch_at(d.year, d.month, d.day, m));
        if !due.is_empty() {
            let clock = Settings::read().map_or_else(ClockOptions::default, |s| s.clock);
            for (event, day) in &due {
                say(event, *day, today, clock);
            }
        }
        if !due.is_empty() || self.revision != Some(revision) {
            if let Some(path) = &self.mark
                && let Err(e) = write_mark(path, now)
            {
                log_warn!("calendar: can't keep {MARK}: {e}");
            }
            self.revision = Some(revision);
        }
        self.since = Some(now);
    }
}

/// Every reminder of `events` whose moment is after `since` and no later
/// than `now` (seconds since the epoch), whose event isn't over by `now`:
/// each event with the day it is on. `epoch` says when a minute of a day
/// is.
pub fn due(events: &[Event], today: Date, since: i64, now: i64, epoch: impl Fn(Date, u32) -> Option<i64>) -> Vec<(Event, Date)> {
    let mut out = Vec::new();
    for day in (-1..=AHEAD).map(|n| today.plus(n)) {
        for event in events.iter().filter(|e| e.on(day)) {
            let Some(before) = event.remind else { continue };
            let (Some(anchor), Some(end)) = (epoch(day, event.anchor()), epoch(day, event.span().1)) else { continue };
            let at = anchor - i64::from(before) * 60;
            if at > since && at <= now && end > now {
                out.push((event.clone(), day));
            }
        }
    }
    out
}

/// What a reminder says under its title: when, from `today`, and the
/// first line of its notes.
pub fn body(event: &Event, day: Date, today: Date, clock: ClockOptions) -> String {
    let when = match event.time {
        Some((start, end)) => format!("{}, {} – {}", day.spoken(today), date::time_text(start, clock), date::time_text(end, clock)),
        None => format!("{}, all day", day.spoken(today)),
    };
    match event.notes.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(note) => format!("{when}\n{note}"),
        None => when,
    }
}

/// Say a reminder in a desktop notification.
fn say(event: &Event, day: Date, today: Date, clock: ClockOptions) {
    let body = body(event, day, today, clock);
    let sent = std::process::Command::new("notify-send").args(["-a", "Calendar", "-i", "x-office-calendar", &event.title, &body]).status();
    match sent {
        Ok(status) if status.success() => {}
        Ok(status) => log_warn!("calendar: notify-send said {status} for {:?}", event.title),
        Err(e) => log_warn!("calendar: no notify-send for {:?}: {e}", event.title),
    }
}

fn now_secs() -> Option<i64> {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()
}

fn read_mark(path: &Path) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn write_mark(path: &Path, secs: i64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = path.with_extension("part");
    std::fs::write(&part, format!("{secs}\n"))?;
    std::fs::rename(&part, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::store::Repeat;

    fn d(year: i32, month: u32, day: u32) -> Date {
        Date::new(year, month, day).unwrap()
    }

    /// A clock with no zone and no summer time.
    fn epoch(day: Date, minute: u32) -> Option<i64> {
        Some(day.days() * 86_400 + i64::from(minute) * 60)
    }

    fn at(day: Date, minute: u32) -> i64 {
        epoch(day, minute).unwrap()
    }

    fn dentist(remind: Option<u32>) -> Event {
        Event { id: 1, title: "Dentist".into(), remind, ..Event::new(d(2026, 10, 17), Some((900, 960))) }
    }

    #[test]
    fn a_reminder_is_due_in_the_minute_its_moment_comes() {
        let today = d(2026, 10, 17);
        let e = [dentist(Some(15))];
        // 2:45 PM is its moment.
        assert!(due(&e, today, at(today, 884), at(today, 885), epoch).len() == 1);
        assert!(due(&e, today, at(today, 885), at(today, 886), epoch).is_empty(), "said once");
        assert!(due(&e, today, at(today, 880), at(today, 884), epoch).is_empty(), "not yet");
        assert!(due(&[dentist(None)], today, at(today, 0), at(today, 1439), epoch).is_empty(), "no reminder, nothing said");
    }

    #[test]
    fn missed_reminders_are_said_while_their_event_isnt_over() {
        let today = d(2026, 10, 17);
        let e = [dentist(Some(15))];
        // Asleep from noon: woken at 3:30 PM, mid-appointment, it's said.
        assert_eq!(due(&e, today, at(today, 720), at(today, 930), epoch).len(), 1);
        // Woken at 4:30 PM, it's over: nothing.
        assert!(due(&e, today, at(today, 720), at(today, 990), epoch).is_empty());
    }

    #[test]
    fn all_day_and_day_before_reminders_count_back_from_the_morning() {
        let today = d(2026, 10, 17);
        let bday = Event { id: 2, title: "Mom's birthday".into(), remind: Some(1440), repeat: Repeat::Yearly, ..Event::new(d(2000, 10, 18), None) };
        // 9 AM the day before this year's.
        let found = due(std::slice::from_ref(&bday), today, at(today, 539), at(today, 540), epoch);
        assert_eq!(found, vec![(bday.clone(), d(2026, 10, 18))]);
        let morning = Event { remind: Some(0), ..bday };
        assert_eq!(due(&[morning], today, at(today, 539), at(today, 540), epoch).len(), 0, "this year's is tomorrow");
    }

    #[test]
    fn a_reminder_says_when_from_today_and_its_first_note() {
        let today = d(2026, 10, 17);
        let e = Event { notes: "\n  Bring the card \nand ID".into(), ..dentist(Some(15)) };
        assert_eq!(body(&e, today, today, ClockOptions::default()), "Today, 3:00 PM – 4:00 PM\nBring the card");
        let all_day = Event { time: None, ..dentist(Some(0)) };
        assert_eq!(body(&all_day, d(2026, 10, 18), today, ClockOptions::default()), "Tomorrow, all day");
    }

    #[test]
    fn the_mark_keeps() {
        let dir = std::env::temp_dir().join(format!("lntrn-cc-remind-{}", std::process::id()));
        let path = dir.join(MARK);
        write_mark(&path, 1_791_000_000).unwrap();
        assert_eq!(read_mark(&path), Some(1_791_000_000));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

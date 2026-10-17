//! The events: what one is (its day, its time or all day, how it repeats
//! and reminds, its colour, its notes) and the book of them that the bar,
//! its reminders and the file's watcher share. The book is kept in
//! `calendar.toml` (`file`), written whole beside it and renamed into
//! place so a crash never leaves half of it; a file that won't parse
//! (caught mid-edit) leaves the events as they were.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use lntrn_core::log_warn;
use lntrn_kit::look;
use lntrn_math::Color;

use super::date::Date;
use super::file::{parse, write};

/// The file, under the bar's config folder.
pub const FILE: &str = "calendar.toml";
/// What an all-day event's reminder counts back from: 9 AM.
pub const MORNING: u32 = 9 * 60;
/// A timed event's length when nothing says otherwise.
pub const HOUR: u32 = 60;
/// A reminder this far ahead, at most: a week.
pub(super) const REMIND_MAX: i64 = 7 * 1440;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Never,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Repeat {
    pub const ALL: [Repeat; 5] = [Repeat::Never, Repeat::Daily, Repeat::Weekly, Repeat::Monthly, Repeat::Yearly];

    pub fn word(self) -> &'static str {
        match self {
            Repeat::Never => "never",
            Repeat::Daily => "daily",
            Repeat::Weekly => "weekly",
            Repeat::Monthly => "monthly",
            Repeat::Yearly => "yearly",
        }
    }

    pub fn from_word(word: &str) -> Repeat {
        Repeat::ALL.into_iter().find(|r| r.word() == word).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Never => "Never",
            Repeat::Daily => "Every day",
            Repeat::Weekly => "Every week",
            Repeat::Monthly => "Every month",
            Repeat::Yearly => "Every year",
        }
    }
}

/// An event's colour: its bar in the day's list, its dot on the month.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Swatch {
    #[default]
    Gold,
    Red,
    Green,
    Blue,
    Purple,
    Pink,
}

impl Swatch {
    pub const ALL: [Swatch; 6] = [Swatch::Gold, Swatch::Red, Swatch::Green, Swatch::Blue, Swatch::Purple, Swatch::Pink];

    pub fn word(self) -> &'static str {
        match self {
            Swatch::Gold => "gold",
            Swatch::Red => "red",
            Swatch::Green => "green",
            Swatch::Blue => "blue",
            Swatch::Purple => "purple",
            Swatch::Pink => "pink",
        }
    }

    pub fn from_word(word: &str) -> Swatch {
        Swatch::ALL.into_iter().find(|s| s.word() == word).unwrap_or_default()
    }

    pub fn color(self) -> Color {
        match self {
            Swatch::Gold => look::GOLD,
            Swatch::Red => look::BAD,
            Swatch::Green => look::GOOD,
            Swatch::Blue => look::INFO,
            Swatch::Purple => Color::hex(0xB894FF),
            Swatch::Pink => Color::hex(0xFF8CC6),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Its own, for finding it again; 0 until it is first kept.
    pub id: u64,
    pub title: String,
    /// The day it happens on, the first of them when it repeats.
    pub date: Date,
    /// When it starts and ends, minutes into its day (the end up to the
    /// midnight after, 1440); `None` lasts all day.
    pub time: Option<(u32, u32)>,
    pub repeat: Repeat,
    /// How many minutes before it starts (an all-day one: before
    /// [`MORNING`] on its day) to say so; `None` says nothing.
    pub remind: Option<u32>,
    pub swatch: Swatch,
    pub notes: String,
    /// Days a repeating event is left out on.
    pub skip: Vec<Date>,
}

impl Event {
    /// A new one on `date`, `time` long or all day, reminding as a new one
    /// does: 15 minutes before, or the morning of.
    pub fn new(date: Date, time: Option<(u32, u32)>) -> Event {
        let remind = Some(if time.is_some() { 15 } else { 0 });
        Event { id: 0, title: String::new(), date, time, repeat: Repeat::Never, remind, swatch: Swatch::Gold, notes: String::new(), skip: Vec::new() }
    }

    /// Whether it happens on `day`.
    pub fn on(&self, day: Date) -> bool {
        let (from, d) = (self.date, day);
        let falls = match self.repeat {
            Repeat::Never => d == from,
            Repeat::Daily => d >= from,
            Repeat::Weekly => d >= from && (d.days() - from.days()) % 7 == 0,
            Repeat::Monthly => d >= from && d.day == from.day,
            Repeat::Yearly => d >= from && d.month == from.month && d.day == from.day,
        };
        falls && !self.skip.contains(&day)
    }

    /// Minutes into its day it starts (all day: from midnight) and ends.
    pub fn span(&self) -> (u32, u32) {
        self.time.unwrap_or((0, 1440))
    }

    /// The minute of its day its reminder counts back from.
    pub fn anchor(&self) -> u32 {
        self.time.map_or(MORNING, |(start, _)| start)
    }

    /// Held to what it can be: a title on one line, a start before its end
    /// within the day, a reminder no more than a week ahead.
    pub(super) fn tidy(mut self) -> Event {
        self.title = self.title.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        self.time = self.time.map(|(start, end)| {
            let start = start.min(1440 - 15);
            (start, if end > start && end <= 1440 { end } else { (start + HOUR).min(1440) })
        });
        self.remind = self.remind.map(|m| m.min(REMIND_MAX as u32));
        self.skip.sort();
        self.skip.dedup();
        self
    }
}

/// Every event, and where they are kept.
#[derive(Debug, Default)]
pub struct Book {
    events: Vec<Event>,
    /// `None` keeps them in memory alone (the tests').
    path: Option<PathBuf>,
    /// Counts their changes, from here or the file.
    revision: u64,
}

/// The book as the bar, its reminders and its watcher share it.
pub type Shared = Arc<Mutex<Book>>;

/// The book, whoever held it last having panicked or not.
pub fn lock(book: &Shared) -> MutexGuard<'_, Book> {
    book.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Book {
    /// The events as their file has them now: none when there is none yet.
    /// The tests' book has no file, so they never touch the real one.
    pub fn load() -> Book {
        let path = if cfg!(test) { None } else { lntrn_sys::dirs::lantern_config().map(|d| d.join("control-center").join(FILE)) };
        let mut book = Book { events: Vec::new(), path, revision: 0 };
        book.reload();
        book
    }

    #[cfg(test)]
    pub fn in_memory(events: Vec<Event>) -> Book {
        let mut book = Book { events: Vec::new(), path: None, revision: 0 };
        for e in events {
            book.put(e);
        }
        book
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Goes up with every change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Read the file again. Returns whether the events changed; a file
    /// that won't parse changes nothing.
    pub fn reload(&mut self) -> bool {
        let Some(path) = &self.path else { return false };
        let events = match std::fs::read_to_string(path) {
            Ok(text) => match parse(&text) {
                Some(events) => events,
                None => {
                    log_warn!("calendar: {} doesn't parse; keeping the events as they were", path.display());
                    return false;
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                log_warn!("calendar: can't read {}: {e}", path.display());
                return false;
            }
        };
        let changed = events != self.events;
        if changed {
            self.events = events;
            self.revision += 1;
        }
        changed
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn get(&self, id: u64) -> Option<&Event> {
        self.events.iter().find(|e| e.id == id)
    }

    /// What happens on `day` (see [`on`]).
    #[cfg(test)]
    pub fn on(&self, day: Date) -> Vec<&Event> {
        on(&self.events, day)
    }

    /// Keep `event`: in its old one's place, or (id 0) as a new one. Saves.
    /// Returns its id.
    pub fn put(&mut self, event: Event) -> u64 {
        let mut event = event.tidy();
        match self.events.iter_mut().find(|e| e.id == event.id && event.id != 0) {
            Some(old) => *old = event.clone(),
            None => {
                event.id = self.events.iter().map(|e| e.id).max().unwrap_or(0) + 1;
                self.events.push(event.clone());
            }
        }
        self.save_or_say();
        event.id
    }

    /// Take the event out, every day of it. Saves.
    pub fn remove(&mut self, id: u64) {
        self.events.retain(|e| e.id != id);
        self.save_or_say();
    }

    /// Leave a repeating event out on `day` alone (one that doesn't repeat
    /// goes). Saves.
    pub fn skip(&mut self, id: u64, day: Date) {
        let Some(event) = self.events.iter_mut().find(|e| e.id == id) else { return };
        if event.repeat == Repeat::Never {
            self.remove(id);
            return;
        }
        event.skip.push(day);
        *event = event.clone().tidy();
        self.save_or_say();
    }

    /// They changed here: count it and save.
    fn save_or_say(&mut self) {
        self.revision += 1;
        if let Err(e) = self.save() {
            log_warn!("calendar: can't save {FILE}: {e}");
        }
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let part = path.with_extension("toml.part");
        std::fs::write(&part, write(&self.events))?;
        std::fs::rename(&part, path)
    }
}

/// What of `events` happens on `day`: all-day ones first, then by when
/// they start.
pub fn on(events: &[Event], day: Date) -> Vec<&Event> {
    let mut on: Vec<&Event> = events.iter().filter(|e| e.on(day)).collect();
    on.sort_by_key(|e| (e.time.is_some(), e.span(), e.id));
    on
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> Date {
        Date::new(year, month, day).unwrap()
    }

    fn event(title: &str, date: Date, repeat: Repeat) -> Event {
        Event { title: title.into(), repeat, ..Event::new(date, Some((900, 960))) }
    }

    #[test]
    fn repeating_events_fall_on_their_days() {
        let from = d(2026, 10, 17);
        let weekly = event("Raid night", from, Repeat::Weekly);
        assert!(weekly.on(from) && weekly.on(d(2026, 10, 24)) && weekly.on(d(2027, 1, 2)));
        assert!(!weekly.on(d(2026, 10, 10)) && !weekly.on(d(2026, 10, 18)), "not before it, nor off its weekday");
        let monthly = event("Rent", d(2026, 1, 31), Repeat::Monthly);
        assert!(monthly.on(d(2026, 3, 31)) && !monthly.on(d(2026, 2, 28)) && !monthly.on(d(2026, 4, 30)), "only months with a 31st");
        let leap = event("Leap day", d(2024, 2, 29), Repeat::Yearly);
        assert!(leap.on(d(2028, 2, 29)) && !leap.on(d(2025, 2, 28)));
        let daily = Event { skip: vec![d(2026, 10, 19)], ..event("Walk", from, Repeat::Daily) };
        assert!(daily.on(d(2026, 10, 18)) && !daily.on(d(2026, 10, 19)) && daily.on(d(2026, 10, 20)), "a skipped day is left out");
        assert!(event("Once", from, Repeat::Never).on(from) && !event("Once", from, Repeat::Never).on(d(2026, 10, 24)));
    }

    #[test]
    fn a_day_lists_all_day_events_first_then_by_start() {
        let day = d(2026, 10, 17);
        let book = Book::in_memory(vec![
            Event { time: Some((1140, 1200)), ..event("Raid", day, Repeat::Never) },
            Event { time: None, ..event("Birthday", day, Repeat::Yearly) },
            Event { time: Some((540, 600)), ..event("Standup", day, Repeat::Daily) },
            event("Elsewhen", d(2026, 10, 18), Repeat::Never),
        ]);
        let titles: Vec<&str> = book.on(day).iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Birthday", "Standup", "Raid"]);
    }

    #[test]
    fn keeping_gives_new_events_ids_and_replaces_old_ones() {
        let mut book = Book::in_memory(Vec::new());
        let day = d(2026, 10, 17);
        let a = book.put(event("A", day, Repeat::Never));
        let b = book.put(event("B", day, Repeat::Weekly));
        assert_eq!((a, b), (1, 2));
        book.put(Event { id: a, ..event("A again", day, Repeat::Never) });
        assert_eq!(book.get(a).map(|e| e.title.as_str()), Some("A again"));
        assert_eq!(book.events().len(), 2);
        book.skip(b, d(2026, 10, 24));
        assert!(!book.get(b).unwrap().on(d(2026, 10, 24)) && book.get(b).unwrap().on(d(2026, 10, 31)));
        book.skip(a, day);
        assert!(book.get(a).is_none(), "skipping the one day of a one-off takes it out");
        book.remove(b);
        assert!(book.events().is_empty());
    }

    #[test]
    fn events_are_held_to_what_they_can_be() {
        let day = d(2026, 10, 17);
        let odd = Event { title: "  Two\nlines ".into(), time: Some((1200, 1100)), remind: Some(99_999), ..Event::new(day, None) }.tidy();
        assert_eq!((odd.title.as_str(), odd.time, odd.remind), ("Two lines", Some((1200, 1260)), Some(10_080)));
        assert_eq!(Event { time: Some((1439, 1440)), ..Event::new(day, None) }.tidy().time, Some((1425, 1440)));
    }
}

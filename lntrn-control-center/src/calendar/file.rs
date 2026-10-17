//! How the events are kept: `~/.lantern/config/control-center/calendar.toml`,
//! one `[[event]]` table each, to read and edit by hand:
//!
//! ```toml
//! [[event]]
//! id = 1
//! title = "Dentist"
//! date = 2026-10-17
//! start = "15:00"
//! end = "16:00"
//! repeat = "never"     # daily, weekly, monthly, yearly
//! remind = 15          # minutes before; left out for none
//! color = "gold"       # red, green, blue, purple, pink
//! notes = "Bring the card"
//! skip = [2026-10-24]  # days a repeating one is left out on
//! ```
//!
//! An event without `start` lasts all day, and its reminder counts back
//! from 9 AM on its day. A monthly event on the 31st happens only in
//! months that have one, a yearly one on February 29th only in leap years.
//! An event with no title or date is passed over, one without an id (or
//! with another's) given its own.

use lntrn_core::log_warn;
use lntrn_data::{Doc, toml};

use super::date::Date;
use super::store::{Event, HOUR, REMIND_MAX, Repeat, Swatch};

const HEADER: &str = "# lntrn-control-center calendar\n\
# One [[event]] table each: title, date (YYYY-MM-DD), start and end (\"HH:MM\",\n\
# left out for all day), repeat (never, daily, weekly, monthly, yearly),\n\
# remind (minutes before, left out for none), color (gold, red, green,\n\
# blue, purple, pink), notes, skip (days a repeating one is left out on).\n\n";

/// `minute` as kept: `15:00`.
fn hhmm(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// `15:00` (or a TOML time, `15:00:00`) as minutes into the day; `24:00`
/// is its end.
fn minutes(text: &str) -> Option<u32> {
    let (h, rest) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, rest.get(..2)?.parse().ok()?);
    (m < 60 && (h < 24 || (h == 24 && m == 0))).then_some(h * 60 + m)
}

/// A date or time as a TOML value or a string.
fn text(doc: &Doc) -> Option<&str> {
    match doc {
        Doc::DateTime(s) | Doc::Str(s) => Some(s),
        _ => None,
    }
}

/// The events in a file's text; `None` when it doesn't parse. An event
/// with no title or no date is passed over; one without an id, or with
/// another's, is given a new one.
pub fn parse(text_in: &str) -> Option<Vec<Event>> {
    let doc = toml::parse(text_in).ok()?;
    let mut events: Vec<Event> = Vec::new();
    for item in doc.get("event").and_then(Doc::as_list).unwrap_or(&[]) {
        let s = |k: &str| item.get(k).and_then(Doc::as_str);
        let (Some(title), Some(date)) = (s("title").filter(|t| !t.trim().is_empty()), item.get("date").and_then(text).and_then(Date::parse)) else {
            log_warn!("calendar: passing over an event with no title or date");
            continue;
        };
        let start = item.get("start").and_then(text).and_then(minutes);
        let end = item.get("end").and_then(text).and_then(minutes);
        let event = Event {
            id: item.get("id").and_then(Doc::as_i64).and_then(|v| u64::try_from(v).ok()).unwrap_or(0),
            title: title.to_owned(),
            date,
            time: start.map(|start| (start, end.unwrap_or(start + HOUR))),
            repeat: s("repeat").map_or(Repeat::Never, Repeat::from_word),
            remind: item.get("remind").and_then(Doc::as_i64).map(|m| m.clamp(0, REMIND_MAX) as u32),
            swatch: s("color").map_or(Swatch::Gold, Swatch::from_word),
            notes: s("notes").unwrap_or("").to_owned(),
            skip: item.get("skip").and_then(Doc::as_list).unwrap_or(&[]).iter().filter_map(text).filter_map(Date::parse).collect(),
        };
        events.push(event.tidy());
    }
    let mut next = events.iter().map(|e| e.id).max().unwrap_or(0) + 1;
    for i in 0..events.len() {
        if events[i].id == 0 || events[..i].iter().any(|e| e.id == events[i].id) {
            events[i].id = next;
            next += 1;
        }
    }
    Some(events)
}

/// The events as the file keeps them.
pub fn write(events: &[Event]) -> String {
    let mut list = Vec::with_capacity(events.len());
    for e in events {
        let mut t = Doc::map();
        t.set("id", Doc::Int(e.id as i64));
        t.set("title", Doc::Str(e.title.clone()));
        t.set("date", Doc::DateTime(e.date.word()));
        if let Some((start, end)) = e.time {
            t.set("start", Doc::Str(hhmm(start)));
            t.set("end", Doc::Str(hhmm(end)));
        }
        t.set("repeat", Doc::Str(e.repeat.word().to_owned()));
        if let Some(m) = e.remind {
            t.set("remind", Doc::Int(i64::from(m)));
        }
        t.set("color", Doc::Str(e.swatch.word().to_owned()));
        if !e.notes.is_empty() {
            t.set("notes", Doc::Str(e.notes.clone()));
        }
        if !e.skip.is_empty() {
            t.set("skip", Doc::List(e.skip.iter().map(|d| Doc::DateTime(d.word())).collect()));
        }
        list.push(t);
    }
    let mut doc = Doc::map();
    doc.set("event", Doc::List(list));
    format!("{HEADER}{}", if events.is_empty() { String::new() } else { toml::write(&doc) })
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
    fn the_file_reads_back_as_written() {
        let day = d(2026, 10, 17);
        let events = vec![
            Event { id: 1, notes: "Bring the \"card\"\nand ID".into(), remind: Some(30), swatch: Swatch::Blue, ..event("Dentist", day, Repeat::Never) },
            Event { id: 4, time: None, remind: None, skip: vec![d(2027, 10, 17)], ..event("Mom's birthday", day, Repeat::Yearly) },
        ];
        let text = write(&events);
        assert!(text.starts_with(HEADER) && text.contains("date = 2026-10-17") && text.contains("start = \"15:00\""), "{text}");
        assert_eq!(parse(&text), Some(events));
        assert_eq!(parse(&write(&[])), Some(Vec::new()));
    }

    #[test]
    fn a_hand_edited_file_reads_as_meant() {
        let text = "[[event]]\ntitle = \"Lunch\"\ndate = 2026-10-20\nstart = 12:30:00\n\n[[event]]\ntitle = \"No date\"\n\n[[event]]\nid = 7\ntitle = \"Trip\"\ndate = \"2026-11-01\"\nrepeat = \"sometimes\"\ncolor = \"plaid\"\nremind = -5\n\n[[event]]\nid = 7\ntitle = \"Twin\"\ndate = 2026-11-02\n";
        let events = parse(text).expect("parses");
        assert_eq!(events.len(), 3, "the one without a date is passed over");
        assert_eq!((events[0].time, events[0].remind, events[0].id), (Some((750, 810)), None, 8), "an hour long, no reminder, a fresh id");
        assert_eq!((events[1].repeat, events[1].swatch, events[1].remind, events[1].time), (Repeat::Never, Swatch::Gold, Some(0), None));
        assert_ne!(events[2].id, 7, "a doubled id is made its own");
        assert_eq!(parse("[[event]\ntitle ="), None, "a file mid-edit");
    }
}

//! Calendar events — persisted as plain text at
//! `~/.lantern/config/command-center/events.toml`.
//!
//! Format (one event per line, hand-editable):
//!
//!   # comments allowed
//!   2026-04-30|Dentist appointment
//!   2026-05-01|Lunch with Sam
//!   2026-05-01|Code review @ 3pm
//!
//! Multiple events on the same date = multiple lines. We don't use the
//! `toml` crate — flat key|value lines are friendlier to grep/edit and
//! Lantern prefers minimal deps. The `.toml` extension is just file
//! convention so editors highlight `#` comments.

use std::path::PathBuf;

use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct Event {
    pub date: NaiveDate,
    pub title: String,
}

pub struct Events {
    items: Vec<Event>,
    path: PathBuf,
}

impl Events {
    pub fn load() -> Self {
        let path = config_path();
        let items = read_or_init(&path);
        Self { items, path }
    }

    /// All events on a given date, in insertion order.
    pub fn on_date(&self, date: NaiveDate) -> impl Iterator<Item = &Event> {
        self.items.iter().filter(move |e| e.date == date)
    }

    /// True if any event exists on the given date.
    #[allow(dead_code)] // wired up by Phase 3 day-detail panel
    pub fn has_events(&self, date: NaiveDate) -> bool {
        self.items.iter().any(|e| e.date == date)
    }

    /// Add a new event. Persists immediately.
    #[allow(dead_code)] // wired up by Phase 3 day-detail panel
    pub fn add(&mut self, date: NaiveDate, title: String) {
        let title = title.trim().to_string();
        if title.is_empty() {
            return;
        }
        self.items.push(Event { date, title });
        self.save();
    }

    /// Remove the event at the given index *within* the on-date list
    /// for `date`. Returns true if something was removed. Persists.
    #[allow(dead_code)] // wired up by Phase 3 day-detail panel
    pub fn remove_at(&mut self, date: NaiveDate, idx_in_date: usize) -> bool {
        let mut seen = 0usize;
        let mut to_remove: Option<usize> = None;
        for (i, e) in self.items.iter().enumerate() {
            if e.date == date {
                if seen == idx_in_date {
                    to_remove = Some(i);
                    break;
                }
                seen += 1;
            }
        }
        if let Some(i) = to_remove {
            self.items.remove(i);
            self.save();
            true
        } else {
            false
        }
    }

    fn save(&self) {
        let mut s = String::new();
        s.push_str("# Lantern Command Center events. Format: YYYY-MM-DD|Title\n");
        s.push_str("# One event per line. Lines starting with # are ignored.\n");
        for e in &self.items {
            s.push_str(&format!("{}|{}\n", e.date.format("%Y-%m-%d"), e.title));
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = std::fs::write(&self.path, s) {
            tracing::warn!("events: failed to save {}: {err}", self.path.display());
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".lantern/config/command-center/events.toml")
}

fn read_or_init(path: &PathBuf) -> Vec<Event> {
    match std::fs::read_to_string(path) {
        Ok(s) => parse(&s),
        Err(_) => {
            // Seed an empty file so users can find it for hand-editing.
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(
                path,
                "# Lantern Command Center events. Format: YYYY-MM-DD|Title\n",
            );
            Vec::new()
        }
    }
}

fn parse(s: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((date_s, title)) = line.split_once('|') else {
            tracing::warn!("events: skipping malformed line: {line}");
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(date_s.trim(), "%Y-%m-%d") else {
            tracing::warn!("events: skipping bad date: {date_s}");
            continue;
        };
        let title = title.trim().to_string();
        if title.is_empty() {
            continue;
        }
        out.push(Event { date, title });
    }
    out
}

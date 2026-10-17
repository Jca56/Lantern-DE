//! The calendar, behind the clock at the bar's right end. The clock is
//! its face (`face`): a click opens the popup above it in the overlay, the
//! pointer resting on it a moment shows a card with the whole date and
//! what's left of today, and a dot beside it says today has events still
//! to come. The popup (`popup`) has the time and date big, the month (the
//! wheel over it turns the months, a day's dots its events' colours) and
//! the picked day's events; `+ New event`, a double click on a day, or a
//! click on an event turns it into the form that makes or changes one
//! (`form`).
//!
//! The events are the bar's own, in a file that reads and edits by hand
//! (`store`, `file`), followed live as it changes. Their reminders are said by a
//! thread of their own, on time whether the bar is drawing or not
//! (`remind`). Which day weeks start on and the dot are System Settings'
//! (`[control_center]` `calendar_week_start`, `calendar_dot`).

pub mod date;
pub mod face;
mod file;
mod form;
pub mod popup;
mod remind;
pub mod store;
#[cfg(test)]
mod tests;
mod widgets;

pub use date::{Date, WeekStart};

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_math::Rect;
use lntrn_sys::watch::Watcher;

use crate::clock::Local;
use store::{Book, Shared};

/// How long a burst of writes to the file is let settle before it is read.
const SETTLE: Duration = Duration::from_millis(100);

/// `[control_center]`'s `calendar_*` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarOptions {
    pub week_start: WeekStart,
    /// The dot beside the clock while today has events to come.
    pub dot: bool,
}

impl Default for CalendarOptions {
    fn default() -> Self {
        CalendarOptions { week_start: WeekStart::Sunday, dot: true }
    }
}

pub struct Calendar {
    pub book: Shared,
    /// The popup is up.
    pub popup: bool,
    /// The card over the clock is up.
    pub tip: bool,
    /// The month shown (its first day), and the day picked in it.
    view: Date,
    picked: Date,
    /// The event being made or changed, in the month's place.
    form: Option<form::Form>,
    /// The clock's face as last drawn, in the bar's pixels.
    face: Option<Rect>,
    /// Since when the pointer rests on the clock (the frame clock's
    /// seconds); and a press there keeps the card away until it leaves.
    resting: Option<f64>,
    hushed: bool,
    /// The wheel over the month, short of a whole notch; how far the
    /// day's list is scrolled.
    wheel: f64,
    scroll: f64,
    options: CalendarOptions,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar::with_book(Book::load())
    }
}

impl Calendar {
    pub fn with_book(book: Book) -> Calendar {
        let today = today();
        Calendar { book: Arc::new(Mutex::new(book)), popup: false, tip: false, view: today.first(), picked: today, form: None, face: None, resting: None, hushed: false, wheel: 0.0, scroll: 0.0, options: CalendarOptions::default() }
    }

    /// Follow the file, and start saying reminders.
    pub fn start(&self, waker: Waker) {
        watch(self.book.clone(), waker);
        remind::start(self.book.clone());
    }

    pub fn follow(&mut self, options: CalendarOptions) {
        self.options = options;
    }

    /// Open the popup on today, or close it when it is up.
    pub fn toggle(&mut self) {
        if self.popup {
            self.close();
            return;
        }
        let today = today();
        (self.popup, self.tip, self.view, self.picked, self.form, self.scroll) = (true, false, today.first(), today, None, 0.0);
    }

    /// The popup goes, and any event half made with it.
    pub fn close(&mut self) {
        (self.popup, self.form) = (false, None);
    }

    /// Where the clock's face was last drawn, in the bar's pixels.
    pub fn face(&self) -> Option<Rect> {
        self.face
    }

    /// Goes up with every change to the events.
    pub fn revision(&self) -> u64 {
        store::lock(&self.book).revision()
    }

    /// The popup or the card is up.
    pub fn up(&self) -> bool {
        self.popup || self.tip
    }

    /// Pick `day`, the month turning to it.
    fn pick(&mut self, day: Date) {
        if day != self.picked {
            self.scroll = 0.0;
        }
        (self.picked, self.view) = (day, day.first());
    }
}

/// The day the local clock is on.
pub fn today() -> Date {
    Local::now().map_or(Date { year: 1970, month: 1, day: 1 }, |t| Date::of(&t))
}

/// Minutes into the day on the local clock.
pub fn minute_now() -> u32 {
    Local::now().map_or(0, |t| t.hour * 60 + t.minute)
}

/// Read the events' file again whenever it changes, from a thread of its
/// own, waking the loop when they did. Our own saves come back here too,
/// and change nothing.
fn watch(book: Shared, waker: Waker) {
    let Some(path) = store::lock(&book).path().cloned() else { return };
    let Some(dir) = path.parent().map(Path::to_path_buf) else { return };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log_warn!("calendar: no {}: {e}", dir.display());
    }
    let mut watcher = match Watcher::new().and_then(|mut w| w.watch(&dir, false).map(|()| w)) {
        Ok(w) => w,
        Err(e) => {
            log_warn!("calendar: can't watch {}: {e}; edits by hand show after a restart", dir.display());
            return;
        }
    };
    let spawned = std::thread::Builder::new().name("calendar.toml".into()).spawn(move || {
        loop {
            if !crate::settings::touches(&watcher.wait(None), &path) {
                continue;
            }
            std::thread::sleep(SETTLE);
            watcher.poll();
            if store::lock(&book).reload() {
                waker.wake();
            }
        }
    });
    if let Err(e) = spawned {
        log_warn!("calendar: no watcher thread: {e}; edits by hand show after a restart");
    }
}

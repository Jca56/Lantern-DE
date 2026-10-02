//! When the Audio section needs frames drawn, and its status message.
//!
//! The main loop draws at 60 fps for as long as `AudioEdit::busy` says so
//! and otherwise only on input. `busy` used to be "a worker thread exists",
//! which for the artwork picker meant a full GPU frame every 16 ms for as
//! long as the user browsed for a picture. It is now "a frame would show
//! something new, or soon will".

use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{AudioEdit, Slot};

pub(super) const STATUS_TTL: Duration = Duration::from_secs(3);

/// How long the loop is kept drawing after a frame that something else
/// caused while the picker is open. The picker closing hands focus back to
/// this window, which draws a frame; the picker's process exits a moment
/// later, and this covers that moment.
const PICK_WATCH: Duration = Duration::from_millis(1500);

/// Line length a message is first broken at. At the 18 px status font this
/// fits the dialog's width for ordinary text; the draw code measures the
/// result and re-breaks it if it does not.
const WRAP_CHARS: usize = 54;

pub(super) struct Status {
    pub lines: Vec<String>,
    pub is_err: bool,
    at: Instant,
}

impl Status {
    pub fn new(msg: &str, is_err: bool) -> Self {
        Self {
            lines: wrap(msg, |line| line.chars().count() <= WRAP_CHARS),
            is_err,
            at: Instant::now(),
        }
    }

    /// Errors stay until the next action; the rest clears by itself.
    fn expired(&self, now: Instant) -> bool {
        !self.is_err && now.duration_since(self.at) > STATUS_TTL
    }

    /// Break the message again with a real measure of what fits.
    pub fn rewrap(&mut self, fits: impl FnMut(&str) -> bool) {
        self.lines = wrap(&self.lines.join(" "), fits);
    }
}

/// Greedy word wrap. A single word that does not fit gets a line of its own.
pub(super) fn wrap(msg: &str, mut fits: impl FnMut(&str) -> bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in msg.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
            continue;
        }
        let candidate = format!("{line} {word}");
        if fits(&candidate) {
            line = candidate;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Decides when to keep drawing while the artwork picker is open.
///
/// Nothing can wake the loop when the picker thread finishes (that needs a
/// descriptor in the loop's poll set), so the result is collected by frames
/// drawn for other reasons, and each such frame is followed by a short
/// burst of our own in case the result is about to land.
#[derive(Default)]
pub(super) struct PickWatch {
    until: Option<Instant>,
    /// The frame now being drawn was asked for by `busy`, not by input.
    asked: Cell<bool>,
}

impl PickWatch {
    /// Called once per drawn frame, before `wants`.
    pub fn frame_started(&mut self, picking: bool, now: Instant) {
        let ours = self.asked.replace(false);
        if !picking {
            self.until = None;
        } else if !ours {
            self.until = Some(now + PICK_WATCH);
        }
    }

    /// `child_done`: the picker has exited and its result is being prepared.
    pub fn wants(&self, child_done: bool, now: Instant) -> bool {
        child_done || self.until.is_some_and(|t| now < t)
    }

    pub fn asked(&self, yes: bool) {
        self.asked.set(yes);
    }
}

/// Tag saves running right now, in any dialog. A save rewrites a music
/// file; the window does not close under one (op_dialogs.rs).
static SAVES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Held by a save thread for as long as it runs.
pub(super) struct SaveInFlight;

impl SaveInFlight {
    pub fn begin() -> Self {
        SAVES.fetch_add(1, Ordering::SeqCst);
        SaveInFlight
    }
}

impl Drop for SaveInFlight {
    fn drop(&mut self) {
        SAVES.fetch_sub(1, Ordering::SeqCst);
        // A window waiting to close looks again.
        crate::bg::wake();
    }
}

/// A tag save is still writing its file.
pub fn save_in_flight() -> bool {
    SAVES.load(Ordering::SeqCst) > 0
}

fn ready<T>(slot: &Slot<T>) -> bool {
    slot.lock().map(|g| g.is_some()).unwrap_or(false)
}

impl AudioEdit {
    /// True while the next frame should be drawn without waiting for input.
    /// Called by the main loop once at the end of every drawn frame.
    pub fn busy(&self) -> bool {
        let now = Instant::now();
        // (A "Saved" note needs no frames until it runs out: the loop asks
        // `frame_wanted` on each of its idle wake-ups and draws one then.)
        let yes = self.decoding
            || self.saving
            || self.frame_wanted()
            || self.relayout.replace(false)
            || (self.picking
                && self
                    .watch
                    .wants(self.pick_child_done.load(Ordering::Relaxed), now));
        self.watch.asked(yes);
        yes
    }

    /// True when a drawn frame would show something new right now: a worker
    /// has delivered, or the status has run out. Cheap (no GPU, no I/O), so
    /// a loop can ask on every wake-up, drawn frame or not.
    pub fn frame_wanted(&self) -> bool {
        (self.picking && !self.saving && ready(&self.pick))
            || (self.decoding && ready(&self.decode))
            || (self.saving && ready(&self.save))
            || self
                .status
                .as_ref()
                .is_some_and(|s| s.expired(Instant::now()))
    }

    pub(super) fn set_status(&mut self, msg: &str, is_err: bool) {
        self.status = Some(Status::new(msg, is_err));
    }

    /// Back to no message, or to the standing notice that this file's tags
    /// cannot be saved.
    pub(super) fn reset_status(&mut self) {
        self.status = self.meta.write_blocker.as_ref().map(|why| {
            Status::new(
                &format!("{why}. Fox shows these tags but will not save changes to them."),
                true,
            )
        });
    }

    pub(super) fn expire_status(&mut self) {
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.expired(Instant::now()))
        {
            self.reset_status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_words() {
        let short = |l: &str| l.chars().count() <= 10;
        assert_eq!(wrap("one two three four", short), ["one two", "three four"]);
        assert_eq!(wrap("  spaced   out  ", short), ["spaced out"]);
        assert_eq!(wrap("", short), Vec::<String>::new());
        // A word longer than a line is not split, and not dropped.
        assert_eq!(
            wrap("a supercalifragilistic word", short),
            ["a", "supercalifragilistic", "word"]
        );
        // Counted in characters, not bytes.
        assert_eq!(wrap("ééééé ééééé", |l| l.chars().count() <= 11).len(), 1);
    }

    #[test]
    fn status_message_lines_and_expiry() {
        let s = Status::new(&"word ".repeat(30), true);
        assert!(s.lines.len() > 1);
        assert!(s.lines.iter().all(|l| l.chars().count() <= WRAP_CHARS));
        assert_eq!(s.lines.join(" "), "word ".repeat(30).trim());
        let later = Instant::now() + STATUS_TTL + Duration::from_secs(1);
        assert!(!s.expired(later), "errors stay");
        let ok = Status::new("Saved", false);
        assert!(!ok.expired(Instant::now()));
        assert!(ok.expired(later));

        let mut s = Status::new("one two three four", true);
        s.rewrap(|l| l.len() <= 9);
        assert_eq!(s.lines, ["one two", "three", "four"]);
    }

    #[test]
    fn picker_watch_is_a_short_burst_not_a_constant_redraw() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut w = PickWatch::default();

        // The frame in which the picker was opened came from a click.
        w.frame_started(true, t0);
        assert!(w.wants(false, t0));
        // Our own frames follow; they do not extend the burst.
        let mut now = t0;
        let mut frames = 0;
        while w.wants(false, now) {
            w.asked(true);
            now += ms(16);
            w.frame_started(true, now);
            frames += 1;
            assert!(frames < 200, "never stopped drawing");
        }
        assert!(now - t0 <= PICK_WATCH + ms(16));
        // Idle: nothing asks for frames while the user browses.
        w.asked(false);
        assert!(!w.wants(false, now + ms(60_000)));

        // The picker closes; focus comes back and a frame is drawn for it.
        let back = now + ms(60_000);
        w.frame_started(true, back);
        assert!(w.wants(false, back + ms(100)));
        // Once the child has exited, drawing continues until the result is
        // collected, however long the picture takes to prepare.
        assert!(w.wants(true, back + ms(60_000)));

        // Result collected (`picking` false): the watch is over.
        w.frame_started(false, back + ms(200));
        assert!(!w.wants(false, back + ms(201)));
    }
}

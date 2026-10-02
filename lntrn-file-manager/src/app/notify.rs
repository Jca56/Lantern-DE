//! Things that end without an input event and still have to show: the short
//! note in the status bar (the result of an undo), and work that finished
//! on a thread of its own (an archive packed or unpacked) after which the
//! listing is stale. The main loop asks `idle_tick` on every wake-up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::App;

/// Long enough to read a line, short enough not to sit over the status bar.
const NOTE_SHOWN_FOR: Duration = Duration::from_secs(6);

impl App {
    /// Show one line in the status bar for a few seconds. For results that
    /// need no answer; anything the user must read goes to `show_notice`.
    pub fn set_status_note(&mut self, text: impl Into<String>) {
        self.status_note = Some((text.into(), Instant::now() + NOTE_SHOWN_FOR));
    }

    /// The note to draw this frame, if one is still current.
    pub fn status_note(&self) -> Option<&str> {
        // A format in progress outranks a passing note: with its dialog
        // hidden this line is the only sign of it, and it says the one
        // thing that matters (do not unplug).
        if let Some(note) = self.format_note() {
            return Some(note);
        }
        match &self.status_note {
            Some((text, until)) if Instant::now() < *until => Some(text),
            _ => None,
        }
    }

    /// A flag a background thread sets when it has changed the folder on
    /// disk; the next `idle_tick` re-lists. The watcher only sees the shown
    /// folder itself, not the expanded subfolders of the tree view.
    pub(crate) fn refresh_flag(&self) -> Arc<AtomicBool> {
        self.refresh_wanted.clone()
    }

    /// Called by the main loop on every wake-up, also when no frame is due.
    /// True when the window has to be drawn again.
    pub fn idle_tick(&mut self) -> bool {
        let mut redraw = false;
        if self
            .status_note
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.status_note = None;
            redraw = true;
        }
        if self.refresh_wanted.swap(false, Ordering::SeqCst) {
            self.reload();
            // What asked may have been a folder icon change.
            if let Some(props) = self.properties.as_mut() {
                props.reread_look();
            }
            redraw = true;
        }
        // Results of work that ran off this thread (app/background.rs).
        redraw |= self.poll_background();
        // Unfinished copies a listing came across (copy_tree.rs).
        let leftovers = crate::copy_tree::take_stale();
        if !leftovers.is_empty() {
            let asked = self.op_dialogs.len();
            self.ask_about_leftovers(leftovers);
            redraw |= self.op_dialogs.len() != asked;
        }
        // The sync engine reports on its own schedule (cloud_ui.rs).
        redraw |= self.poll_cloud();
        redraw
    }
}

//! The password being typed and what each key does to it. Plain state with
//! no Wayland or PAM in it, so the rules can be tested on their own.

use crate::auth::wipe;
use crate::keyboard::Key;

/// Room for any sane password up front. The buffer never grows past this, so
/// it never reallocates and leaves a stale copy of the password on the heap.
const CAPACITY: usize = 256;

pub struct Entry {
    password: String,
    /// A check is running in the background.
    checking: bool,
    /// Enter was pressed and hasn't been acted on yet.
    submit: bool,
    /// Shown under the field until the next edit.
    error: Option<String>,
}

impl Entry {
    pub fn new() -> Self {
        Self {
            password: String::with_capacity(CAPACITY),
            checking: false,
            submit: false,
            error: None,
        }
    }

    /// Apply one key press. Returns whether anything on screen changed.
    ///
    /// Typing stays live while a check runs: a wrong password holds PAM for
    /// about two seconds, and whoever notices the typo is already retyping.
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Enter => {
                // Acted on by `take_submission`, which waits out a running
                // check, so Enter pressed during one isn't lost either.
                self.submit = !self.password.is_empty();
                false
            }
            Key::Text(text) => {
                if self.password.len() + text.len() > CAPACITY {
                    return false;
                }
                self.password.push_str(&text);
                self.edited();
                true
            }
            Key::Backspace => {
                let changed = self.password.pop().is_some() || self.error.is_some();
                self.edited();
                changed
            }
            Key::Escape => {
                let changed = !self.password.is_empty() || self.error.is_some();
                wipe(&mut self.password);
                self.edited();
                changed
            }
        }
    }

    /// Any edit clears the error, and cancels an Enter that was still waiting:
    /// what gets submitted is always exactly what Enter was pressed on.
    fn edited(&mut self) {
        self.submit = false;
        self.error = None;
    }

    /// The password to check, once Enter has asked for it and no check is
    /// running. The field starts over empty; the caller owns what it gets.
    pub fn take_submission(&mut self) -> Option<String> {
        if !self.submit || self.checking || self.password.is_empty() {
            return None;
        }
        self.submit = false;
        self.checking = true;
        self.error = None;
        Some(std::mem::replace(&mut self.password, String::with_capacity(CAPACITY)))
    }

    /// The running check refused the password.
    pub fn denied(&mut self, message: String) {
        self.checking = false;
        self.error = Some(message);
    }

    /// Characters typed so far.
    pub fn len(&self) -> usize {
        self.password.chars().count()
    }

    pub fn checking(&self) -> bool {
        self.checking
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        wipe(&mut self.password);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(entry: &mut Entry, text: &str) {
        for c in text.chars() {
            entry.key(Key::Text(c.to_string()));
        }
    }

    #[test]
    fn typing_and_erasing() {
        let mut entry = Entry::new();
        type_text(&mut entry, "héllo");
        assert_eq!(entry.len(), 5);

        assert!(entry.key(Key::Backspace));
        assert_eq!(entry.len(), 4);

        assert!(entry.key(Key::Escape));
        assert_eq!(entry.len(), 0);
        assert!(!entry.key(Key::Backspace), "nothing left to erase");
        assert!(!entry.key(Key::Escape));
    }

    #[test]
    fn enter_submits_what_was_typed() {
        let mut entry = Entry::new();
        entry.key(Key::Enter);
        assert_eq!(entry.take_submission(), None, "an empty field submits nothing");

        type_text(&mut entry, "hunter2");
        assert_eq!(entry.take_submission(), None, "not before Enter");
        entry.key(Key::Enter);
        assert_eq!(entry.take_submission().as_deref(), Some("hunter2"));
        assert!(entry.checking());
        assert_eq!(entry.len(), 0);
        assert_eq!(entry.take_submission(), None, "one Enter, one check");
    }

    #[test]
    fn typing_during_a_check_is_kept() {
        let mut entry = Entry::new();
        type_text(&mut entry, "wrnog");
        entry.key(Key::Enter);
        entry.take_submission().unwrap();

        // Retyping while PAM is still chewing on the typo.
        type_text(&mut entry, "right");
        assert_eq!(entry.len(), 5);

        entry.denied("Incorrect password".into());
        assert_eq!(entry.len(), 5, "the retyped password survives the verdict");
        assert_eq!(entry.error(), Some("Incorrect password"));

        entry.key(Key::Enter);
        assert_eq!(entry.take_submission().as_deref(), Some("right"));
    }

    #[test]
    fn enter_during_a_check_goes_through_once_it_ends() {
        let mut entry = Entry::new();
        type_text(&mut entry, "wrnog");
        entry.key(Key::Enter);
        entry.take_submission().unwrap();

        type_text(&mut entry, "right");
        entry.key(Key::Enter);
        assert_eq!(entry.take_submission(), None, "one check at a time");

        entry.denied("Incorrect password".into());
        assert_eq!(entry.take_submission().as_deref(), Some("right"));
        assert_eq!(entry.error(), None);
    }

    #[test]
    fn an_edit_cancels_a_waiting_enter() {
        let mut entry = Entry::new();
        type_text(&mut entry, "first");
        entry.key(Key::Enter);
        entry.take_submission().unwrap();

        type_text(&mut entry, "secon");
        entry.key(Key::Enter);
        type_text(&mut entry, "d"); // still typing after all

        entry.denied("Incorrect password".into());
        assert_eq!(entry.take_submission(), None);
        entry.key(Key::Enter);
        assert_eq!(entry.take_submission().as_deref(), Some("second"));
    }

    #[test]
    fn the_error_clears_on_the_next_edit() {
        let mut entry = Entry::new();
        type_text(&mut entry, "nope");
        entry.key(Key::Enter);
        entry.take_submission().unwrap();
        entry.denied("Incorrect password".into());

        assert!(entry.key(Key::Backspace), "clearing the error is a visible change");
        assert_eq!(entry.error(), None);
    }

    #[test]
    fn the_buffer_never_grows() {
        let mut entry = Entry::new();
        let before = entry.password.as_ptr();
        for _ in 0..CAPACITY + 50 {
            entry.key(Key::Text("x".into()));
        }
        assert_eq!(entry.len(), CAPACITY);
        assert_eq!(entry.password.as_ptr(), before, "reallocated");
    }
}

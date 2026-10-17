//! `[session]`: what the session runs. Its `panel` is the one the session
//! manager keeps running, the Control Center or the Command Center, never
//! both, swapping them the moment it changes; a Super tap summons it.

use lntrn_props::props;

/// The panels: the word stored, and how the page shows it.
pub const PANELS: [(&str, &str); 2] = [("control_center", "Control Center"), ("command_center", "Command Center")];

props! {
    /// What the session runs.
    pub struct Session {
        /// One of [`PANELS`]' words.
        pub panel: String = "control_center".to_owned() => { id: 1 },
    }
}

impl Session {
    pub fn clamp(&mut self) {
        if !PANELS.iter().any(|(word, _)| *word == self.panel) {
            self.panel = "control_center".to_owned();
        }
    }
}

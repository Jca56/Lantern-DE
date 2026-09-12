//! Bells from the terminals. A program that rings while its terminal is
//! hidden, or while the window is not focused, wants a look: the tab
//! wears a dot and the status bar says so until the terminal shows in a
//! focused window. A toast names it when the window is focused; a
//! desktop notification goes out when it is not.

use lntrn_ui::{Shell, ShellRequest};

use crate::app::App;

/// Bells closer than this make one notification.
const NOTIFY_GAP: f64 = 2.0;

impl App {
    /// Settle the bells rung since the last look. Returns whether a mark
    /// changed (the tabs and status bar draw again).
    pub(crate) fn settle_bells(&mut self, shell: &mut Shell<Self>) -> bool {
        let now = shell.state.now;
        let focused = shell.window_focused;
        let mut changed = false;
        let mut toasts = Vec::new();
        for t in &mut self.terminals {
            let seen = focused && t.viewed_at == now;
            if seen && t.attention {
                t.attention = false;
                changed = true;
            }
            if !std::mem::take(&mut t.rang) || seen {
                continue;
            }
            t.attention = true;
            changed = true;
            let title = t.title();
            if focused {
                toasts.push(format!("{title} needs you"));
            } else if now - t.notified_at > NOTIFY_GAP {
                t.notified_at = now;
                crate::launch::notify("Terminal needs you", &title);
            }
        }
        for text in toasts {
            shell.request(self, ShellRequest::Toast(text));
        }
        changed
    }

    /// The status bar's word on terminals wanting a look.
    pub(crate) fn attention_status(&self) -> String {
        let waiting: Vec<String> = self.terminals.iter().filter(|t| t.attention).map(|t| t.title()).collect();
        match waiting.len() {
            0 => String::new(),
            1 => format!("● {} needs you", waiting[0]),
            n => format!("● {n} terminals need you"),
        }
    }
}

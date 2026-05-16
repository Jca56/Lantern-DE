//! Keyboard handling for the Chat tab. Wired in from
//! `layershell::input::handle_keypress` when the active view is Chat.

use crate::search::input::{
    keycode_to_char, KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_HOME,
    KEY_KP_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP,
};
use lntrn_terminal::clipboard::WaylandClipboard;

use super::ChatState;

const SCROLL_STEP: f32 = 60.0;

/// Returns true if the key was consumed.
pub fn handle_key(
    chat: &mut ChatState,
    key: u32,
    shift: bool,
    ctrl: bool,
    caps_lock: bool,
    clipboard: Option<&WaylandClipboard>,
) -> bool {
    // Chord shortcuts.
    if ctrl {
        if let Some(ch) = keycode_to_char(key, false, false).map(|c| c.to_ascii_lowercase()) {
            match ch {
                'n' => { chat.new_thread(); return true; }
                'v' => {
                    if let Some(clip) = clipboard {
                        if let Some(text) = clip.get_text() {
                            chat.draft.insert_str(&text);
                        }
                    }
                    return true;
                }
                'a' => { chat.draft.select_all(); return true; }
                'c' | 'x' => {
                    let selected = chat.draft.selected_text().map(|s| s.to_string());
                    let full = chat.draft.query().to_string();
                    let payload = selected.clone().unwrap_or(full);
                    if !payload.is_empty() {
                        if let Some(clip) = clipboard { clip.set_text(&payload); }
                    }
                    if ch == 'x' && selected.is_some() { chat.draft.insert_str(""); }
                    return true;
                }
                _ => {}
            }
        }
    }

    match key {
        KEY_ENTER | KEY_KP_ENTER => {
            if shift {
                chat.draft.insert_str("\n");
            } else {
                chat.submit_draft();
            }
            true
        }
        KEY_UP => { chat.scroll_messages(-SCROLL_STEP); true }
        KEY_DOWN => { chat.scroll_messages(SCROLL_STEP); true }
        KEY_BACKSPACE | KEY_DELETE | KEY_LEFT | KEY_RIGHT | KEY_HOME | KEY_END => {
            let _ = chat.draft.on_key(key, shift, caps_lock);
            true
        }
        other => {
            let _ = chat.draft.on_key(other, shift, caps_lock);
            true
        }
    }
}


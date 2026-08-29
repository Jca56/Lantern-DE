//! Dialog flows (save-name prompt, unsaved-changes confirms, errors) and the
//! save/new commands they drive. Split from `input.rs` so the pointer/keyboard
//! state machine stays focused on the canvas itself.

use super::editor::{CanvasEditor, DialogKind};
use super::input::CanvasAction;
use super::persist;
use crate::{ZONE_DIALOG_BTN0, ZONE_DIALOG_BTN1, ZONE_DIALOG_BTN2};

const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;

pub fn on_dialog_button(ed: &mut CanvasEditor, zone: u32) -> CanvasAction {
    match ed.dialog {
        Some(DialogKind::SaveName { quit_after }) => match zone {
            ZONE_DIALOG_BTN0 => return confirm_save_name(ed, quit_after),
            ZONE_DIALOG_BTN1 => ed.dialog = None,
            _ => {}
        },
        Some(DialogKind::ConfirmQuit) => match zone {
            ZONE_DIALOG_BTN0 => return request_save(ed, true),
            ZONE_DIALOG_BTN1 => return CanvasAction::Quit,
            ZONE_DIALOG_BTN2 => ed.dialog = None,
            _ => {}
        },
        Some(DialogKind::ConfirmNew) => match zone {
            ZONE_DIALOG_BTN0 => reset_to_new(ed),
            ZONE_DIALOG_BTN1 => ed.dialog = None,
            _ => {}
        },
        Some(DialogKind::Error(_)) => {
            if zone == ZONE_DIALOG_BTN0 {
                ed.dialog = None;
            }
        }
        None => {}
    }
    CanvasAction::None
}

pub fn on_dialog_key(ed: &mut CanvasEditor, key: u32, shift: bool) -> CanvasAction {
    let editing_name = matches!(ed.dialog, Some(DialogKind::SaveName { .. }));
    match key {
        KEY_ESC => ed.dialog = None,
        KEY_ENTER => match ed.dialog {
            Some(DialogKind::SaveName { quit_after }) => return confirm_save_name(ed, quit_after),
            // Enter triggers the primary button: Save, then quit.
            Some(DialogKind::ConfirmQuit) => return request_save(ed, true),
            Some(DialogKind::ConfirmNew) => reset_to_new(ed),
            Some(DialogKind::Error(_)) | None => ed.dialog = None,
        },
        KEY_BACKSPACE if editing_name => {
            if ed.name_cursor > 0 {
                let pos = ed
                    .name_buf
                    .char_indices()
                    .nth(ed.name_cursor - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                ed.name_buf.remove(pos);
                ed.name_cursor -= 1;
            }
        }
        KEY_LEFT if editing_name => ed.name_cursor = ed.name_cursor.saturating_sub(1),
        KEY_RIGHT if editing_name => {
            ed.name_cursor = (ed.name_cursor + 1).min(ed.name_buf.chars().count());
        }
        _ if editing_name => {
            if let Some(ch) = keycode_to_char(key, shift) {
                let pos = ed
                    .name_buf
                    .char_indices()
                    .nth(ed.name_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(ed.name_buf.len());
                ed.name_buf.insert(pos, ch);
                ed.name_cursor += 1;
            }
        }
        _ => {}
    }
    CanvasAction::None
}

// ── Save / new ──────────────────────────────────────────────────────────────

/// Save now if the canvas has a file, otherwise open the name dialog.
pub fn request_save(ed: &mut CanvasEditor, quit_after: bool) -> CanvasAction {
    if let Some(path) = ed.save_path.clone() {
        match persist::save_canvas(&ed.doc, &path) {
            Ok(()) => {
                ed.mark_saved();
                ed.dialog = None;
                if quit_after {
                    return CanvasAction::Quit;
                }
            }
            Err(e) => ed.dialog = Some(DialogKind::Error(format!("Save failed: {e}"))),
        }
    } else {
        ed.name_buf = ed.doc.name.clone();
        ed.name_cursor = ed.name_buf.chars().count();
        ed.dialog = Some(DialogKind::SaveName { quit_after });
    }
    CanvasAction::None
}

fn confirm_save_name(ed: &mut CanvasEditor, quit_after: bool) -> CanvasAction {
    let name = persist::sanitize_name(&ed.name_buf);
    let path = persist::canvases_dir().join(format!("{name}.lcanvas"));
    ed.doc.name = name;
    match persist::save_canvas(&ed.doc, &path) {
        Ok(()) => {
            ed.save_path = Some(path);
            ed.mark_saved();
            ed.dialog = None;
            if quit_after {
                return CanvasAction::Quit;
            }
        }
        Err(e) => ed.dialog = Some(DialogKind::Error(format!("Save failed: {e}"))),
    }
    CanvasAction::None
}

pub fn reset_to_new(ed: &mut CanvasEditor) {
    *ed = CanvasEditor::new_empty();
}

fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    // Same map as the file manager's rename dialog (US layout keycodes).
    let ch = match key {
        2..=11 => {
            if shift {
                b"!@#$%^&*()"[(key - 2) as usize]
            } else {
                b"1234567890"[(key - 2) as usize]
            }
        }
        12 => {
            if shift {
                b'_'
            } else {
                b'-'
            }
        }
        13 => {
            if shift {
                b'+'
            } else {
                b'='
            }
        }
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        39 => {
            if shift {
                b':'
            } else {
                b';'
            }
        }
        40 => {
            if shift {
                b'"'
            } else {
                b'\''
            }
        }
        51 => {
            if shift {
                b'<'
            } else {
                b','
            }
        }
        52 => {
            if shift {
                b'>'
            } else {
                b'.'
            }
        }
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}

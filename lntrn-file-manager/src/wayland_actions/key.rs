use lntrn_ui::gpu::{ContextMenu, WaylandPopupBackend};

use crate::app::App;
use crate::settings::Settings;
use crate::wayland::State;

// Linux keycodes
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;
const KEY_A: u32 = 30;
const KEY_C: u32 = 46;
const KEY_V: u32 = 47;
const KEY_X: u32 = 45;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;
const KEY_Z: u32 = 44;
const KEY_F2: u32 = 60;
const KEY_DELETE: u32 = 111;
const KEY_HOME: u32 = 102;
const KEY_END: u32 = 107;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;

/// Map an evdev keycode to a character for filename entry.
fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    // Number row: keycodes 2=1, 3=2, ..., 10=9, 11=0
    let ch = match key {
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift {
                b"!@#$%^&*()"[(key - 2) as usize]
            } else {
                base
            }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        // Letters (a=30..z)
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        // Punctuation
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ', // space
        _ => return None,
    };
    Some(ch as char)
}

pub(crate) fn handle_key(
    app: &mut App, _settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    key: u32, ctrl: bool, shift: bool,
    running: &mut bool,
) {
    // Conflict dialog — ESC = cancel paste, Enter = Replace.
    if app.conflict_dialog.is_some() {
        let _ = ctrl;
        let _ = shift;
        match key {
            KEY_ESC => app.cancel_paste(),
            KEY_ENTER => app.resolve_conflict(crate::conflict::ConflictAction::Replace),
            _ => {}
        }
        return;
    }

    // Sudo password modal — captures keys until dismissed.
    if app.sudo_prompt.is_some() {
        let _ = ctrl;
        match key {
            KEY_ESC => app.cancel_sudo_prompt(),
            KEY_ENTER => app.submit_sudo_prompt(),
            KEY_BACKSPACE => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    if p.cursor > 0 {
                        let byte_pos = p.password.char_indices().nth(p.cursor - 1).map(|(i, _)| i).unwrap_or(0);
                        p.password.remove(byte_pos);
                        p.cursor -= 1;
                    }
                }
            }
            KEY_DELETE => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    let char_len = p.password.chars().count();
                    if p.cursor < char_len {
                        let byte_pos = p.password.char_indices().nth(p.cursor).map(|(i, _)| i).unwrap_or(p.password.len());
                        p.password.remove(byte_pos);
                    }
                }
            }
            KEY_LEFT => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    if p.cursor > 0 { p.cursor -= 1; }
                }
            }
            KEY_RIGHT => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    if p.cursor < p.password.chars().count() { p.cursor += 1; }
                }
            }
            KEY_HOME => {
                if let Some(p) = app.sudo_prompt.as_mut() { p.cursor = 0; }
            }
            KEY_END => {
                if let Some(p) = app.sudo_prompt.as_mut() { p.cursor = p.password.chars().count(); }
            }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    if let Some(p) = app.sudo_prompt.as_mut() {
                        let byte_pos = p.password.char_indices().nth(p.cursor).map(|(i, _)| i).unwrap_or(p.password.len());
                        p.password.insert(byte_pos, ch);
                        p.cursor += 1;
                    }
                }
            }
        }
        return;
    }

    // Cloud login dialog — captures keys until dismissed.
    if app.cloud_login.is_some() {
        let _ = ctrl;
        match key {
            KEY_ESC => {
                app.cloud_login = None;
            }
            KEY_ENTER => app.submit_cloud_login(),
            KEY_TAB => {
                if let Some(d) = app.cloud_login.as_mut() {
                    d.focus_next();
                }
            }
            KEY_BACKSPACE => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (buf, cur) = d.focused_buf_mut();
                    if *cur > 0 {
                        let byte_pos = buf.char_indices().nth(*cur - 1).map(|(i, _)| i).unwrap_or(0);
                        buf.remove(byte_pos);
                        *cur -= 1;
                    }
                }
            }
            KEY_DELETE => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (buf, cur) = d.focused_buf_mut();
                    let char_len = buf.chars().count();
                    if *cur < char_len {
                        let byte_pos = buf.char_indices().nth(*cur).map(|(i, _)| i).unwrap_or(buf.len());
                        buf.remove(byte_pos);
                    }
                }
            }
            KEY_LEFT => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (_buf, cur) = d.focused_buf_mut();
                    if *cur > 0 { *cur -= 1; }
                }
            }
            KEY_RIGHT => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (buf, cur) = d.focused_buf_mut();
                    if *cur < buf.chars().count() { *cur += 1; }
                }
            }
            KEY_HOME => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (_buf, cur) = d.focused_buf_mut();
                    *cur = 0;
                }
            }
            KEY_END => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (buf, cur) = d.focused_buf_mut();
                    *cur = buf.chars().count();
                }
            }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    if let Some(d) = app.cloud_login.as_mut() {
                        let (buf, cur) = d.focused_buf_mut();
                        let byte_pos = buf.char_indices().nth(*cur).map(|(i, _)| i).unwrap_or(buf.len());
                        buf.insert(byte_pos, ch);
                        *cur += 1;
                    }
                }
            }
        }
        return;
    }

    // Drive dialog — ESC dismisses, ENTER confirms (Format only)
    if app.drive_dialog.is_some() {
        match key {
            KEY_ESC => app.dismiss_drive_dialog(),
            KEY_ENTER => {
                if matches!(app.drive_dialog, Some(crate::dialogs::DriveDialog::ConfirmFormat { .. })) {
                    app.confirm_drive_format();
                } else {
                    app.dismiss_drive_dialog();
                }
            }
            _ => {}
        }
        return;
    }

    // Drop confirmation modal — ESC cancels
    if app.pending_drop.is_some() {
        if key == KEY_ESC {
            app.pending_drop = None;
        }
        return;
    }

    if context_menu.is_open() {
        if key == KEY_ESC {
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        return;
    }

    // ── Search mode ──────────────────────────────────────────────────
    if app.searching {
        match key {
            KEY_ESC => app.close_search(),
            KEY_BACKSPACE => {
                if app.search_cursor > 0 {
                    app.search_cursor -= 1;
                    app.search_buf.remove(app.search_cursor);
                    app.run_search();
                }
            }
            KEY_LEFT => {
                if app.search_cursor > 0 { app.search_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.search_cursor < app.search_buf.len() { app.search_cursor += 1; }
            }
            KEY_HOME => app.search_cursor = 0,
            KEY_END => app.search_cursor = app.search_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.search_buf.insert(app.search_cursor, ch);
                    app.search_cursor += 1;
                    app.run_search();
                }
            }
        }
        return;
    }

    // ── Path bar editing ─────────────────────────────────────────────
    if app.path_editing {
        if ctrl {
            match key {
                KEY_A => {
                    let len = app.path_buf.chars().count();
                    app.path_selection = Some((0, len));
                    app.path_cursor = len;
                }
                KEY_C => {
                    if let Some(text) = app.path_selected_text() {
                        if let Some(clip) = &app.wayland_clipboard {
                            clip.set_text(&text);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        // Helper: delete selected range and place cursor at selection start
        let delete_selection = |app: &mut App| -> bool {
            if let Some((a, b)) = app.path_selection.take() {
                let s = a.min(b);
                let e = a.max(b);
                if s != e {
                    let byte_start = app.path_buf.char_indices().nth(s).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                    let byte_end = app.path_buf.char_indices().nth(e).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                    app.path_buf.replace_range(byte_start..byte_end, "");
                    app.path_cursor = s;
                    return true;
                }
            }
            false
        };
        match key {
            KEY_ENTER => app.commit_path_edit(),
            KEY_ESC => app.cancel_path_edit(),
            KEY_BACKSPACE => {
                if !delete_selection(app) && app.path_cursor > 0 {
                    let byte_pos = app.path_buf.char_indices().nth(app.path_cursor - 1).map(|(i,_)| i).unwrap_or(0);
                    app.path_buf.remove(byte_pos);
                    app.path_cursor -= 1;
                }
                app.path_selection = None;
            }
            KEY_DELETE => {
                if !delete_selection(app) {
                    let char_len = app.path_buf.chars().count();
                    if app.path_cursor < char_len {
                        let byte_pos = app.path_buf.char_indices().nth(app.path_cursor).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                        app.path_buf.remove(byte_pos);
                    }
                }
                app.path_selection = None;
            }
            KEY_LEFT => {
                if app.path_cursor > 0 { app.path_cursor -= 1; }
                app.path_selection = None;
            }
            KEY_RIGHT => {
                let char_len = app.path_buf.chars().count();
                if app.path_cursor < char_len { app.path_cursor += 1; }
                app.path_selection = None;
            }
            KEY_HOME => { app.path_cursor = 0; app.path_selection = None; }
            KEY_END => { app.path_cursor = app.path_buf.chars().count(); app.path_selection = None; }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    delete_selection(app);
                    let byte_pos = app.path_buf.char_indices().nth(app.path_cursor).map(|(i,_)| i).unwrap_or(app.path_buf.len());
                    app.path_buf.insert(byte_pos, ch);
                    app.path_cursor += 1;
                    app.path_selection = None;
                }
            }
        }
        return;
    }

    // ── Rename mode ─────────────────────────────────────────────────
    if app.renaming.is_some() {
        if ctrl {
            match key {
                KEY_A => {
                    let len = app.rename_buf.len();
                    app.rename_selection = Some((0, len));
                    app.rename_cursor = len;
                }
                _ => {}
            }
            return;
        }
        match key {
            KEY_ENTER => app.commit_rename(),
            KEY_ESC => app.cancel_rename(),
            KEY_BACKSPACE => {
                if !app.rename_delete_selection() && app.rename_cursor > 0 {
                    app.rename_cursor -= 1;
                    app.rename_buf.remove(app.rename_cursor);
                }
                app.rename_selection = None;
            }
            KEY_DELETE => {
                if !app.rename_delete_selection() && app.rename_cursor < app.rename_buf.len() {
                    app.rename_buf.remove(app.rename_cursor);
                }
                app.rename_selection = None;
            }
            KEY_LEFT => {
                if let Some((a, b)) = app.rename_selection.take() {
                    app.rename_cursor = a.min(b);
                } else if app.rename_cursor > 0 {
                    app.rename_cursor -= 1;
                }
            }
            KEY_RIGHT => {
                if let Some((a, b)) = app.rename_selection.take() {
                    app.rename_cursor = a.max(b).min(app.rename_buf.len());
                } else if app.rename_cursor < app.rename_buf.len() {
                    app.rename_cursor += 1;
                }
            }
            KEY_HOME => { app.rename_cursor = 0; app.rename_selection = None; }
            KEY_END => { app.rename_cursor = app.rename_buf.len(); app.rename_selection = None; }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.rename_delete_selection();
                    app.rename_buf.insert(app.rename_cursor, ch);
                    app.rename_cursor += ch.len_utf8();
                    app.rename_selection = None;
                }
            }
        }
        return;
    }

    // ── Pick mode: save name editing ───────────────────────────────
    if app.save_name_editing {
        if ctrl {
            if key == KEY_A {
                let len = app.save_name_buf.len();
                app.save_name_selection = Some((0, len));
                app.save_name_cursor = len;
            }
            return;
        }
        match key {
            KEY_ENTER => {
                app.save_name_editing = false;
                app.confirm_pick();
                *running = false;
            }
            KEY_ESC => {
                app.save_name_editing = false;
                app.save_name_selection = None;
            }
            KEY_BACKSPACE => {
                if !app.save_name_delete_selection() && app.save_name_cursor > 0 {
                    app.save_name_cursor -= 1;
                    app.save_name_buf.remove(app.save_name_cursor);
                }
                app.save_name_selection = None;
            }
            KEY_DELETE => {
                if !app.save_name_delete_selection() && app.save_name_cursor < app.save_name_buf.len() {
                    app.save_name_buf.remove(app.save_name_cursor);
                }
                app.save_name_selection = None;
            }
            KEY_LEFT => {
                if let Some((a, b)) = app.save_name_selection.take() {
                    app.save_name_cursor = a.min(b);
                } else if app.save_name_cursor > 0 {
                    app.save_name_cursor -= 1;
                }
            }
            KEY_RIGHT => {
                if let Some((a, b)) = app.save_name_selection.take() {
                    app.save_name_cursor = a.max(b).min(app.save_name_buf.len());
                } else if app.save_name_cursor < app.save_name_buf.len() {
                    app.save_name_cursor += 1;
                }
            }
            KEY_HOME => { app.save_name_cursor = 0; app.save_name_selection = None; }
            KEY_END => { app.save_name_cursor = app.save_name_buf.len(); app.save_name_selection = None; }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.save_name_delete_selection();
                    app.save_name_buf.insert(app.save_name_cursor, ch);
                    app.save_name_cursor += ch.len_utf8();
                    app.save_name_selection = None;
                }
            }
        }
        return;
    }

    if ctrl {
        match key {
            KEY_A => app.select_all(),
            KEY_C => app.copy_selected(),
            KEY_X => app.cut_selected(),
            KEY_V => app.paste(),
            KEY_Z => {
                if shift {
                    let _ = app.undo_stack.redo(app.root_mode);
                } else {
                    let _ = app.undo_stack.undo(app.root_mode);
                }
                app.reload();
            }
            KEY_T if app.pick.is_none() => app.new_tab(),
            KEY_W if app.pick.is_none() => app.close_tab(app.current_tab),
            _ => {}
        }
    } else {
        match key {
            KEY_BACKSPACE => app.go_up(),
            KEY_ESC if app.pick.is_some() => {
                app.cancel_pick();
                *running = false;
            }
            KEY_ESC => app.clear_selection(),
            KEY_ENTER if app.pick.is_some() => {
                app.confirm_pick();
                *running = false;
            }
            KEY_F2 if app.pick.is_none() => {
                if let Some(idx) = app.entries.iter().position(|e| e.selected) {
                    app.start_rename(idx);
                }
            }
            KEY_DELETE if app.pick.is_none() => app.trash_selected(),
            _ => {}
        }
    }
}

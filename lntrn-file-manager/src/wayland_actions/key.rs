use lntrn_ui::gpu::{ContextMenu, WaylandPopupBackend};

use crate::app::{floor_boundary, next_boundary, prev_boundary, App};
use crate::keyboard::{code, KeyPress};
use crate::settings::Settings;
use crate::wayland::State;

// Layout-independent key codes (see keyboard.rs): what a `KeyPress` names.
const KEY_ESC: u32 = code::ESC;
const KEY_BACKSPACE: u32 = code::BACKSPACE;
const KEY_TAB: u32 = code::TAB;
const KEY_ENTER: u32 = code::ENTER;
const KEY_A: u32 = 30;
const KEY_C: u32 = 46;
const KEY_V: u32 = 47;
const KEY_X: u32 = 45;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;
const KEY_Z: u32 = 44;
const KEY_F2: u32 = code::F2;
const KEY_DELETE: u32 = code::DELETE;
const KEY_HOME: u32 = code::HOME;
const KEY_END: u32 = code::END;
const KEY_LEFT: u32 = code::LEFT;
const KEY_RIGHT: u32 = code::RIGHT;
const KEY_SPACE: u32 = code::SPACE;

/// A text field has the keyboard: a held key repeats into it.
pub(crate) fn text_entry_active(app: &App) -> bool {
    // A question over the field has the keyboard instead (`handle_key`),
    // and nothing repeats into it: Enter held a moment too long in a Save
    // picker would answer its "Replace?" (with Cancel) before it was read.
    if app.op_dialog_open() && app.quick_look.is_none() {
        return false;
    }
    app.renaming.is_some()
        || app.path_editing
        || app.save_name_editing
        || app.searching
        || app.sudo_prompt.is_some()
        || app.cloud_login.is_some()
        || app
            .properties
            .as_ref()
            .and_then(|p| p.audio.as_ref())
            .is_some_and(|a| a.focused.is_some())
}

/// Byte offset of the `char_idx`-th character of `s` (its length past the
/// end). The search, path, password and login cursors count characters.
fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

pub(crate) fn handle_key(
    app: &mut App,
    _settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    press: KeyPress,
    running: &mut bool,
) {
    // `ch` is what the press types in the user's layout; `key` names it for
    // shortcuts and editing commands.
    let KeyPress {
        key,
        ch: typed,
        ctrl,
        shift,
        ..
    } = press;
    // File-operation dialogs — Esc is the safe answer; Enter too, except
    // that it never confirms a permanent delete. (Quick Look, when open,
    // sits on top and keeps the keys.)
    if app.op_dialog_open() && app.quick_look.is_none() {
        match key {
            KEY_ESC => app.op_dialog_choose(false),
            KEY_ENTER => app.op_dialog_enter(),
            // The arrow and page keys move a long list in the dialog.
            _ => {
                app.cloud_list_key(key);
            }
        }
        return;
    }

    // Conflict dialog — ESC = cancel. Enter does nothing: Replace takes an
    // existing item out of its place and must be a deliberate click.
    // Dispatches to the rename branch when a rename is pending; otherwise
    // the paste branch.
    if app.conflict_dialog.is_some() {
        let _ = ctrl;
        let _ = shift;
        if key == KEY_ESC {
            if app.pending_rename.is_some() {
                app.cancel_rename_conflict();
            } else {
                app.cancel_paste();
            }
        }
        return;
    }

    // Privileged-operation modal — captures keys until it is gone. Only
    // its password phase has a field to type into; on the delete question
    // Enter does nothing (a permanent delete is a deliberate click), and
    // while the commands run even Esc waits.
    if app.sudo_prompt.is_some() {
        if !app.sudo_wants_text() {
            if key == KEY_ESC {
                app.cancel_sudo_prompt();
            }
            return;
        }
        match key {
            KEY_ESC => app.cancel_sudo_prompt(),
            KEY_ENTER => app.submit_sudo_prompt(),
            KEY_BACKSPACE => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    if p.cursor > 0 {
                        let byte_pos = p
                            .password
                            .char_indices()
                            .nth(p.cursor - 1)
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        p.password.remove(byte_pos);
                        p.cursor -= 1;
                    }
                }
            }
            KEY_DELETE => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    let char_len = p.password.chars().count();
                    if p.cursor < char_len {
                        let byte_pos = p
                            .password
                            .char_indices()
                            .nth(p.cursor)
                            .map(|(i, _)| i)
                            .unwrap_or(p.password.len());
                        p.password.remove(byte_pos);
                    }
                }
            }
            KEY_LEFT => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    if p.cursor > 0 {
                        p.cursor -= 1;
                    }
                }
            }
            KEY_RIGHT => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    if p.cursor < p.password.chars().count() {
                        p.cursor += 1;
                    }
                }
            }
            KEY_HOME => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    p.cursor = 0;
                }
            }
            KEY_END => {
                if let Some(p) = app.sudo_prompt.as_mut() {
                    p.cursor = p.password.chars().count();
                }
            }
            // Ctrl+V / Ctrl+A are chords, not letters to type.
            _ if ctrl => {}
            _ => {
                if let Some(ch) = typed {
                    if let Some(p) = app.sudo_prompt.as_mut() {
                        let byte_pos = p
                            .password
                            .char_indices()
                            .nth(p.cursor)
                            .map(|(i, _)| i)
                            .unwrap_or(p.password.len());
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
        match key {
            KEY_ESC => app.cancel_cloud_login(),
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
                        let byte_pos = buf
                            .char_indices()
                            .nth(*cur - 1)
                            .map(|(i, _)| i)
                            .unwrap_or(0);
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
                        let byte_pos = buf
                            .char_indices()
                            .nth(*cur)
                            .map(|(i, _)| i)
                            .unwrap_or(buf.len());
                        buf.remove(byte_pos);
                    }
                }
            }
            KEY_LEFT => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (_buf, cur) = d.focused_buf_mut();
                    if *cur > 0 {
                        *cur -= 1;
                    }
                }
            }
            KEY_RIGHT => {
                if let Some(d) = app.cloud_login.as_mut() {
                    let (buf, cur) = d.focused_buf_mut();
                    if *cur < buf.chars().count() {
                        *cur += 1;
                    }
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
            _ if ctrl => {}
            _ => {
                if let Some(ch) = typed {
                    if let Some(d) = app.cloud_login.as_mut() {
                        let (buf, cur) = d.focused_buf_mut();
                        let byte_pos = buf
                            .char_indices()
                            .nth(*cur)
                            .map(|(i, _)| i)
                            .unwrap_or(buf.len());
                        buf.insert(byte_pos, ch);
                        *cur += 1;
                    }
                }
            }
        }
        return;
    }

    // Properties dialog — swallows everything so shortcuts don't leak to the
    // file view underneath. ESC closes; WAV/MP3 tag fields get text editing.
    if let Some(props) = app.properties.as_mut() {
        if crate::properties_audio::handle_dialog_key(props, key, typed, ctrl, shift) {
            app.properties = None;
        }
        return;
    }

    // Drive dialog — ESC dismisses, ENTER confirms (Format only)
    if app.drive_dialog.is_some() {
        match key {
            KEY_ESC => app.dismiss_drive_dialog(),
            KEY_ENTER => {
                if matches!(
                    app.drive_dialog,
                    Some(crate::dialogs::DriveDialog::ConfirmFormat { .. })
                ) {
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
            } else {
                // Desktop mode draws the menu inline, with no popup backend.
                context_menu.close();
            }
        }
        return;
    }

    // ── Search mode ──────────────────────────────────────────────────
    // A rename (or the path bar / Save name) started while the search bar is
    // open owns the keyboard; those branches follow below.
    let other_text_field = app.renaming.is_some() || app.path_editing || app.save_name_editing;
    if app.searching && !other_text_field {
        match key {
            KEY_ESC => app.close_search(),
            KEY_BACKSPACE => {
                if app.search_cursor > 0 {
                    app.search_cursor -= 1;
                    let at = byte_at(&app.search_buf, app.search_cursor);
                    app.search_buf.remove(at);
                    app.run_search();
                }
            }
            KEY_DELETE => {
                let at = byte_at(&app.search_buf, app.search_cursor);
                if at < app.search_buf.len() {
                    app.search_buf.remove(at);
                    app.run_search();
                }
            }
            KEY_LEFT => {
                if app.search_cursor > 0 {
                    app.search_cursor -= 1;
                }
            }
            KEY_RIGHT => {
                if app.search_cursor < app.search_buf.chars().count() {
                    app.search_cursor += 1;
                }
            }
            KEY_HOME => app.search_cursor = 0,
            KEY_END => app.search_cursor = app.search_buf.chars().count(),
            _ if ctrl => {}
            _ => {
                if let Some(ch) = typed {
                    let at = byte_at(&app.search_buf, app.search_cursor);
                    app.search_buf.insert(at, ch);
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
                    let byte_start = app
                        .path_buf
                        .char_indices()
                        .nth(s)
                        .map(|(i, _)| i)
                        .unwrap_or(app.path_buf.len());
                    let byte_end = app
                        .path_buf
                        .char_indices()
                        .nth(e)
                        .map(|(i, _)| i)
                        .unwrap_or(app.path_buf.len());
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
                    let byte_pos = app
                        .path_buf
                        .char_indices()
                        .nth(app.path_cursor - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    app.path_buf.remove(byte_pos);
                    app.path_cursor -= 1;
                }
                app.path_selection = None;
            }
            KEY_DELETE => {
                if !delete_selection(app) {
                    let char_len = app.path_buf.chars().count();
                    if app.path_cursor < char_len {
                        let byte_pos = app
                            .path_buf
                            .char_indices()
                            .nth(app.path_cursor)
                            .map(|(i, _)| i)
                            .unwrap_or(app.path_buf.len());
                        app.path_buf.remove(byte_pos);
                    }
                }
                app.path_selection = None;
            }
            KEY_LEFT => {
                if app.path_cursor > 0 {
                    app.path_cursor -= 1;
                }
                app.path_selection = None;
            }
            KEY_RIGHT => {
                let char_len = app.path_buf.chars().count();
                if app.path_cursor < char_len {
                    app.path_cursor += 1;
                }
                app.path_selection = None;
            }
            KEY_HOME => {
                app.path_cursor = 0;
                app.path_selection = None;
            }
            KEY_END => {
                app.path_cursor = app.path_buf.chars().count();
                app.path_selection = None;
            }
            _ => {
                if let Some(ch) = typed {
                    delete_selection(app);
                    let byte_pos = app
                        .path_buf
                        .char_indices()
                        .nth(app.path_cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(app.path_buf.len());
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
                    app.rename_cursor = prev_boundary(&app.rename_buf, app.rename_cursor);
                    app.rename_buf.remove(app.rename_cursor);
                }
                app.rename_selection = None;
            }
            KEY_DELETE => {
                if !app.rename_delete_selection() {
                    app.rename_cursor = floor_boundary(&app.rename_buf, app.rename_cursor);
                    if app.rename_cursor < app.rename_buf.len() {
                        app.rename_buf.remove(app.rename_cursor);
                    }
                }
                app.rename_selection = None;
            }
            KEY_LEFT => {
                if let Some((a, b)) = app.rename_selection.take() {
                    app.rename_cursor = a.min(b);
                } else {
                    app.rename_cursor = prev_boundary(&app.rename_buf, app.rename_cursor);
                }
            }
            KEY_RIGHT => {
                if let Some((a, b)) = app.rename_selection.take() {
                    app.rename_cursor = a.max(b).min(app.rename_buf.len());
                } else {
                    app.rename_cursor = next_boundary(&app.rename_buf, app.rename_cursor);
                }
            }
            KEY_HOME => {
                app.rename_cursor = 0;
                app.rename_selection = None;
            }
            KEY_END => {
                app.rename_cursor = app.rename_buf.len();
                app.rename_selection = None;
            }
            _ => {
                if let Some(ch) = typed {
                    app.rename_delete_selection();
                    app.rename_cursor = floor_boundary(&app.rename_buf, app.rename_cursor);
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
                // An empty or invalid name produces no result: keep typing.
                app.confirm_pick();
                if app.pick_result.is_some() {
                    app.save_name_editing = false;
                    *running = false;
                }
            }
            KEY_ESC => {
                app.save_name_editing = false;
                app.save_name_selection = None;
            }
            KEY_BACKSPACE => {
                if !app.save_name_delete_selection() && app.save_name_cursor > 0 {
                    app.save_name_cursor = prev_boundary(&app.save_name_buf, app.save_name_cursor);
                    app.save_name_buf.remove(app.save_name_cursor);
                }
                app.save_name_selection = None;
            }
            KEY_DELETE => {
                if !app.save_name_delete_selection() {
                    app.save_name_cursor = floor_boundary(&app.save_name_buf, app.save_name_cursor);
                    if app.save_name_cursor < app.save_name_buf.len() {
                        app.save_name_buf.remove(app.save_name_cursor);
                    }
                }
                app.save_name_selection = None;
            }
            KEY_LEFT => {
                if let Some((a, b)) = app.save_name_selection.take() {
                    app.save_name_cursor = a.min(b);
                } else {
                    app.save_name_cursor = prev_boundary(&app.save_name_buf, app.save_name_cursor);
                }
            }
            KEY_RIGHT => {
                if let Some((a, b)) = app.save_name_selection.take() {
                    app.save_name_cursor = a.max(b).min(app.save_name_buf.len());
                } else {
                    app.save_name_cursor = next_boundary(&app.save_name_buf, app.save_name_cursor);
                }
            }
            KEY_HOME => {
                app.save_name_cursor = 0;
                app.save_name_selection = None;
            }
            KEY_END => {
                app.save_name_cursor = app.save_name_buf.len();
                app.save_name_selection = None;
            }
            _ => {
                if let Some(ch) = typed {
                    app.save_name_delete_selection();
                    app.save_name_cursor = floor_boundary(&app.save_name_buf, app.save_name_cursor);
                    app.save_name_buf.insert(app.save_name_cursor, ch);
                    app.save_name_cursor += ch.len_utf8();
                    app.save_name_selection = None;
                }
            }
        }
        return;
    }

    // Quick Look overlay — Space/Esc close, ←/→ step through files,
    // everything else is swallowed while it's open.
    if app.quick_look.is_some() {
        match key {
            KEY_ESC | KEY_SPACE => app.quick_look = None,
            KEY_LEFT | KEY_RIGHT => {
                let step: isize = if key == KEY_LEFT { -1 } else { 1 };
                let cur = app
                    .quick_look
                    .as_ref()
                    .and_then(|ql| app.entries.iter().position(|e| e.path == ql.path));
                if let Some(cur) = cur {
                    let mut i = cur as isize + step;
                    while i >= 0 && (i as usize) < app.entries.len() {
                        if !app.entries[i as usize].is_dir {
                            app.select_only(i as usize);
                            app.quick_look = Some(crate::quick_look::QuickLook::open(
                                &app.entries[i as usize],
                            ));
                            break;
                        }
                        i += step;
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // Space opens Quick Look on the selected file (browse mode only —
    // every text-entry mode already returned above).
    if key == KEY_SPACE && !ctrl {
        if let Some(entry) = app.entries.iter().find(|e| e.selected && !e.is_dir) {
            app.quick_look = Some(crate::quick_look::QuickLook::open(entry));
        }
        return;
    }

    if ctrl {
        match key {
            KEY_A => app.select_all(),
            KEY_C => app.copy_selected(),
            KEY_X => app.cut_selected(),
            KEY_V => app.paste(),
            // Only asks: the work runs on the ops worker, and its result
            // (and the reload) arrive when it ends.
            KEY_Z => app.request_history(if shift {
                crate::undo::Direction::Redo
            } else {
                crate::undo::Direction::Undo
            }),
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
                // Nothing eligible selected: stay open instead of exiting as
                // "cancelled".
                app.confirm_pick();
                if app.pick_result.is_some() {
                    *running = false;
                }
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

#[cfg(test)]
mod tests {
    use super::byte_at;

    #[test]
    fn a_character_cursor_finds_its_byte_in_text_that_is_not_ascii() {
        let s = "a\u{e9}\u{1F44D}z";
        assert_eq!(byte_at(s, 0), 0);
        assert_eq!(byte_at(s, 1), 1);
        assert_eq!(byte_at(s, 2), 3);
        assert_eq!(byte_at(s, 3), 7);
        // At and past the end: the place to append.
        assert_eq!(byte_at(s, 4), s.len());
        assert_eq!(byte_at(s, 9), s.len());
        // Every answer is a place a character can be inserted or removed.
        assert!((0..6).all(|i| s.is_char_boundary(byte_at(s, i))));
    }
}

//! Per-frame input dispatch — pulls scroll-wheel deltas and pending
//! keys out of [`WlState`] and routes them into whichever view is
//! currently active.

use super::WlState;
use crate::app::{AppState, PanelRect};

/// Drain the accumulated vertical scroll delta into whichever view is
/// currently scrolling. Each view computes its own viewport rect so the
/// max offset matches what the renderer actually shows.
pub(super) fn handle_scroll(wl: &mut WlState, app: &mut AppState) {
    if wl.scroll_delta_v.abs() <= 0.0 {
        return;
    }
    let dy = wl.scroll_delta_v as f32;
    wl.scroll_delta_v = 0.0;

    let scale_f = wl.fractional_scale() as f32;
    let phys_w = wl.phys_width().max(1);
    let panel = PanelRect::compute_with_dims(
        phys_w,
        scale_f,
        app.desired_panel_w_logical(),
        app.desired_panel_h_logical(),
    );
    let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);

    if app.emojis.open {
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let panel_bottom = panel_rect.y + panel_rect.h;
        let grid = crate::emojis::grid_rect(panel_rect, top_y, scale_f, panel_bottom);
        let visible = app.emojis.visible_indices();
        let per_row = crate::emojis::cells_per_row(grid, scale_f);
        let max = crate::emojis::max_scroll(visible.len(), per_row, grid.h, scale_f) / scale_f;
        app.emojis.scroll = (app.emojis.scroll + dy).clamp(0.0, max);
    } else if matches!(app.mode, crate::app::PanelMode::Control(crate::controls::TileId::Wifi)) {
        let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let list_top = crate::controls::wifi::row_list_top_y(view_top_y, scale_f);
        let viewport_h = (panel_rect.y + panel_rect.h - list_top).max(0.0);
        let max = crate::controls::wifi::max_scroll(&app.controls.wifi, viewport_h, scale_f);
        app.controls.wifi.scroll = (app.controls.wifi.scroll + dy).clamp(0.0, max);
    } else if app.clipboard.open {
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let panel_bottom = panel_rect.y + panel_rect.h;
        let list = crate::clipboard::list_rect(panel_rect, top_y, scale_f, panel_bottom);
        let max = crate::clipboard::max_scroll(&app.clipboard, list.h, scale_f) / scale_f;
        app.clipboard.scroll = (app.clipboard.scroll + dy).clamp(0.0, max);
    } else if app.notes.open {
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let panel_bottom = panel_rect.y + panel_rect.h;
        let editor = crate::notes::editor_rect(panel_rect, top_y, scale_f, panel_bottom);
        let body = crate::notes::body_rect(editor, scale_f);
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        let over_body = phys_cx >= body.x && phys_cx <= body.x + body.w
            && phys_cy >= body.y && phys_cy <= body.y + body.h;
        if over_body {
            let line_count = app.notes.body.raw().split('\n').count();
            let max = crate::notes::body_max_scroll(
                line_count, body, scale_f, app.config.text_size,
            );
            // dy is in logical pixels here; body_scroll is too.
            app.notes.body_scroll = (app.notes.body_scroll + dy).clamp(0.0, max);
        } else {
            let list = crate::notes::list_rect(panel_rect, top_y, scale_f, panel_bottom);
            let visible = app.notes.visible_indices();
            let max = crate::notes::list_max_scroll(visible.len(), list.h, scale_f) / scale_f;
            app.notes.list_scroll = (app.notes.list_scroll + dy).clamp(0.0, max);
        }
    } else if app.panel_view == crate::app::PanelView::Terminal {
        // libinput reports negative dy on wheel-up (toward older
        // history) and positive on wheel-down. Convert to lines via a
        // soft divisor so one notch ≈ 3 lines.
        let lines = (-dy / 16.0).round() as i32;
        if lines != 0 {
            app.terminal.scroll_by(lines);
        }
    } else if app.panel_view == crate::app::PanelView::Files {
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let max = crate::files::max_scroll(
            &app.files, panel_rect, top_y, scale_f, app.config.text_size,
        );
        app.files.scroll = (app.files.scroll + dy * scale_f).clamp(0.0, max);
    } else if matches!(app.mode, crate::app::PanelMode::Launcher)
        && (!app.search.input.is_empty() || app.search.all_apps_mode)
        && !app.search.results().is_empty()
    {
        let max = crate::search::max_scroll(&app.search, panel_rect, scale_f);
        // libinput hi-res wheel reports ~15 per notch, low-res ~10.
        // Scaling by scale_f keeps "one notch ≈ one row" feel.
        app.search.scroll_offset = (app.search.scroll_offset + dy * scale_f).clamp(0.0, max);
    }
}

/// Apply key auto-repeat: once a key has been held past `REPEAT_DELAY`,
/// synthesize fresh `pending_key` events at `REPEAT_INTERVAL` so
/// backspace, arrows, etc. behave naturally.
pub(super) fn apply_key_autorepeat(wl: &mut WlState) {
    if wl.pending_key.is_some() {
        return;
    }
    let Some((held, pressed_at)) = wl.held_key else { return };
    const REPEAT_DELAY: std::time::Duration = std::time::Duration::from_millis(420);
    const REPEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(32);
    let now = std::time::Instant::now();
    if now.duration_since(pressed_at) < REPEAT_DELAY {
        return;
    }
    let ready = match wl.last_repeat {
        None => true,
        Some(last) => now.duration_since(last) >= REPEAT_INTERVAL,
    };
    if ready {
        wl.pending_key = Some(held);
        wl.last_repeat = Some(now);
    }
}


/// Dispatch the next pending keypress from `wl.pending_key`. Routing
/// priority (most-specific first):
///   1. WiFi password modal — typed chars into the SSID buffer; Enter submits.
///   2. BT pair-prompt modal — Enter confirms, typing fills the passkey
///      buffer when the prompt kind is `Enter`.
///   3. Notes overlay — supports text editing + Ctrl-A/V/C/X chord keys.
///   4. Clipboard overlay — filter input + Enter copies first hit.
///   5. Emojis overlay — filter input + Enter copies first hit.
///   6. Terminal view — Ctrl-Shift-{C,V} copy/paste, else raw PTY input.
///   7. SysMon / Temp view — typed chars edit the process filter.
///   8. Calendar add-event input — typed chars edit the title.
///   9. Files view — `Ctrl+H` toggles hidden, typing activates filter.
///   10. Default — launcher selection (Up/Down/Left/Right/Enter) + search.
pub(super) fn handle_keypress(wl: &mut WlState, app: &mut AppState) {
    let Some(key) = wl.pending_key.take() else { return };
    use crate::controls::bluetooth::PairPromptKind;
    use crate::search::input::*;

    // Global Ctrl+arrow chord shortcuts — intercept before any per-view
    // routing so textboxes (terminal, chat, search) don't hijack them.
    //
    //   Ctrl+Up/Down         expand / collapse the panel
    //   Ctrl+Left/Right      cycle panel tabs
    //   Ctrl+Shift+Up/Down   grow / shrink the *expanded* panel  (expanded only)
    //   Ctrl+Shift+Left/Right grow / shrink the *collapsed bar*  (collapsed only)
    if wl.ctrl_held && matches!(key, KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT) {
        match (wl.shift_held, key) {
            (false, KEY_UP) => app.set_collapsed(true),
            (false, KEY_DOWN) => app.set_collapsed(false),
            (false, KEY_LEFT) => app.cycle_view_prev(),
            (false, KEY_RIGHT) => app.cycle_view_next(),
            (true, KEY_DOWN) if !app.collapsed => app.inc_panel_size(),
            (true, KEY_UP) if !app.collapsed => app.dec_panel_size(),
            (true, KEY_RIGHT) if app.collapsed => app.inc_bar_size(),
            (true, KEY_LEFT) if app.collapsed => app.dec_bar_size(),
            _ => {}
        }
        return;
    }

    if app.controls.wifi.prompt.is_some() {
        match key {
            KEY_ENTER | KEY_KP_ENTER => app.controls.wifi.submit_prompt(),
            other => {
                if let Some(prompt) = app.controls.wifi.prompt.as_mut() {
                    let _ = prompt.input.on_key(other, wl.shift_held, wl.caps_lock);
                }
            }
        }
    } else if let Some(kind) = app
        .controls
        .bluetooth
        .pair_prompt
        .as_ref()
        .map(|p| p.kind.clone())
    {
        match (key, kind) {
            (KEY_ENTER | KEY_KP_ENTER, PairPromptKind::Confirm(_))
            | (KEY_ENTER | KEY_KP_ENTER, PairPromptKind::Authorize(_)) => {
                app.controls.bluetooth.pair_confirm_yes();
            }
            (KEY_ENTER | KEY_KP_ENTER, PairPromptKind::Enter) => {
                app.controls.bluetooth.pair_submit_passkey();
            }
            (other, PairPromptKind::Enter) => {
                if let Some(prompt) = app.controls.bluetooth.pair_prompt.as_mut() {
                    let _ = prompt.passkey_input.on_key(other, wl.shift_held, wl.caps_lock);
                }
            }
            _ => {
                // Confirm/Authorize: only Enter is accepted as a key
                // shortcut. Y/N typing isn't wired (yet).
            }
        }
    } else if app.notes.open {
        if app.notes.confirm_delete {
            if matches!(key, KEY_ENTER | KEY_KP_ENTER) {
                app.notes.delete_selected();
                app.notes.confirm_delete = false;
            }
        } else {
            // Ctrl chords intercept before the editor sees the raw key
            // (otherwise "Ctrl+V" inserts a literal 'v').
            use crate::search::input::keycode_to_char;
            let chord_ch = if wl.ctrl_held {
                keycode_to_char(key, false, false).map(|c| c.to_ascii_lowercase())
            } else {
                None
            };
            match chord_ch {
                Some('a') => match app.notes.focus {
                    crate::notes::NoteFocus::Filter => app.notes.filter.select_all(),
                    crate::notes::NoteFocus::Title => app.notes.title.select_all(),
                    crate::notes::NoteFocus::Body => app.notes.body.select_all(),
                },
                Some('v') => {
                    if let Some(clip) = app.clipboard_handle.as_ref() {
                        if let Some(text) = clip.get_text() {
                            match app.notes.focus {
                                crate::notes::NoteFocus::Filter => {
                                    app.notes.filter.insert_str(&text);
                                    app.notes.list_scroll = 0.0;
                                }
                                crate::notes::NoteFocus::Title => {
                                    app.notes.title.insert_str(&text);
                                    app.notes.flush_edits_to_selected();
                                }
                                crate::notes::NoteFocus::Body => {
                                    app.notes.body.insert_str(&text);
                                    app.notes.flush_edits_to_selected();
                                }
                            }
                        }
                    }
                }
                Some('c') | Some('x') => {
                    // Copy / Cut: prefer selection if any, else the entire field.
                    let (selected, full) = match app.notes.focus {
                        crate::notes::NoteFocus::Filter => (
                            app.notes.filter.selected_text().map(|s| s.to_string()),
                            app.notes.filter.query().to_string(),
                        ),
                        crate::notes::NoteFocus::Title => (
                            app.notes.title.selected_text().map(|s| s.to_string()),
                            app.notes.title.query().to_string(),
                        ),
                        crate::notes::NoteFocus::Body => (
                            app.notes.body.selected_text().map(|s| s.to_string()),
                            app.notes.body.text(),
                        ),
                    };
                    let payload = selected.clone().unwrap_or(full);
                    if !payload.is_empty() {
                        if let Some(clip) = app.clipboard_handle.as_ref() {
                            clip.set_text(&payload);
                        }
                    }
                    // Cut removes the selection (no-op if none).
                    if chord_ch == Some('x') && selected.is_some() {
                        match app.notes.focus {
                            crate::notes::NoteFocus::Filter => {
                                app.notes.filter.insert_str("");
                                app.notes.list_scroll = 0.0;
                            }
                            crate::notes::NoteFocus::Title => {
                                app.notes.title.insert_str("");
                                app.notes.flush_edits_to_selected();
                            }
                            crate::notes::NoteFocus::Body => {
                                app.notes.body.insert_str("");
                                app.notes.flush_edits_to_selected();
                            }
                        }
                    }
                }
                Some(_) => {
                    // Other ctrl chords: ignore for now.
                }
                None => {
                    // Normal key dispatch.
                    match app.notes.focus {
                        crate::notes::NoteFocus::Filter => {
                            let _ = app.notes.filter.on_key(key, wl.shift_held, wl.caps_lock);
                            app.notes.list_scroll = 0.0;
                        }
                        crate::notes::NoteFocus::Title => {
                            let _ = app.notes.title.on_key(key, wl.shift_held, wl.caps_lock);
                            app.notes.flush_edits_to_selected();
                        }
                        crate::notes::NoteFocus::Body => {
                            let _ = app.notes.body.on_key(key, wl.shift_held, wl.caps_lock);
                            app.notes.flush_edits_to_selected();
                        }
                    }
                }
            }
        }
        // Auto-scroll the body to keep the caret on-screen after any
        // edit / cursor move.
        if app.notes.focus == crate::notes::NoteFocus::Body {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let panel_bottom = panel_rect.y + panel_rect.h;
            let editor = crate::notes::editor_rect(panel_rect, top_y, scale_f, panel_bottom);
            let body = crate::notes::body_rect(editor, scale_f);
            app.notes.body_scroll = crate::notes::body_scroll_for_caret(
                &app.notes, body, scale_f, app.config.text_size,
            );
        }
    } else if app.clipboard.open {
        if app.clipboard.confirm_clear {
            if matches!(key, KEY_ENTER | KEY_KP_ENTER) {
                crate::clipboard::ipc::clear();
                app.clipboard.confirm_clear = false;
                app.refresh_clipboard();
            }
        } else {
            match key {
                KEY_ENTER | KEY_KP_ENTER => {
                    let visible = app.clipboard.visible_indices();
                    if let Some(&eidx) = visible.first() {
                        if let Some(entry) = app.clipboard.entries.get(eidx) {
                            let id = entry.id;
                            if crate::clipboard::ipc::copy(id) {
                                app.clipboard.recent_copy =
                                    Some((id, std::time::Instant::now()));
                            }
                        }
                    }
                }
                other => {
                    let _ = app.clipboard.filter.on_key(other, wl.shift_held, wl.caps_lock);
                    app.clipboard.reset_scroll();
                }
            }
        }
    } else if app.emojis.open {
        // Emojis overlay: filter input always focused. Enter copies the
        // first visible emoji. Esc handled elsewhere.
        match key {
            KEY_ENTER | KEY_KP_ENTER => {
                let visible = app.emojis.visible_indices();
                if let Some(&entry_idx) = visible.first() {
                    let entry = &crate::emojis::data::EMOJIS[entry_idx];
                    if let Some(clip) = app.clipboard_handle.as_ref() {
                        clip.set_text(entry.glyph);
                    }
                    app.emojis.recent_copy = Some((0, std::time::Instant::now()));
                }
            }
            other => {
                let _ = app.emojis.filter.on_key(other, wl.shift_held, wl.caps_lock);
                app.emojis.reset_scroll();
            }
        }
    } else if app.panel_view == crate::app::PanelView::Terminal {
        // Copy / Paste chord keys take priority over normal PTY input.
        // Ctrl-Shift-C copies the current selection; Ctrl-Shift-V
        // pastes clipboard into the PTY (xterm convention).
        use crate::search::input::keycode_to_char;
        let chord = wl.ctrl_held && wl.shift_held;
        let ch = keycode_to_char(key, false, false).map(|c| c.to_ascii_lowercase());
        if chord && ch == Some('c') {
            if app.terminal.copy_selection() {
                tracing::info!("terminal: copied selection");
            }
        } else if chord && ch == Some('v') {
            if app.terminal.paste_from_clipboard() {
                tracing::info!("terminal: pasted from clipboard");
            }
        } else if let Some(bytes) =
            crate::terminal::keycode_to_bytes(key, wl.shift_held, wl.ctrl_held, wl.caps_lock)
        {
            app.terminal.write(&bytes);
        }
    } else if matches!(
        app.mode,
        crate::app::PanelMode::Control(crate::controls::TileId::SysMon)
            | crate::app::PanelMode::Control(crate::controls::TileId::Temp)
    ) {
        // Sysmon process filter: typed chars edit the buffer. Esc
        // (handled above via esc_pressed) clears it; Enter and arrows
        // are inert here.
        if !matches!(key, KEY_ENTER | KEY_KP_ENTER | KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT) {
            let _ = app.controls.sysmon.filter.on_key(key, wl.shift_held, wl.caps_lock);
        }
    } else if app.controls.clock.add_event_input.is_some()
        && matches!(app.mode, crate::app::PanelMode::Control(crate::controls::TileId::Clock))
    {
        // Calendar add-event input has focus.
        match key {
            KEY_ENTER | KEY_KP_ENTER => {
                if let (Some(date), Some(input)) = (
                    app.controls.clock.selected_day,
                    app.controls.clock.add_event_input.as_ref(),
                ) {
                    let title = input.query().to_string();
                    if !title.trim().is_empty() {
                        app.controls.events.add(date, title);
                    }
                }
                app.controls.clock.add_event_input = None;
            }
            other => {
                if let Some(input) = app.controls.clock.add_event_input.as_mut() {
                    let _ = input.on_key(other, wl.shift_held, wl.caps_lock);
                }
            }
        }
    } else if app.panel_view == crate::app::PanelView::Files {
        // Files view: route to the filter input when active, else
        // typing auto-activates filter mode. Ctrl+H toggles hidden
        // files; Esc exits filter mode.
        use crate::search::input::keycode_to_char;
        let chord_ctrl_h = wl.ctrl_held
            && keycode_to_char(key, false, false).map(|c| c.to_ascii_lowercase()) == Some('h');
        if chord_ctrl_h {
            app.files.toggle_hidden();
        } else {
            match key {
                KEY_LEFT if !app.files.filter_active => app.cycle_view_prev(),
                KEY_RIGHT if !app.files.filter_active => app.cycle_view_next(),
                KEY_ENTER | KEY_KP_ENTER if app.files.filter_active => {
                    if let Some(entry) = app.files.entry_for_visible(0).cloned() {
                        if entry.is_dir {
                            app.files.navigate_to(&entry.path);
                        } else {
                            let exec = format!(
                                "xdg-open '{}'",
                                entry.path.to_string_lossy().replace('\'', "'\\''"),
                            );
                            crate::app::spawn_detached(&exec);
                            app.close();
                        }
                    }
                }
                other => {
                    // If a printable character was pressed and we're
                    // not in filter mode yet, auto-activate so the
                    // first letter shows up in the input.
                    let printable = keycode_to_char(other, wl.shift_held, wl.caps_lock).is_some();
                    if printable && !app.files.filter_active {
                        app.files.activate_filter();
                    }
                    if app.files.filter_active {
                        let _ = app.files.filter.on_key(other, wl.shift_held, wl.caps_lock);
                        app.files.apply_filter();
                    }
                }
            }
        }
    } else {
        match key {
            KEY_UP => app.select_up(),
            KEY_DOWN => app.select_down(),
            // Left / Right cycle the panel-view tabs. Only fires when
            // the search field is empty, so typing letters into the
            // launcher still moves the text cursor instead of switching
            // tabs.
            KEY_LEFT if app.search.input.is_empty() => app.cycle_view_prev(),
            KEY_RIGHT if app.search.input.is_empty() => app.cycle_view_next(),
            KEY_ENTER | KEY_KP_ENTER => {
                app.launch_selected();
            }
            _ => {
                let _ = app.forward_key(key, wl.shift_held, wl.caps_lock);
            }
        }
    }
}

//! Keyboard input dispatcher. Routes key events to whichever subsystem
//! currently owns input — for now that's always the editor; once the find
//! bar lands it will get first dibs when active.

use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::actions;
use crate::auto_pair;
use crate::lsp;
use crate::term_panel::TermPanel;
use crate::TextHandler;

/// Result of handling a key. Lets `main.rs` decide whether to redraw or quit.
pub enum KeyAction {
    /// Nothing happened (key was unrecognized or consumed without state
    /// change).
    Ignored,
    /// Something changed — request a redraw and reset cursor blink.
    Consumed,
}

/// Handle a single pressed key event.
pub fn handle_key(
    handler: &mut TextHandler,
    key: &Key,
    mods: ModifiersState,
) -> KeyAction {
    // Escape closes any floating LSP UI first, then the menu/find bar.
    if matches!(key, Key::Named(NamedKey::Escape)) {
        if handler.completion.visible {
            handler.completion.clear();
            return KeyAction::Consumed;
        }
        if handler.hover.visible {
            handler.hover.clear();
            return KeyAction::Consumed;
        }
        if handler.menu_bar.is_open() {
            handler.menu_bar.close();
            return KeyAction::Consumed;
        }
        if handler.find_bar.is_visible() {
            handler.find_bar.close();
            return KeyAction::Consumed;
        }
        return KeyAction::Ignored;
    }

    // ── Completion popup nav (must run before normal text input) ─────
    if handler.completion.visible {
        match key {
            Key::Named(NamedKey::ArrowDown) => {
                let n = handler.completion.filtered().len();
                if n > 0 {
                    handler.completion.selected =
                        (handler.completion.selected + 1).min(n - 1);
                }
                return KeyAction::Consumed;
            }
            Key::Named(NamedKey::ArrowUp) => {
                handler.completion.selected =
                    handler.completion.selected.saturating_sub(1);
                return KeyAction::Consumed;
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Tab) => {
                let choice = handler
                    .completion
                    .filtered()
                    .get(handler.completion.selected)
                    .map(|i| (*i).clone());
                if let Some(item) = choice {
                    lsp::glue::accept_completion(handler, &item);
                    return KeyAction::Consumed;
                }
                handler.completion.clear();
            }
            _ => {}
        }
    }

    let ctrl = mods.contains(ModifiersState::CONTROL);
    let shift = mods.contains(ModifiersState::SHIFT);
    let alt = mods.contains(ModifiersState::ALT);

    // ── Ctrl+` toggles terminal panel ────────────────────────────────
    if ctrl && !shift && !alt {
        if let Key::Character(s) = key {
            if s.as_str() == "`" {
                if let Some(panel) = &mut handler.term_panel {
                    if panel.visible {
                        // Toggle focus, or hide if already focused
                        if panel.focused {
                            panel.visible = false;
                            panel.focused = false;
                        } else {
                            panel.focused = true;
                        }
                    } else {
                        panel.visible = true;
                        panel.focused = true;
                    }
                } else {
                    // First time — spawn the terminal
                    match TermPanel::new(handler.proxy.clone()) {
                        Ok(panel) => handler.term_panel = Some(panel),
                        Err(e) => eprintln!("[lntrn-code] terminal spawn failed: {e}"),
                    }
                }
                return KeyAction::Consumed;
            }
        }
    }

    // ── Route to terminal when it has focus ───────────────────────────
    if let Some(panel) = &mut handler.term_panel {
        if panel.visible && panel.focused {
            if panel.handle_key(key, mods) {
                return KeyAction::Consumed;
            }
        }
    }

    // ── LSP shortcuts ────────────────────────────────────────────────
    // F12 → goto-definition on the current caret.
    if matches!(key, Key::Named(NamedKey::F12)) {
        lsp::glue::request_definition(handler, false);
        return KeyAction::Consumed;
    }
    // Ctrl+Space → completion at caret.
    if ctrl && !shift && !alt {
        if matches!(key, Key::Named(NamedKey::Space)) {
            let (ax, ay) = caret_anchor(handler);
            lsp::glue::request_completion(handler, ax, ay);
            return KeyAction::Consumed;
        }
    }

    // ── Alt shortcuts ────────────────────────────────────────────────
    if alt && !ctrl {
        if let Key::Character(s) = key {
            match s.as_str() {
                "z" => {
                    let ed = handler.editor_mut();
                    ed.wrap_enabled = !ed.wrap_enabled;
                    return KeyAction::Consumed;
                }
                "m" => {
                    handler.minimap_visible = !handler.minimap_visible;
                    return KeyAction::Consumed;
                }
                _ => {}
            }
        }
    }

    // ── Find / Replace shortcuts ──────────────────────────────────────
    if ctrl && !shift {
        if let Key::Character(s) = key {
            match s.as_str() {
                "f" => {
                    let prefill = handler.editor().selected_text();
                    handler.find_bar.open_find(prefill);
                    let active = handler.active_tab;
                    let find_bar = &mut handler.find_bar;
                    let editor = &mut handler.tabs[active];
                    find_bar.recompute(editor);
                    find_bar.focus_current(editor);
                    return KeyAction::Consumed;
                }
                "h" => {
                    handler.find_bar.open_replace();
                    let active = handler.active_tab;
                    let find_bar = &mut handler.find_bar;
                    let editor = &mut handler.tabs[active];
                    find_bar.recompute(editor);
                    return KeyAction::Consumed;
                }
                _ => {}
            }
        }
    }

    // If find bar is visible, route input there first.
    if handler.find_bar.is_visible() {
        let active = handler.active_tab;
        let find_bar = &mut handler.find_bar;
        let editor = &mut handler.tabs[active];
        if find_bar.handle_key(key, mods, editor) {
            return KeyAction::Consumed;
        }
    }

    // ── Tab navigation (Ctrl+Tab / Ctrl+Shift+Tab / Ctrl+1..9) ────────
    if ctrl {
        if matches!(key, Key::Named(NamedKey::Tab)) {
            handler.cycle_tab(!shift);
            return KeyAction::Consumed;
        }
        if let Key::Character(s) = key {
            if let Some(ch) = s.chars().next() {
                if ch.is_ascii_digit() && ch != '0' {
                    let idx = ch.to_digit(10).unwrap() as usize - 1;
                    handler.switch_tab(idx);
                    return KeyAction::Consumed;
                }
            }
        }
    }

    // Preview tabs are read-only — block any key that would mutate the
    // document. Navigation keys (arrows, Home/End, PgUp/PgDn) still work
    // because they move the cursor without changing text.
    let preview_block = handler.editor().is_preview()
        && match key {
            Key::Named(NamedKey::Enter)
            | Key::Named(NamedKey::Backspace)
            | Key::Named(NamedKey::Delete)
            | Key::Named(NamedKey::Tab) => true,
            Key::Named(NamedKey::Space) => true,
            Key::Character(_) if !ctrl => true,
            Key::Character(s) if ctrl => matches!(s.as_str(), "v" | "x" | "z"),
            _ => false,
        };
    if preview_block {
        return KeyAction::Ignored;
    }

    match key {
        Key::Character(s) if ctrl && shift => match s.as_str() {
            "Z" | "z" => {
                handler.editor_mut().redo();
                KeyAction::Consumed
            }
            "B" | "b" => {
                handler.sidebar.toggle_visible();
                KeyAction::Consumed
            }
            _ => KeyAction::Ignored,
        },
        Key::Character(s) if ctrl => match s.as_str() {
            "s" => {
                actions::save_file_dialog(handler);
                KeyAction::Consumed
            }
            "o" => {
                actions::open_file_dialog(handler);
                KeyAction::Consumed
            }
            "p" => {
                handler.open_preview();
                KeyAction::Consumed
            }
            "n" | "t" => {
                handler.new_tab();
                KeyAction::Consumed
            }
            "w" => {
                let idx = handler.active_tab;
                handler.close_tab(idx);
                KeyAction::Consumed
            }
            "a" => {
                handler.editor_mut().select_all();
                KeyAction::Consumed
            }
            "c" => {
                actions::do_copy(handler);
                KeyAction::Consumed
            }
            "x" => {
                actions::do_cut(handler);
                KeyAction::Consumed
            }
            "v" => {
                actions::do_paste(handler);
                KeyAction::Consumed
            }
            "z" => {
                handler.editor_mut().undo();
                KeyAction::Consumed
            }
            _ => KeyAction::Ignored,
        },
        Key::Named(NamedKey::Enter) => {
            handler.editor_mut().insert_char('\n');
            KeyAction::Consumed
        }
        Key::Named(NamedKey::Backspace) => {
            let editor = handler.editor_mut();
            if !auto_pair::handle_backspace(editor) {
                editor.backspace();
            }
            KeyAction::Consumed
        }
        Key::Named(NamedKey::Delete) => {
            handler.editor_mut().delete();
            KeyAction::Consumed
        }
        Key::Named(NamedKey::ArrowLeft) => {
            handler.editor_mut().move_left(shift);
            KeyAction::Consumed
        }
        Key::Named(NamedKey::ArrowRight) => {
            handler.editor_mut().move_right(shift);
            KeyAction::Consumed
        }
        Key::Named(NamedKey::ArrowUp) => {
            handler.editor_mut().move_up(shift);
            KeyAction::Consumed
        }
        Key::Named(NamedKey::ArrowDown) => {
            handler.editor_mut().move_down(shift);
            KeyAction::Consumed
        }
        Key::Named(NamedKey::Home) => {
            handler.editor_mut().home(shift);
            KeyAction::Consumed
        }
        Key::Named(NamedKey::End) => {
            handler.editor_mut().end(shift);
            KeyAction::Consumed
        }
        Key::Named(NamedKey::Space) => {
            handler.editor_mut().insert_char(' ');
            KeyAction::Consumed
        }
        Key::Named(NamedKey::Tab) => {
            handler.editor_mut().insert_str("    ");
            KeyAction::Consumed
        }
        Key::Character(s) if !ctrl => {
            let editor = handler.editor_mut();
            for ch in s.chars() {
                if !auto_pair::handle_typed_char(ch, editor) {
                    editor.insert_char(ch);
                }
            }
            KeyAction::Consumed
        }
        _ => KeyAction::Ignored,
    }
}

/// Compute a reasonable anchor point for popups (hover / completion) at the
/// caret's current screen position. Returns `(x, y_below_caret)`.
fn caret_anchor(handler: &TextHandler) -> (f32, f32) {
    let (cx, cy) = handler
        .input
        .cursor()
        .unwrap_or((80.0 * handler.scale, 80.0 * handler.scale));
    // Anchor is "just below the cursor caret". The caret draws on a blink
    // so the exact on-screen position is non-trivial to query from here —
    // falling back to the mouse position is close enough: users typically
    // trigger completion right where they're looking.
    (cx, cy + handler.scale * 20.0)
}

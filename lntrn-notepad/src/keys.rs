//! Keyboard input dispatcher. Routes key events to the editor and any
//! overlay subsystems (find bar) that currently own input.

use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::actions;
use crate::TextHandler;

/// Result of handling a key. Lets `main.rs` decide whether to redraw or quit.
pub enum KeyAction {
    /// Nothing happened (key was unrecognized or consumed without state change).
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
    if matches!(key, Key::Named(NamedKey::Escape)) {
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

    let ctrl = mods.contains(ModifiersState::CONTROL);
    let shift = mods.contains(ModifiersState::SHIFT);

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

    match key {
        Key::Character(s) if ctrl && shift => match s.as_str() {
            "Z" | "z" => {
                handler.editor_mut().redo();
                KeyAction::Consumed
            }
            "X" | "x" => {
                handler
                    .editor_mut()
                    .toggle_format(|a| a.strikethrough = !a.strikethrough);
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
            "b" => {
                handler.editor_mut().toggle_format(|a| a.bold = !a.bold);
                KeyAction::Consumed
            }
            "i" => {
                handler.editor_mut().toggle_format(|a| a.italic = !a.italic);
                KeyAction::Consumed
            }
            "u" => {
                handler
                    .editor_mut()
                    .toggle_format(|a| a.underline = !a.underline);
                KeyAction::Consumed
            }
            _ => KeyAction::Ignored,
        },
        Key::Named(NamedKey::Enter) => {
            handler.editor_mut().insert_char('\n');
            KeyAction::Consumed
        }
        Key::Named(NamedKey::Backspace) => {
            handler.editor_mut().backspace();
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
                editor.insert_char(ch);
            }
            KeyAction::Consumed
        }
        _ => KeyAction::Ignored,
    }
}

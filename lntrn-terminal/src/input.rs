use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::clipboard::WaylandClipboard;
use crate::pty::Pty;
use crate::terminal::TerminalState;

/// Process a winit keyboard event and write appropriate bytes to PTY.
/// Returns true if the event was handled.
pub fn handle_key(
    key: &Key,
    state: ElementState,
    modifiers: ModifiersState,
    terminal: &mut TerminalState,
    pty: &Pty,
    clipboard: &Option<WaylandClipboard>,
) -> bool {
    if state != ElementState::Pressed {
        return false;
    }

    let ctrl = modifiers.contains(ModifiersState::CONTROL);
    let shift = modifiers.contains(ModifiersState::SHIFT);

    // Ctrl+Shift shortcuts (terminal chrome)
    if ctrl && shift {
        match key {
            Key::Character(s) => match s.as_str() {
                "C" | "c" => {
                    do_copy(terminal, clipboard);
                    return true;
                }
                "V" | "v" => {
                    do_paste(clipboard, pty);
                    terminal.scroll_offset = 0;
                    return true;
                }
                "N" | "n" => {
                    do_new_window();
                    return true;
                }
                _ => {}
            },
            _ => {}
        }
    }

    // Ctrl+key → control characters
    if ctrl && !shift {
        if let Key::Character(s) = key {
            if let Some(c) = s.chars().next() {
                let ctrl_char = match c.to_ascii_lowercase() {
                    'a'..='z' => Some(c.to_ascii_lowercase() as u8 - b'a' + 1),
                    _ => None,
                };
                if let Some(byte) = ctrl_char {
                    terminal.scroll_offset = 0;
                    pty.write(&[byte]);
                    return true;
                }
            }
        }
    }

    // Shift+PageUp/PageDown for scrollback
    if shift {
        match key {
            Key::Named(NamedKey::PageUp) => {
                let page = terminal.rows.max(1);
                let max_offset = terminal.scrollback.len();
                terminal.scroll_offset = (terminal.scroll_offset + page).min(max_offset);
                return true;
            }
            Key::Named(NamedKey::PageDown) => {
                let page = terminal.rows.max(1);
                terminal.scroll_offset = terminal.scroll_offset.saturating_sub(page);
                return true;
            }
            _ => {}
        }
    }

    // Named keys → escape sequences
    if let Key::Named(named) = key {
        let seq = named_key_to_seq(named, terminal.application_cursor);
        if !seq.is_empty() {
            terminal.scroll_offset = 0;
            terminal.clear_selection();
            pty.write(&seq);
            return true;
        }
    }

    // Regular text input
    if let Key::Character(s) = key {
        if !ctrl {
            terminal.scroll_offset = 0;
            terminal.clear_selection();
            pty.write(s.as_bytes());
            return true;
        }
    }

    false
}

fn named_key_to_seq(key: &NamedKey, app_cursor: bool) -> Vec<u8> {
    match key {
        NamedKey::Space => vec![0x20],
        NamedKey::Enter => vec![0x0D],
        NamedKey::Tab => vec![0x09],
        NamedKey::Backspace => vec![0x7F],
        NamedKey::Escape => vec![0x1B],
        NamedKey::ArrowUp => {
            if app_cursor {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        NamedKey::ArrowDown => {
            if app_cursor {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        NamedKey::ArrowRight => {
            if app_cursor {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        NamedKey::ArrowLeft => {
            if app_cursor {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        NamedKey::Home => b"\x1b[H".to_vec(),
        NamedKey::End => b"\x1b[F".to_vec(),
        NamedKey::PageUp => b"\x1b[5~".to_vec(),
        NamedKey::PageDown => b"\x1b[6~".to_vec(),
        NamedKey::Insert => b"\x1b[2~".to_vec(),
        NamedKey::Delete => b"\x1b[3~".to_vec(),
        NamedKey::F1 => b"\x1bOP".to_vec(),
        NamedKey::F2 => b"\x1bOQ".to_vec(),
        NamedKey::F3 => b"\x1bOR".to_vec(),
        NamedKey::F4 => b"\x1bOS".to_vec(),
        NamedKey::F5 => b"\x1b[15~".to_vec(),
        NamedKey::F6 => b"\x1b[17~".to_vec(),
        NamedKey::F7 => b"\x1b[18~".to_vec(),
        NamedKey::F8 => b"\x1b[19~".to_vec(),
        NamedKey::F9 => b"\x1b[20~".to_vec(),
        NamedKey::F10 => b"\x1b[21~".to_vec(),
        NamedKey::F11 => b"\x1b[23~".to_vec(),
        NamedKey::F12 => b"\x1b[24~".to_vec(),
        _ => vec![],
    }
}

pub fn do_copy(terminal: &TerminalState, clipboard: &Option<WaylandClipboard>) {
    // Copy selection if any, otherwise copy full grid
    let text = if let Some(selected) = terminal.selected_text() {
        selected
    } else {
        let mut t = String::new();
        for row in 0..terminal.rows {
            let line = &terminal.grid[row];
            let row_text: String = line.iter().map(|c| c.c).collect();
            let trimmed = row_text.trim_end();
            t.push_str(trimmed);
            if row < terminal.rows - 1 {
                t.push('\n');
            }
        }
        t.trim_end_matches('\n').to_string()
    };
    if let Some(cb) = clipboard {
        cb.set_text(&text);
    }
}

pub fn do_paste(clipboard: &Option<WaylandClipboard>, pty: &Pty) {
    let text = if let Some(cb) = clipboard {
        cb.get_text()
    } else {
        None
    };

    if let Some(text) = text {
        if !text.is_empty() {
            pty.write(b"\x1b[200~");
            pty.write(text.as_bytes());
            pty.write(b"\x1b[201~");
        }
    }
}

fn do_new_window() {
    if let Ok(exe) = std::env::current_exe() {
        std::process::Command::new(exe).spawn().ok();
    }
}

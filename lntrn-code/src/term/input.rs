use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::pty::Pty;
use super::grid::TerminalState;

/// Process a winit keyboard event and write appropriate bytes to PTY.
/// Returns true if the event was handled.
pub fn handle_key(
    key: &Key,
    state: ElementState,
    modifiers: ModifiersState,
    terminal: &mut TerminalState,
    pty: &Pty,
) -> bool {
    if state != ElementState::Pressed {
        return false;
    }

    let ctrl = modifiers.contains(ModifiersState::CONTROL);
    let shift = modifiers.contains(ModifiersState::SHIFT);

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
        let alt = modifiers.contains(ModifiersState::ALT);
        let seq = named_key_to_seq(named, terminal.application_cursor, shift, ctrl, alt);
        if !seq.is_empty() {
            terminal.scroll_offset = 0;
            terminal.clear_selection();
            pty.write(&seq);
            return true;
        }
    }

    // Regular text input
    if let Key::Character(s) = key {
        let alt = modifiers.contains(ModifiersState::ALT);
        if !ctrl {
            terminal.scroll_offset = 0;
            terminal.clear_selection();
            if alt {
                // Alt+char sends ESC prefix then the character
                let mut seq = vec![0x1b];
                seq.extend_from_slice(s.as_bytes());
                pty.write(&seq);
            } else {
                pty.write(s.as_bytes());
            }
            return true;
        }
    }

    false
}

/// Compute the xterm modifier parameter: 1 + (shift?1:0) + (alt?2:0) + (ctrl?4:0).
/// Returns 0 when no modifiers are held (meaning: use the plain sequence).
fn modifier_param(shift: bool, ctrl: bool, alt: bool) -> u8 {
    let bits = (shift as u8) | ((alt as u8) << 1) | ((ctrl as u8) << 2);
    if bits == 0 { 0 } else { 1 + bits }
}

/// Build a CSI sequence with an optional modifier parameter.
/// `base` is the final char (e.g. b'A' for Up). When `num` is non-zero the
/// format is `ESC [ num ; mod base`, otherwise `ESC [ 1 ; mod base`.
fn csi_modified(num: u8, base: u8, modp: u8) -> Vec<u8> {
    if modp == 0 {
        if num == 0 {
            return vec![0x1b, b'[', base];
        } else {
            return format!("\x1b[{}~", num).into_bytes();
        }
    }
    if num == 0 {
        format!("\x1b[1;{}{}", modp, base as char).into_bytes()
    } else {
        format!("\x1b[{};{}~", num, modp).into_bytes()
    }
}

fn named_key_to_seq(key: &NamedKey, app_cursor: bool, shift: bool, ctrl: bool, alt: bool) -> Vec<u8> {
    let modp = modifier_param(shift, ctrl, alt);
    let has_mods = modp != 0;

    match key {
        NamedKey::Space => vec![0x20],
        NamedKey::Enter => vec![0x0D],
        NamedKey::Tab => {
            if shift {
                b"\x1b[Z".to_vec() // Reverse tab / backtab
            } else {
                vec![0x09]
            }
        }
        NamedKey::Backspace => vec![0x7F],
        NamedKey::Escape => vec![0x1B],

        // Arrow keys: plain uses app_cursor mode, modified always uses CSI
        NamedKey::ArrowUp => {
            if has_mods {
                csi_modified(0, b'A', modp)
            } else if app_cursor {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        NamedKey::ArrowDown => {
            if has_mods {
                csi_modified(0, b'B', modp)
            } else if app_cursor {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        NamedKey::ArrowRight => {
            if has_mods {
                csi_modified(0, b'C', modp)
            } else if app_cursor {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        NamedKey::ArrowLeft => {
            if has_mods {
                csi_modified(0, b'D', modp)
            } else if app_cursor {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }

        // Home/End: modified uses CSI 1;mod H/F
        NamedKey::Home => {
            if has_mods {
                csi_modified(0, b'H', modp)
            } else {
                b"\x1b[H".to_vec()
            }
        }
        NamedKey::End => {
            if has_mods {
                csi_modified(0, b'F', modp)
            } else {
                b"\x1b[F".to_vec()
            }
        }

        // Tilde-style keys: ESC [ num ; mod ~
        NamedKey::PageUp => csi_modified(5, b'~', modp),
        NamedKey::PageDown => csi_modified(6, b'~', modp),
        NamedKey::Insert => csi_modified(2, b'~', modp),
        NamedKey::Delete => csi_modified(3, b'~', modp),

        // Function keys: F1-F4 use SS3 plain, CSI 1;mod P/Q/R/S modified
        NamedKey::F1 => {
            if has_mods { format!("\x1b[1;{}P", modp).into_bytes() } else { b"\x1bOP".to_vec() }
        }
        NamedKey::F2 => {
            if has_mods { format!("\x1b[1;{}Q", modp).into_bytes() } else { b"\x1bOQ".to_vec() }
        }
        NamedKey::F3 => {
            if has_mods { format!("\x1b[1;{}R", modp).into_bytes() } else { b"\x1bOR".to_vec() }
        }
        NamedKey::F4 => {
            if has_mods { format!("\x1b[1;{}S", modp).into_bytes() } else { b"\x1bOS".to_vec() }
        }
        NamedKey::F5 => csi_modified(15, b'~', modp),
        NamedKey::F6 => csi_modified(17, b'~', modp),
        NamedKey::F7 => csi_modified(18, b'~', modp),
        NamedKey::F8 => csi_modified(19, b'~', modp),
        NamedKey::F9 => csi_modified(20, b'~', modp),
        NamedKey::F10 => csi_modified(21, b'~', modp),
        NamedKey::F11 => csi_modified(23, b'~', modp),
        NamedKey::F12 => csi_modified(24, b'~', modp),
        _ => vec![],
    }
}

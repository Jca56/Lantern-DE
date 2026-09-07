//! Keys for a focused terminal, turned into the bytes a program expects:
//! control characters, escape sequences for the navigation keys (with
//! modifier parameters), Alt as an escape prefix, and pastes (bracketed
//! when the program asked). Typed text arrives separately and goes
//! through as UTF-8.

use lntrn_ui::{Key, KeyPress, Ui, UiState};

use super::Terminal;

/// Whether the terminal takes this key. Ctrl+Q, Ctrl+`, Ctrl+Shift+*
/// (except paste and copy), Ctrl+Tab/PageUp/PageDown and Ctrl+Space stay
/// with the app.
pub fn handles(k: &KeyPress) -> bool {
    let m = k.mods;
    if m.super_key() {
        return false;
    }
    let (ctrl, shift) = (m.ctrl(), m.shift());
    match k.key {
        Key::Char(c) => {
            let l = c.to_ascii_lowercase();
            if ctrl && shift {
                matches!(l, 'v' | 'c' | 'f')
            } else if ctrl {
                !matches!(l, 'q' | '`')
            } else {
                true
            }
        }
        Key::Tab => !ctrl,
        Key::Space => !ctrl,
        Key::PageUp | Key::PageDown => !ctrl,
        Key::Enter | Key::Backspace | Key::Delete | Key::Escape | Key::Insert | Key::Home | Key::End | Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::F(_) => !(ctrl && shift),
        _ => false,
    }
}

/// The bytes for a key, or an empty vector when typed text carries it.
pub fn key_bytes(k: &KeyPress, app_cursor: bool) -> Vec<u8> {
    let m = k.mods;
    let (ctrl, shift, alt) = (m.ctrl(), m.shift(), m.alt());
    let mods = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let cursor = |f: char| -> Vec<u8> {
        if mods > 1 {
            format!("\x1b[1;{mods}{f}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{f}").into_bytes()
        } else {
            format!("\x1b[{f}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> { if mods > 1 { format!("\x1b[{n};{mods}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() } };
    match k.key {
        Key::Enter => {
            if alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        Key::Tab => {
            if shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        Key::Backspace => {
            let mut v = Vec::new();
            if alt {
                v.push(0x1b);
            }
            v.push(if ctrl { 0x08 } else { 0x7f });
            v
        }
        Key::Delete => tilde(3),
        Key::Insert => tilde(2),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::ArrowUp => cursor('A'),
        Key::ArrowDown => cursor('B'),
        Key::ArrowRight => cursor('C'),
        Key::ArrowLeft => cursor('D'),
        Key::Escape => {
            if alt {
                b"\x1b\x1b".to_vec()
            } else {
                b"\x1b".to_vec()
            }
        }
        Key::F(n) => match n {
            1..=4 => {
                let f = (b'P' + n - 1) as char;
                if mods > 1 { format!("\x1b[1;{mods}{f}").into_bytes() } else { format!("\x1bO{f}").into_bytes() }
            }
            5 => tilde(15),
            6 => tilde(17),
            7 => tilde(18),
            8 => tilde(19),
            9 => tilde(20),
            10 => tilde(21),
            11 => tilde(23),
            12 => tilde(24),
            _ => Vec::new(),
        },
        Key::Space => {
            if ctrl {
                vec![0]
            } else if alt {
                b"\x1b ".to_vec()
            } else {
                Vec::new()
            }
        }
        Key::Char(c) => {
            let mut v = Vec::new();
            if ctrl {
                let byte = match c.to_ascii_lowercase() {
                    l @ 'a'..='z' => l as u8 - b'a' + 1,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '7' | '/' => 0x1f,
                    '2' | '@' | ' ' => 0,
                    '?' | '8' => 0x7f,
                    _ => return v,
                };
                if alt {
                    v.push(0x1b);
                }
                v.push(byte);
            } else if alt {
                v.push(0x1b);
                let mut buf = [0u8; 4];
                v.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            v
        }
        _ => Vec::new(),
    }
}

enum Edit {
    Key(KeyPress),
    Text(String),
}

fn take_edits(state: &mut UiState) -> Vec<Edit> {
    let mut out: Vec<(u32, Edit)> = Vec::new();
    state.keys.retain(|k| {
        if handles(k) {
            out.push((k.seq, Edit::Key(*k)));
            false
        } else {
            true
        }
    });
    let alt = state.mods.alt();
    for (seq, t) in state.text_input.drain(..) {
        if alt && t.is_ascii() {
            continue;
        }
        out.push((seq, Edit::Text(t)));
    }
    out.sort_by_key(|(seq, _)| *seq);
    out.into_iter().map(|(_, e)| e).collect()
}

/// This frame's keys and text to the terminal. A paste asked for last
/// frame goes first, now that the clipboard has been read.
pub fn handle(ui: &mut Ui, term: &mut Terminal) {
    if std::mem::take(&mut term.paste_pending) {
        let text = ui.state.clipboard.clone();
        if !text.is_empty() {
            term.paste(&text);
        }
    }
    let app_cursor = term.grid.app_cursor;
    let mut out = Vec::new();
    let mut want_paste = false;
    for e in take_edits(ui.state) {
        match e {
            Edit::Text(t) => out.extend_from_slice(t.as_bytes()),
            Edit::Key(k) => {
                let paste_key = (k.mods.ctrl() && k.mods.shift() && matches!(k.key, Key::Char('v' | 'V'))) || (k.mods.shift() && k.key == Key::Insert);
                if paste_key {
                    want_paste = true;
                    continue;
                }
                if k.mods.ctrl() && k.mods.shift() && matches!(k.key, Key::Char('f' | 'F')) {
                    let s = term.search.get_or_insert_with(super::search::TermSearch::new);
                    s.focus = true;
                    ui.state.request_rebuild = true;
                    continue;
                }
                if k.mods.ctrl() && k.mods.shift() && matches!(k.key, Key::Char('c' | 'C')) {
                    if let Some(text) = term.selection_text() {
                        ui.state.set_clipboard(text);
                    }
                    continue;
                }
                if term.exited.is_some() && k.key == Key::Enter {
                    term.respawn();
                    ui.state.request_rebuild = true;
                    continue;
                }
                out.extend(key_bytes(&k, app_cursor));
            }
        }
    }
    if !out.is_empty() {
        term.write(&out);
        term.grid.view_offset = 0;
        ui.state.request_rebuild = true;
    }
    if want_paste {
        term.paste_pending = true;
        ui.state.clipboard_wanted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_ui::Modifiers;

    fn press(key: Key, mods: Modifiers) -> KeyPress {
        KeyPress { key, mods, repeat: false, seq: 0 }
    }

    #[test]
    fn bytes_for_keys() {
        let none = Modifiers::NONE;
        assert_eq!(key_bytes(&press(Key::Enter, none), false), b"\r");
        assert_eq!(key_bytes(&press(Key::ArrowUp, none), false), b"\x1b[A");
        assert_eq!(key_bytes(&press(Key::ArrowUp, none), true), b"\x1bOA");
        assert_eq!(key_bytes(&press(Key::ArrowUp, Modifiers::CTRL), true), b"\x1b[1;5A");
        assert_eq!(key_bytes(&press(Key::Delete, Modifiers::SHIFT), false), b"\x1b[3;2~");
        assert_eq!(key_bytes(&press(Key::Char('c'), Modifiers::CTRL), false), [3]);
        assert_eq!(key_bytes(&press(Key::Char('x'), Modifiers::ALT), false), b"\x1bx");
        assert_eq!(key_bytes(&press(Key::Char('x'), none), false), b"", "typed text carries it");
        assert_eq!(key_bytes(&press(Key::F(1), none), false), b"\x1bOP");
        assert_eq!(key_bytes(&press(Key::F(5), none), false), b"\x1b[15~");
        assert_eq!(key_bytes(&press(Key::Tab, Modifiers::SHIFT), false), b"\x1b[Z");
        assert!(handles(&press(Key::Char('c'), Modifiers::CTRL)));
        assert!(!handles(&press(Key::Char('q'), Modifiers::CTRL)));
        assert!(!handles(&press(Key::Char('p'), Modifiers::CTRL | Modifiers::SHIFT)));
        assert!(handles(&press(Key::Char('v'), Modifiers::CTRL | Modifiers::SHIFT)));
        assert!(!handles(&press(Key::Tab, Modifiers::CTRL)));
    }
}

use std::time::Instant;

use crate::terminal::TerminalState;
use crate::terminal_render::measure_cell;
use crate::pty::Pty;

/// A single terminal session (PTY + grid).
pub struct TerminalSession {
    pub terminal: TerminalState,
    pub pty: Pty,
}

impl TerminalSession {
    fn new(cols: usize, rows: usize) -> Self {
        let terminal = TerminalState::new(cols, rows);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let pty = Pty::spawn(&shell, None, Box::new(|| {}))
            .expect("Failed to spawn PTY");
        pty.resize(cols as u16, rows as u16);
        Self { terminal, pty }
    }

    fn tick(&mut self) {
        while let Some((data, has_more)) = self.pty.read(8192) {
            self.terminal.process(&data);
            for resp in self.terminal.pending_responses.drain(..) {
                self.pty.write(&resp);
            }
            if !has_more { break; }
        }
    }

    /// Tab label: use OSC title, or CWD basename, or "Terminal".
    pub fn label(&self) -> String {
        if let Some(ref title) = self.terminal.title {
            if !title.is_empty() { return title.clone(); }
        }
        if let Some(cwd) = self.pty.cwd() {
            if let Some(name) = std::path::Path::new(&cwd).file_name() {
                return name.to_string_lossy().to_string();
            }
        }
        "Terminal".into()
    }
}

pub const TAB_BAR_HEIGHT: f32 = 36.0;

/// Manages multiple terminal sessions with a tab bar.
pub struct TerminalPanel {
    pub sessions: Vec<TerminalSession>,
    pub active: usize,
    pub font_size: f32,
    pub cursor_visible: bool,
    pub cursor_blink_deadline: Instant,
}

impl TerminalPanel {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            sessions: vec![TerminalSession::new(cols, rows)],
            active: 0,
            font_size: 20.0,
            cursor_visible: true,
            cursor_blink_deadline: Instant::now() + std::time::Duration::from_millis(500),
        }
    }

    pub fn active_session(&self) -> &TerminalSession {
        &self.sessions[self.active]
    }

    pub fn active_session_mut(&mut self) -> &mut TerminalSession {
        &mut self.sessions[self.active]
    }

    /// Drain PTY output for ALL sessions + toggle cursor blink.
    pub fn tick(&mut self) {
        for session in &mut self.sessions {
            session.tick();
        }
        // Remove dead sessions (but keep at least one)
        if self.sessions.len() > 1 {
            let mut i = 0;
            while i < self.sessions.len() {
                if !self.sessions[i].pty.alive && self.sessions.len() > 1 {
                    self.sessions.remove(i);
                    if self.active >= self.sessions.len() {
                        self.active = self.sessions.len() - 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
        let now = Instant::now();
        if now >= self.cursor_blink_deadline {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_deadline = now + std::time::Duration::from_millis(500);
        }
    }

    pub fn handle_key(&mut self, key_code: u32, ctrl: bool, shift: bool, alt: bool) {
        // Ctrl+Shift+T: new tab
        if ctrl && shift && key_code == 20 {
            self.new_tab();
            return;
        }
        // Ctrl+Shift+W: close current tab
        if ctrl && shift && key_code == 17 {
            self.close_tab(self.active);
            return;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab: cycle tabs
        if ctrl && key_code == 15 {
            if shift {
                self.prev_tab();
            } else {
                self.next_tab();
            }
            return;
        }
        let session = &mut self.sessions[self.active];
        let seq = keycode_to_seq(key_code, ctrl, shift, alt, session.terminal.application_cursor);
        if !seq.is_empty() {
            session.terminal.scroll_offset = 0;
            session.pty.write(&seq);
        }
    }

    pub fn new_tab(&mut self) {
        let (cols, rows) = if let Some(s) = self.sessions.first() {
            (s.terminal.cols, s.terminal.rows)
        } else {
            (80, 24)
        };
        self.sessions.push(TerminalSession::new(cols, rows));
        self.active = self.sessions.len() - 1;
    }

    pub fn close_tab(&mut self, idx: usize) {
        if self.sessions.len() <= 1 { return; }
        let mut session = self.sessions.remove(idx);
        session.pty.cleanup();
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
    }

    pub fn next_tab(&mut self) {
        self.active = (self.active + 1) % self.sessions.len();
    }

    pub fn prev_tab(&mut self) {
        self.active = if self.active == 0 { self.sessions.len() - 1 } else { self.active - 1 };
    }

    pub fn switch_tab(&mut self, idx: usize) {
        if idx < self.sessions.len() {
            self.active = idx;
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 { return; }
        for session in &mut self.sessions {
            session.terminal.resize(cols, rows);
            session.pty.resize(cols as u16, rows as u16);
        }
    }

    pub fn calc_grid_size(width: f32, height: f32, font_size: f32) -> (usize, usize) {
        let (cell_w, cell_h) = measure_cell(font_size);
        let cols = (width / cell_w).floor() as usize;
        let rows = (height / cell_h).floor() as usize;
        (cols.max(2), rows.max(2))
    }

    pub fn tab_labels(&self) -> Vec<String> {
        self.sessions.iter().map(|s| s.label()).collect()
    }
}

// ── Key translation (raw Linux keycodes → terminal escape sequences) ───────

fn keycode_to_seq(
    key: u32,
    ctrl: bool,
    shift: bool,
    alt: bool,
    app_cursor: bool,
) -> Vec<u8> {
    if ctrl && !shift {
        if let Some(ch) = keycode_to_char(key) {
            if ch.is_ascii_alphabetic() {
                let byte = ch.to_ascii_lowercase() as u8 - b'a' + 1;
                return vec![byte];
            }
        }
    }

    let modp = modifier_param(shift, ctrl, alt);

    match key {
        28 => return vec![0x0D],         // Enter
        14 => return vec![0x7F],         // Backspace
        15 => {                          // Tab
            if shift { return b"\x1b[Z".to_vec(); }
            return vec![0x09];
        }
        1 => return vec![0x1B],          // Escape
        57 => return vec![0x20],         // Space
        103 => return arrow_seq(b'A', modp, app_cursor),
        108 => return arrow_seq(b'B', modp, app_cursor),
        106 => return arrow_seq(b'C', modp, app_cursor),
        105 => return arrow_seq(b'D', modp, app_cursor),
        102 => return if modp == 0 { b"\x1b[H".to_vec() } else { csi_modified(0, b'H', modp) },
        107 => return if modp == 0 { b"\x1b[F".to_vec() } else { csi_modified(0, b'F', modp) },
        104 => return csi_modified(5, b'~', modp),
        109 => return csi_modified(6, b'~', modp),
        110 => return csi_modified(2, b'~', modp),
        111 => return csi_modified(3, b'~', modp),
        59 => return if modp == 0 { b"\x1bOP".to_vec() } else { format!("\x1b[1;{}P", modp).into_bytes() },
        60 => return if modp == 0 { b"\x1bOQ".to_vec() } else { format!("\x1b[1;{}Q", modp).into_bytes() },
        61 => return if modp == 0 { b"\x1bOR".to_vec() } else { format!("\x1b[1;{}R", modp).into_bytes() },
        62 => return if modp == 0 { b"\x1bOS".to_vec() } else { format!("\x1b[1;{}S", modp).into_bytes() },
        63 => return csi_modified(15, b'~', modp),
        64 => return csi_modified(17, b'~', modp),
        65 => return csi_modified(18, b'~', modp),
        66 => return csi_modified(19, b'~', modp),
        67 => return csi_modified(20, b'~', modp),
        68 => return csi_modified(21, b'~', modp),
        87 => return csi_modified(23, b'~', modp),
        88 => return csi_modified(24, b'~', modp),
        _ => {}
    }

    if !ctrl {
        if let Some(ch) = keycode_to_char_shifted(key, shift) {
            if alt {
                let mut seq = vec![0x1b];
                let mut buf = [0u8; 4];
                seq.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                return seq;
            }
            let mut buf = [0u8; 4];
            return ch.encode_utf8(&mut buf).as_bytes().to_vec();
        }
    }

    vec![]
}

fn modifier_param(shift: bool, ctrl: bool, alt: bool) -> u8 {
    let bits = (shift as u8) | ((alt as u8) << 1) | ((ctrl as u8) << 2);
    if bits == 0 { 0 } else { 1 + bits }
}

fn csi_modified(num: u8, base: u8, modp: u8) -> Vec<u8> {
    if modp == 0 {
        if num == 0 { vec![0x1b, b'[', base] }
        else { format!("\x1b[{}~", num).into_bytes() }
    } else if num == 0 {
        format!("\x1b[1;{}{}", modp, base as char).into_bytes()
    } else {
        format!("\x1b[{};{}~", num, modp).into_bytes()
    }
}

fn arrow_seq(base: u8, modp: u8, app_cursor: bool) -> Vec<u8> {
    if modp != 0 { csi_modified(0, base, modp) }
    else if app_cursor { vec![0x1b, b'O', base] }
    else { vec![0x1b, b'[', base] }
}

fn keycode_to_char(key: u32) -> Option<char> {
    match key {
        2 => Some('1'), 3 => Some('2'), 4 => Some('3'), 5 => Some('4'),
        6 => Some('5'), 7 => Some('6'), 8 => Some('7'), 9 => Some('8'),
        10 => Some('9'), 11 => Some('0'),
        16 => Some('q'), 17 => Some('w'), 18 => Some('e'), 19 => Some('r'),
        20 => Some('t'), 21 => Some('y'), 22 => Some('u'), 23 => Some('i'),
        24 => Some('o'), 25 => Some('p'),
        30 => Some('a'), 31 => Some('s'), 32 => Some('d'), 33 => Some('f'),
        34 => Some('g'), 35 => Some('h'), 36 => Some('j'), 37 => Some('k'),
        38 => Some('l'),
        44 => Some('z'), 45 => Some('x'), 46 => Some('c'), 47 => Some('v'),
        48 => Some('b'), 49 => Some('n'), 50 => Some('m'),
        12 => Some('-'), 13 => Some('='),
        26 => Some('['), 27 => Some(']'),
        39 => Some(';'), 40 => Some('\''),
        41 => Some('`'), 43 => Some('\\'),
        51 => Some(','), 52 => Some('.'), 53 => Some('/'),
        _ => None,
    }
}

fn keycode_to_char_shifted(key: u32, shift: bool) -> Option<char> {
    let base = keycode_to_char(key)?;
    if !shift { return Some(base); }
    let shifted = match base {
        'a'..='z' => (base as u8 - b'a' + b'A') as char,
        '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%',
        '6' => '^', '7' => '&', '8' => '*', '9' => '(', '0' => ')',
        '-' => '_', '=' => '+', '[' => '{', ']' => '}',
        ';' => ':', '\'' => '"', '`' => '~', '\\' => '|',
        ',' => '<', '.' => '>', '/' => '?',
        _ => base,
    };
    Some(shifted)
}

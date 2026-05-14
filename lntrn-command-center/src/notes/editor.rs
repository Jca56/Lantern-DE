//! Tiny multi-line text editor used by the Notes body field.
//!
//! Cursor is a byte offset into a UTF-8 `String`. Up/Down arrows try to
//! preserve the visual column. Enter inserts a literal `\n`.

use crate::search::input::{keycode_to_char, KeyEffect};

pub const KEY_ENTER: u32 = 28;
pub const KEY_KP_ENTER: u32 = 96;
pub const KEY_BACKSPACE: u32 = 14;
pub const KEY_DELETE: u32 = 111;
pub const KEY_LEFT: u32 = 105;
pub const KEY_RIGHT: u32 = 106;
pub const KEY_UP: u32 = 103;
pub const KEY_DOWN: u32 = 108;
pub const KEY_HOME: u32 = 102;
pub const KEY_END: u32 = 107;
pub const KEY_TAB: u32 = 15;

pub struct Editor {
    buf: String,
    cursor: usize,
    /// Selection anchor (byte offset). When `Some` AND different from
    /// `cursor`, a range is selected and rendered with a highlight.
    anchor: Option<usize>,
    last_edit: std::time::Instant,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: 0,
            anchor: None,
            last_edit: std::time::Instant::now(),
        }
    }

    pub fn text(&self) -> String {
        self.buf.clone()
    }

    pub fn raw(&self) -> &str {
        &self.buf
    }

    pub fn cursor_byte(&self) -> usize {
        self.cursor
    }

    pub fn set_text(&mut self, text: &str) {
        self.buf.clear();
        self.buf.push_str(text);
        self.cursor = self.buf.len();
        self.anchor = None;
        self.touch();
    }

    // ── Selection API ──────────────────────────────────────────────────────

    pub fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn selected_text(&self) -> Option<&str> {
        let (s, e) = self.selection_range()?;
        Some(&self.buf[s..e])
    }

    pub fn select_all(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        self.anchor = Some(0);
        self.cursor = self.buf.len();
        self.touch();
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// Place cursor and start a fresh selection there (used on mousedown).
    pub fn begin_drag_at(&mut self, byte: usize) {
        let byte = clamp_to_char_boundary(&self.buf, byte);
        self.cursor = byte;
        self.anchor = Some(byte);
        self.touch();
    }

    /// Move cursor (used while a mouse drag is in progress). Leaves the
    /// anchor in place so the selection extends/shrinks.
    pub fn drag_to(&mut self, byte: usize) {
        let byte = clamp_to_char_boundary(&self.buf, byte);
        self.cursor = byte;
        self.touch();
    }

    /// Move cursor without changing the anchor — used for click-to-place.
    /// If no drag follows, the selection ends up empty.
    pub fn place_cursor(&mut self, byte: usize) {
        let byte = clamp_to_char_boundary(&self.buf, byte);
        self.cursor = byte;
        self.anchor = None;
        self.touch();
    }

    /// Replace the selection with `s` (or insert at the cursor if no
    /// selection). Returns true iff the buffer changed.
    pub fn delete_selection(&mut self) -> bool {
        let Some((s, e)) = self.selection_range() else {
            self.anchor = None;
            return false;
        };
        self.buf.replace_range(s..e, "");
        self.cursor = s;
        self.anchor = None;
        self.touch();
        true
    }

    pub fn on_key(&mut self, key: u32, shift: bool, caps_lock: bool) -> KeyEffect {
        match key {
            KEY_ENTER | KEY_KP_ENTER => {
                self.insert_str("\n");
                KeyEffect::ContentChanged
            }
            KEY_TAB => {
                self.insert_str("    ");
                KeyEffect::ContentChanged
            }
            KEY_BACKSPACE => self.delete_back(),
            KEY_DELETE => self.delete_forward(),
            KEY_LEFT => {
                if let Some((s, _e)) = self.selection_range() {
                    self.cursor = s;
                    self.anchor = None;
                    self.touch();
                    KeyEffect::CursorMoved
                } else {
                    self.move_left()
                }
            }
            KEY_RIGHT => {
                if let Some((_s, e)) = self.selection_range() {
                    self.cursor = e;
                    self.anchor = None;
                    self.touch();
                    KeyEffect::CursorMoved
                } else {
                    self.move_right()
                }
            }
            KEY_UP => {
                self.anchor = None;
                self.move_up()
            }
            KEY_DOWN => {
                self.anchor = None;
                self.move_down()
            }
            KEY_HOME => {
                self.anchor = None;
                self.move_line_start()
            }
            KEY_END => {
                self.anchor = None;
                self.move_line_end()
            }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift, caps_lock) {
                    let mut tmp = [0u8; 4];
                    self.insert_str(ch.encode_utf8(&mut tmp));
                    return KeyEffect::ContentChanged;
                }
                KeyEffect::None
            }
        }
    }

    pub fn cursor_visible(&self) -> bool {
        let since = self.last_edit.elapsed().as_millis();
        if since < 250 {
            return true;
        }
        ((since / 500) % 2) == 0
    }

    /// Line index (0-based) the cursor currently sits on.
    pub fn cursor_line(&self) -> usize {
        self.buf[..self.cursor].matches('\n').count()
    }

    /// Byte offset of the start of the current line.
    pub fn line_start(&self) -> usize {
        self.buf[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// Byte offset of the end of the current line (exclusive of \n).
    pub fn line_end(&self) -> usize {
        self.buf[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.buf.len())
    }

    /// Character (not byte) column within the current line.
    pub fn cursor_column_chars(&self) -> usize {
        let start = self.line_start();
        self.buf[start..self.cursor].chars().count()
    }

    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        self.buf.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.touch();
    }

    fn delete_back(&mut self) -> KeyEffect {
        if self.delete_selection() {
            return KeyEffect::ContentChanged;
        }
        if self.cursor == 0 {
            return KeyEffect::None;
        }
        let prev = self.buf[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.buf.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.touch();
        KeyEffect::ContentChanged
    }

    fn delete_forward(&mut self) -> KeyEffect {
        if self.delete_selection() {
            return KeyEffect::ContentChanged;
        }
        if self.cursor >= self.buf.len() {
            return KeyEffect::None;
        }
        let next = self.buf[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buf.len());
        self.buf.replace_range(self.cursor..next, "");
        self.touch();
        KeyEffect::ContentChanged
    }

    fn move_left(&mut self) -> KeyEffect {
        if self.cursor == 0 {
            return KeyEffect::None;
        }
        self.cursor = self.buf[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_right(&mut self) -> KeyEffect {
        if self.cursor >= self.buf.len() {
            return KeyEffect::None;
        }
        self.cursor = self.buf[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buf.len());
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_line_start(&mut self) -> KeyEffect {
        let new = self.line_start();
        if new == self.cursor {
            return KeyEffect::None;
        }
        self.cursor = new;
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_line_end(&mut self) -> KeyEffect {
        let new = self.line_end();
        if new == self.cursor {
            return KeyEffect::None;
        }
        self.cursor = new;
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_up(&mut self) -> KeyEffect {
        let col = self.cursor_column_chars();
        let line_start = self.line_start();
        if line_start == 0 {
            return KeyEffect::None;
        }
        // Previous line ends at line_start - 1 ('\n'). Find its start.
        let prev_end = line_start - 1;
        let prev_start = self.buf[..prev_end]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        // Walk `col` chars into the previous line, clamped to its length.
        let prev_line = &self.buf[prev_start..prev_end];
        let take = col.min(prev_line.chars().count());
        let byte_in_line = prev_line
            .char_indices()
            .nth(take)
            .map(|(i, _)| i)
            .unwrap_or(prev_line.len());
        self.cursor = prev_start + byte_in_line;
        self.touch();
        KeyEffect::CursorMoved
    }

    fn move_down(&mut self) -> KeyEffect {
        let col = self.cursor_column_chars();
        let line_end = self.line_end();
        if line_end == self.buf.len() {
            return KeyEffect::None;
        }
        let next_start = line_end + 1; // skip the \n
        let next_end = self.buf[next_start..]
            .find('\n')
            .map(|i| next_start + i)
            .unwrap_or(self.buf.len());
        let next_line = &self.buf[next_start..next_end];
        let take = col.min(next_line.chars().count());
        let byte_in_line = next_line
            .char_indices()
            .nth(take)
            .map(|(i, _)| i)
            .unwrap_or(next_line.len());
        self.cursor = next_start + byte_in_line;
        self.touch();
        KeyEffect::CursorMoved
    }

    fn touch(&mut self) {
        self.last_edit = std::time::Instant::now();
    }
}

/// Clamp `byte` to the nearest UTF-8 char boundary at or before it,
/// also clamping to the buffer length. Avoids panics when click/drag
/// math lands in the middle of a multi-byte codepoint.
fn clamp_to_char_boundary(buf: &str, byte: usize) -> usize {
    let mut b = byte.min(buf.len());
    while b > 0 && !buf.is_char_boundary(b) {
        b -= 1;
    }
    b
}

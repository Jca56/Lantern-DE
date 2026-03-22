use xkbcommon::xkb;

/// Keyboard state manager using xkbcommon for keymap translation.
pub struct KeyboardState {
    context: xkb::Context,
    keymap: Option<xkb::Keymap>,
    state: Option<xkb::State>,
}

impl KeyboardState {
    pub fn new() -> Self {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        Self { context, keymap: None, state: None }
    }

    /// Called when wl_keyboard sends a keymap event.
    /// `format` should be XkbV1, `fd` is the file descriptor, `size` is the map size.
    pub fn update_keymap(&mut self, fd: std::os::fd::RawFd, size: u32) {
        use std::os::fd::FromRawFd;
        use std::io::Read;
        // SAFETY: We take ownership of the fd (it was mem::forget'd to avoid double-close).
        let map_str = unsafe {
            let file = std::fs::File::from_raw_fd(fd);
            let mut buf = Vec::with_capacity(size as usize);
            let mut reader = std::io::BufReader::new(&file);
            let _ = reader.read_to_end(&mut buf);
            // Truncate trailing nulls from the keymap
            while buf.last() == Some(&0) { buf.pop(); }
            String::from_utf8_lossy(&buf).into_owned()
        };

        if let Some(keymap) = xkb::Keymap::new_from_string(
            &self.context,
            map_str,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) {
            let state = xkb::State::new(&keymap);
            self.keymap = Some(keymap);
            self.state = Some(state);
        }
    }

    /// Translate a raw keycode (evdev) to a UTF-8 string.
    /// Returns None if no printable character.
    pub fn key_to_utf8(&mut self, keycode: u32) -> Option<String> {
        let state = self.state.as_mut()?;
        let xkb_keycode = xkb::Keycode::new(keycode + 8);
        let utf8 = state.key_get_utf8(xkb_keycode);
        if utf8.is_empty() || utf8.chars().all(|c| c.is_control()) {
            None
        } else {
            Some(utf8)
        }
    }

    /// Get the keysym for a raw keycode.
    pub fn key_get_sym(&self, keycode: u32) -> xkb::Keysym {
        if let Some(state) = &self.state {
            state.key_get_one_sym(xkb::Keycode::new(keycode + 8))
        } else {
            xkb::Keysym::new(0)
        }
    }

    /// Update modifier state from wl_keyboard::modifiers event.
    pub fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(state) = &mut self.state {
            state.update_mask(depressed, latched, locked, 0, 0, group);
        }
    }
}

/// A simple single-line text buffer with cursor.
pub struct TextBuffer {
    pub text: String,
    pub cursor: usize,
}

impl TextBuffer {
    pub fn new(initial: &str) -> Self {
        let cursor = initial.chars().count();
        Self { text: initial.to_string(), cursor }
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.chars().count();
    }

    /// Insert text at cursor position.
    pub fn insert(&mut self, s: &str) {
        let byte_pos = self.byte_pos();
        self.text.insert_str(byte_pos, s);
        self.cursor += s.chars().count();
    }

    /// Delete character before cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let byte_pos = self.byte_pos();
            let prev_char_start = self.text[..byte_pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.drain(prev_char_start..byte_pos);
            self.cursor -= 1;
        }
    }

    /// Delete character at cursor (delete key).
    pub fn delete(&mut self) {
        let byte_pos = self.byte_pos();
        if byte_pos < self.text.len() {
            let next_char_end = self.text[byte_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| byte_pos + i)
                .unwrap_or(self.text.len());
            self.text.drain(byte_pos..next_char_end);
        }
    }

    /// Move cursor left.
    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right.
    pub fn right(&mut self) {
        if self.cursor < self.text.chars().count() {
            self.cursor += 1;
        }
    }

    /// Move cursor to start.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end.
    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// Get byte position from char cursor.
    fn byte_pos(&self) -> usize {
        self.text.char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }
}

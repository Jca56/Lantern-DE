use xkbcommon::xkb;

/// Keyboard state manager using xkbcommon for keymap translation.
/// Mirrors the proven pattern from lntrn-system-settings so the lock screen
/// honors the user's actual keyboard layout (important for non-US passwords).
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

    /// Called when wl_keyboard sends a keymap event (format XkbV1).
    pub fn update_keymap(&mut self, fd: std::os::fd::RawFd, size: u32) {
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let map_str = unsafe {
            let file = std::fs::File::from_raw_fd(fd);
            let mut buf = Vec::with_capacity(size as usize);
            let mut reader = std::io::BufReader::new(&file);
            let _ = reader.read_to_end(&mut buf);
            while buf.last() == Some(&0) {
                buf.pop();
            }
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

    /// Translate a raw evdev keycode to a UTF-8 string (printable chars only).
    pub fn key_to_utf8(&mut self, keycode: u32) -> Option<String> {
        let state = self.state.as_mut()?;
        let utf8 = state.key_get_utf8(xkb::Keycode::new(keycode + 8));
        if utf8.is_empty() || utf8.chars().all(|c| c.is_control()) {
            None
        } else {
            Some(utf8)
        }
    }

    /// Get the keysym for a raw evdev keycode (for Enter/Backspace/Escape).
    pub fn key_get_sym(&self, keycode: u32) -> xkb::Keysym {
        if let Some(state) = &self.state {
            state.key_get_one_sym(xkb::Keycode::new(keycode + 8))
        } else {
            xkb::Keysym::new(0)
        }
    }

    pub fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(state) = &mut self.state {
            state.update_mask(depressed, latched, locked, 0, 0, group);
        }
    }

    /// Whether Caps Lock is currently active.
    pub fn caps_active(&self) -> bool {
        self.state
            .as_ref()
            .map(|s| s.mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_EFFECTIVE))
            .unwrap_or(false)
    }
}

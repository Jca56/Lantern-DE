use xkbcommon::xkb;

pub struct KeyboardState {
    context: xkb::Context,
    keymap: Option<xkb::Keymap>,
    state: Option<xkb::State>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self {
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            keymap: None,
            state: None,
        }
    }

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

    pub fn key_to_utf8(&mut self, keycode: u32) -> Option<String> {
        let state = self.state.as_mut()?;
        let utf8 = state.key_get_utf8(xkb::Keycode::new(keycode + 8));
        if utf8.is_empty() || utf8.chars().all(|c| c.is_control()) {
            None
        } else {
            Some(utf8)
        }
    }

    pub fn key_get_sym(&self, keycode: u32) -> u32 {
        if let Some(state) = &self.state {
            state.key_get_one_sym(xkb::Keycode::new(keycode + 8)).raw()
        } else {
            0
        }
    }

    pub fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(state) = &mut self.state {
            state.update_mask(depressed, latched, locked, 0, 0, group);
        }
    }
}

// X11 keysym constants we care about.
pub const KEY_BACKSPACE: u32 = 0xff08;
pub const KEY_RETURN: u32 = 0xff0d;
pub const KEY_ESCAPE: u32 = 0xff1b;
pub const KEY_LEFT: u32 = 0xff51;
pub const KEY_RIGHT: u32 = 0xff53;
pub const KEY_HOME: u32 = 0xff50;
pub const KEY_END: u32 = 0xff57;
pub const KEY_DELETE: u32 = 0xffff;
pub const KEY_F2: u32 = 0xffbf;
pub const KEY_F5: u32 = 0xffc2;

/// True if the depressed modifier mask includes Ctrl.
pub fn ctrl_held(depressed: u32) -> bool {
    depressed & 0x04 != 0
}

/// True if the depressed modifier mask includes Shift.
pub fn shift_held(depressed: u32) -> bool {
    depressed & 0x01 != 0
}

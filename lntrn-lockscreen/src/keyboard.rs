use std::time::{Duration, Instant};

use xkbcommon::xkb;

/// What a key press means to the lock screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Enter,
    Backspace,
    Escape,
    /// A printable character (or a few, for keys that produce a sequence).
    Text(String),
}

const SYM_RETURN: u32 = 0xff0d; // XKB_KEY_Return
const SYM_KP_ENTER: u32 = 0xff8d;
const SYM_BACKSPACE: u32 = 0xff08;
const SYM_ESCAPE: u32 = 0xff1b;

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
        Self {
            context,
            keymap: None,
            state: None,
        }
    }

    /// Called when wl_keyboard sends a keymap event (format XkbV1). Takes
    /// ownership of the fd (closed on return). Returns true if a usable keymap
    /// was compiled.
    ///
    /// The fd MUST be mmap'd at offset 0, not `read()`: Wayland dup's the
    /// keymap fd to the client, and dup'd fds share a file offset. If the
    /// compositor left that offset at EOF after writing the keymap, a `read()`
    /// returns zero bytes and the keymap fails to compile.
    pub fn update_keymap(&mut self, fd: std::os::fd::OwnedFd, size: u32) -> bool {
        use std::os::fd::AsRawFd;
        let len = size as usize;
        if len == 0 {
            return false;
        }
        let map_str = unsafe {
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd.as_raw_fd(),
                0,
            );
            if ptr == libc::MAP_FAILED {
                return false;
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
            // Keymap is NUL-terminated; cut at the first NUL.
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
            let s = String::from_utf8_lossy(&bytes[..end]).into_owned();
            libc::munmap(ptr, len);
            s
        };
        // fd dropped here → closed.

        if let Some(keymap) = xkb::Keymap::new_from_string(
            &self.context,
            map_str,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) {
            let state = xkb::State::new(&keymap);
            self.keymap = Some(keymap);
            self.state = Some(state);
            true
        } else {
            false
        }
    }

    /// Resolve a raw evdev keycode against the keymap and the modifiers as
    /// they are right now. Call it the moment the key event arrives: resolved
    /// any later, a Shift released in the meantime changes which character
    /// the press was, and a correct password comes out wrong.
    pub fn resolve(&self, keycode: u32) -> Option<Key> {
        let state = self.state.as_ref()?;
        let code = xkb::Keycode::new(keycode + 8);
        match state.key_get_one_sym(code).raw() {
            SYM_RETURN | SYM_KP_ENTER => Some(Key::Enter),
            SYM_BACKSPACE => Some(Key::Backspace),
            SYM_ESCAPE => Some(Key::Escape),
            _ => {
                let utf8 = state.key_get_utf8(code);
                // Printable characters only.
                if utf8.is_empty() || utf8.chars().all(|c| c.is_control()) {
                    None
                } else {
                    Some(Key::Text(utf8))
                }
            }
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

/// Auto-repeat for a held key. Wayland leaves repeating to the client; the
/// compositor only says how fast (`wl_keyboard.repeat_info`).
///
/// Only Backspace is ever armed: a character that repeats is one a slow key
/// press can silently double, and in a masked field nobody would see it.
pub struct Repeat {
    /// Repeats per second; 0 turns repeating off.
    rate: i32,
    /// How long a key is held before it starts repeating.
    delay: Duration,
    /// The held key and when its next repeat is due.
    held: Option<(u32, Instant)>,
}

/// After a stall, fire at most this many repeats at once rather than every
/// one that was missed: a hiccup shouldn't wipe out the whole field.
const MAX_CATCH_UP: u32 = 4;

impl Repeat {
    pub fn new() -> Self {
        Self {
            rate: 25,
            delay: Duration::from_millis(400),
            held: None,
        }
    }

    pub fn set_info(&mut self, rate: i32, delay_ms: i32) {
        self.rate = rate.max(0);
        self.delay = Duration::from_millis(delay_ms.max(0) as u64);
        if self.rate == 0 {
            self.held = None;
        }
    }

    /// A key went down. `repeats` arms it; any other key ends the repeat of
    /// the one before, as on any keyboard.
    pub fn press(&mut self, keycode: u32, repeats: bool, now: Instant) {
        self.held = (repeats && self.rate > 0).then(|| (keycode, now + self.delay));
    }

    pub fn release(&mut self, keycode: u32) {
        if self.held.is_some_and(|(held, _)| held == keycode) {
            self.held = None;
        }
    }

    pub fn cancel(&mut self) {
        self.held = None;
    }

    /// When the next repeat is due, if a key is held.
    pub fn deadline(&self) -> Option<Instant> {
        self.held.map(|(_, at)| at)
    }

    /// How many repeats have come due by `now`.
    pub fn due(&mut self, now: Instant) -> u32 {
        let Some((_, next)) = self.held.as_mut() else {
            return 0;
        };
        let interval = Duration::from_secs(1) / self.rate.max(1) as u32;
        let mut fired = 0;
        while *next <= now && fired < MAX_CATCH_UP {
            *next += interval;
            fired += 1;
        }
        if *next <= now {
            *next = now + interval;
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_1: u32 = 2;
    const KEY_BACKSPACE: u32 = 14;
    const KEY_ENTER: u32 = 28;
    const KEY_A: u32 = 30;

    /// A keyboard with the US layout loaded, as if the compositor had sent it.
    fn us_keyboard() -> KeyboardState {
        let mut kb = KeyboardState::new();
        let keymap = xkb::Keymap::new_from_names(
            &kb.context,
            "",
            "",
            "us",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("the system has no xkb data for the us layout");
        kb.state = Some(xkb::State::new(&keymap));
        kb.keymap = Some(keymap);
        kb
    }

    fn shift(kb: &KeyboardState) -> u32 {
        1 << kb.keymap.as_ref().unwrap().mod_get_index(xkb::MOD_NAME_SHIFT)
    }

    fn text(s: &str) -> Option<Key> {
        Some(Key::Text(s.to_string()))
    }

    #[test]
    fn keys_resolve_to_what_they_type() {
        let kb = us_keyboard();
        assert_eq!(kb.resolve(KEY_A), text("a"));
        assert_eq!(kb.resolve(KEY_ENTER), Some(Key::Enter));
        assert_eq!(kb.resolve(KEY_BACKSPACE), Some(Key::Backspace));
        assert_eq!(kb.resolve(1), Some(Key::Escape));
        assert_eq!(kb.resolve(15), None, "Tab types nothing");
    }

    #[test]
    fn a_key_keeps_the_shift_it_was_pressed_with() {
        // Shift down, 'a' and '1' pressed, Shift up: all in one burst. Each
        // press is resolved as its event arrives, so the Shift that was held
        // then is the one that counts.
        let mut kb = us_keyboard();
        let shift = shift(&kb);

        kb.update_modifiers(shift, 0, 0, 0);
        let first = kb.resolve(KEY_A);
        let second = kb.resolve(KEY_1);
        kb.update_modifiers(0, 0, 0, 0);
        let third = kb.resolve(KEY_A);

        assert_eq!(first, text("A"));
        assert_eq!(second, text("!"));
        assert_eq!(third, text("a"));
    }

    #[test]
    fn caps_lock_is_reported() {
        let mut kb = us_keyboard();
        assert!(!kb.caps_active());
        let caps = 1 << kb.keymap.as_ref().unwrap().mod_get_index(xkb::MOD_NAME_CAPS);
        kb.update_modifiers(0, 0, caps, 0);
        assert!(kb.caps_active());
        assert_eq!(kb.resolve(KEY_A), text("A"));
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_held_key_repeats_after_the_delay() {
        let t0 = Instant::now();
        let mut repeat = Repeat::new();
        repeat.set_info(25, 200); // every 40 ms after 200 ms
        repeat.press(KEY_BACKSPACE, true, t0);

        assert_eq!(repeat.due(t0 + ms(199)), 0);
        assert_eq!(repeat.due(t0 + ms(200)), 1);
        assert_eq!(repeat.due(t0 + ms(239)), 0);
        assert_eq!(repeat.due(t0 + ms(280)), 2);
        assert_eq!(repeat.deadline(), Some(t0 + ms(320)));
    }

    #[test]
    fn letting_go_stops_the_repeat() {
        let t0 = Instant::now();
        let mut repeat = Repeat::new();
        repeat.press(KEY_BACKSPACE, true, t0);
        repeat.release(KEY_A); // some other key coming up changes nothing
        assert!(repeat.deadline().is_some());
        repeat.release(KEY_BACKSPACE);
        assert_eq!(repeat.due(t0 + ms(5000)), 0);
    }

    #[test]
    fn only_armed_keys_repeat() {
        let t0 = Instant::now();
        let mut repeat = Repeat::new();
        repeat.press(KEY_BACKSPACE, true, t0);
        repeat.press(KEY_A, false, t0 + ms(10)); // a letter: ends the Backspace repeat
        assert_eq!(repeat.due(t0 + ms(5000)), 0);
    }

    #[test]
    fn a_rate_of_zero_turns_repeat_off() {
        let t0 = Instant::now();
        let mut repeat = Repeat::new();
        repeat.set_info(0, 200);
        repeat.press(KEY_BACKSPACE, true, t0);
        assert_eq!(repeat.due(t0 + ms(5000)), 0);
    }

    #[test]
    fn a_stall_does_not_fire_every_missed_repeat() {
        let t0 = Instant::now();
        let mut repeat = Repeat::new();
        repeat.set_info(25, 200);
        repeat.press(KEY_BACKSPACE, true, t0);

        // Two seconds late: 45 repeats were missed, only a few are delivered.
        let late = t0 + ms(2000);
        assert_eq!(repeat.due(late), MAX_CATCH_UP);
        assert_eq!(repeat.deadline(), Some(late + ms(40)));
    }
}

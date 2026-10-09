//! Key presses as typed text, through the user's own keymap (xkbcommon),
//! for the text tool. The pattern is lntrn-lockscreen's.

use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use xkbcommon::xkb;

/// How long a key is held before it starts repeating, and how fast it
/// then repeats. The compositor's own figures arrive in `repeat_info`.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_EVERY: Duration = Duration::from_millis(33);

/// What a key press puts into the text being typed.
#[derive(Clone, PartialEq)]
pub enum Typed {
    Text(String),
    Backspace,
}

pub struct KeyboardState {
    context: xkb::Context,
    state: Option<xkb::State>,
    repeat_delay: Duration,
    repeat_every: Duration,
    /// The key being held and when it next repeats.
    held: Option<(u32, Typed, Instant)>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self {
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            state: None,
            repeat_delay: REPEAT_DELAY,
            repeat_every: REPEAT_EVERY,
            held: None,
        }
    }

    /// Called when wl_keyboard sends a keymap (format XkbV1). Takes
    /// ownership of the fd (closed on return).
    ///
    /// The fd MUST be mmap'd at offset 0, not `read()`: Wayland dup's the
    /// keymap fd to the client, and dup'd fds share a file offset. If the
    /// compositor left that offset at EOF after writing the keymap, a
    /// `read()` returns zero bytes and the keymap fails to compile.
    pub fn update_keymap(&mut self, fd: OwnedFd, size: u32) {
        let len = size as usize;
        if len == 0 {
            return;
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
                return;
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
            // Keymap is NUL-terminated; cut at the first NUL.
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
            let s = String::from_utf8_lossy(&bytes[..end]).into_owned();
            libc::munmap(ptr, len);
            s
        };
        if let Some(keymap) = xkb::Keymap::new_from_string(
            &self.context,
            map_str,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) {
            self.state = Some(xkb::State::new(&keymap));
        }
    }

    pub fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(state) = &mut self.state {
            state.update_mask(depressed, latched, locked, 0, 0, group);
        }
    }

    /// The compositor's repeat rate (keys per second; 0 turns repeat off)
    /// and the delay before it starts, in milliseconds.
    pub fn set_repeat(&mut self, rate: i32, delay: i32) {
        self.repeat_delay = Duration::from_millis(delay.max(0) as u64);
        self.repeat_every = if rate > 0 {
            Duration::from_millis((1000 / rate as u64).max(1))
        } else {
            Duration::MAX
        };
    }

    /// The printable text a raw evdev keycode types, if any.
    pub fn key_to_utf8(&self, keycode: u32) -> Option<String> {
        let utf8 = self
            .state
            .as_ref()?
            .key_get_utf8(xkb::Keycode::new(keycode + 8));
        if utf8.is_empty() || utf8.chars().all(|c| c.is_control()) {
            None
        } else {
            Some(utf8)
        }
    }

    /// A key that types went down: it repeats until it is released.
    pub fn hold(&mut self, keycode: u32, typed: Typed) {
        let next = Instant::now().checked_add(self.repeat_delay);
        self.held = next.map(|at| (keycode, typed, at));
    }

    /// A key went up (or focus left): if it was the one repeating, stop.
    pub fn release(&mut self, keycode: Option<u32>) {
        if keycode.is_none() || self.held.as_ref().map(|h| h.0) == keycode {
            self.held = None;
        }
    }

    /// The repeats of the held key that have come due since last asked.
    pub fn due_repeats(&mut self) -> Vec<Typed> {
        let mut due = Vec::new();
        let now = Instant::now();
        if let Some((_, typed, next)) = self.held.as_mut() {
            while *next <= now && due.len() < 32 {
                due.push(typed.clone());
                match next.checked_add(self.repeat_every) {
                    Some(at) => *next = at,
                    None => break,
                }
            }
        }
        due
    }
}

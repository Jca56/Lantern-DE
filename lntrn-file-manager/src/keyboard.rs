//! Keyboard input, read through the keymap the compositor sends.
//!
//! `wl_keyboard` delivers key *positions*; what a position means is in the
//! keymap. Every press is translated here, at the moment it is dispatched,
//! into a `KeyPress`: the character it types (layout, Shift, Caps Lock,
//! AltGr, dead keys and all) and a layout-independent key for shortcuts and
//! editing commands. Translating at dispatch time matters twice over:
//!  - the compositor's text injection (the Command Center's emoji picker)
//!    swaps in a throwaway keymap, taps one key per character and swaps
//!    back, all in one batch; each tap has to be read with the keymap that
//!    was current when it was sent;
//!  - presses are queued in order with the modifiers they were made with,
//!    so fast typing or a slow frame no longer drops or re-cases letters.

use std::collections::VecDeque;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use xkbcommon::xkb;

/// Layout-independent key codes: the evdev code the key has on a US
/// keyboard. `KeyPress::key` is one of these, or `NONE`.
pub mod code {
    pub const NONE: u32 = 0;
    pub const ESC: u32 = 1;
    pub const BACKSPACE: u32 = 14;
    pub const TAB: u32 = 15;
    pub const ENTER: u32 = 28;
    pub const SPACE: u32 = 57;
    pub const F2: u32 = 60;
    pub const F11: u32 = 87;
    pub const HOME: u32 = 102;
    pub const UP: u32 = 103;
    pub const PAGE_UP: u32 = 104;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const END: u32 = 107;
    pub const DOWN: u32 = 108;
    pub const PAGE_DOWN: u32 = 109;
    pub const DELETE: u32 = 111;
}

/// One key press, translated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    /// Which key, for shortcuts and editing commands: a `code::*` constant,
    /// the US-keyboard evdev code of a Latin letter (Ctrl+C is the key that
    /// types "c" in the active layout), or `code::NONE` for a key that only
    /// types text.
    pub key: u32,
    /// The character the press types, if it types one. Never a control
    /// character, and `None` while Ctrl, Alt or Super is held.
    pub ch: Option<char>,
    pub ctrl: bool,
    pub shift: bool,
    pub logo: bool,
}

const REPEAT_DELAY: Duration = Duration::from_millis(300);
const REPEAT_INTERVAL: Duration = Duration::from_millis(30);

struct Held {
    /// The wl_keyboard key, to match its release.
    raw: u32,
    press: KeyPress,
}

struct Xkb {
    keymap: xkb::Keymap,
    state: xkb::State,
}

enum Compose {
    /// Not looked for yet: the table is only loaded when a dead key or the
    /// Compose key is first pressed (it costs a few milliseconds, and most
    /// sessions never need it).
    Untried,
    Unavailable,
    Ready(xkb::compose::State),
}

pub struct Keyboard {
    context: xkb::Context,
    xkb: Option<Xkb>,
    compose: Compose,
    /// The last `wl_keyboard.modifiers` (depressed, latched, locked,
    /// group), to carry Caps Lock and friends over a keymap change.
    mods: (u32, u32, u32, u32),
    ctrl: bool,
    shift: bool,
    alt: bool,
    logo: bool,
    queue: VecDeque<KeyPress>,
    held: Option<Held>,
    repeat_deadline: Instant,
    repeat_started: bool,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb: None,
            compose: Compose::Untried,
            mods: (0, 0, 0, 0),
            ctrl: false,
            shift: false,
            alt: false,
            logo: false,
            queue: VecDeque::new(),
            held: None,
            repeat_deadline: Instant::now(),
            repeat_started: false,
        }
    }

    /// Modifiers as of the last event dispatched (for mouse clicks; key
    /// presses carry their own).
    pub fn ctrl(&self) -> bool {
        self.ctrl
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    /// `wl_keyboard.keymap`. `xkb_v1` says whether the format is the one
    /// libxkbcommon reads.
    pub fn on_keymap(&mut self, xkb_v1: bool, fd: OwnedFd, size: u32) {
        // Positions mean something else from here on: nothing held or half
        // composed carries over.
        self.held = None;
        self.reset_compose();
        self.xkb = None;
        if !xkb_v1 || size == 0 {
            return;
        }
        // Kept for a second way in, should mapping the fd fail.
        let spare = fd.try_clone().ok();
        // SAFETY: the fd is the keymap the compositor just handed us, mapped
        // privately and read-only for the length it announced.
        let mapped = unsafe {
            xkb::Keymap::new_from_fd(
                &self.context,
                fd,
                size as usize,
                xkb::KEYMAP_FORMAT_TEXT_V1,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        let keymap = match mapped {
            Ok(keymap) => keymap,
            Err(e) => {
                eprintln!("[fox] keyboard: cannot map the keymap ({e}), reading it instead");
                spare.and_then(|fd| self.keymap_by_reading(fd, size as usize))
            }
        };
        match keymap {
            Some(keymap) => self.set_keymap(keymap),
            // Without a keymap only the editing keys work (by position).
            None => eprintln!("[fox] keyboard: the compositor's keymap could not be used"),
        }
    }

    /// The keymap read from its fd (from the start, wherever another
    /// reader of the same file left the offset) instead of mapped.
    fn keymap_by_reading(&self, fd: OwnedFd, size: usize) -> Option<xkb::Keymap> {
        use std::os::unix::fs::FileExt;
        let mut text = vec![0u8; size];
        std::fs::File::from(fd).read_exact_at(&mut text, 0).ok()?;
        while text.last() == Some(&0) {
            text.pop();
        }
        xkb::Keymap::new_from_string(
            &self.context,
            String::from_utf8(text).ok()?,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    }

    fn set_keymap(&mut self, keymap: xkb::Keymap) {
        let state = xkb::State::new(&keymap);
        self.xkb = Some(Xkb { keymap, state });
        // A fresh state knows no modifiers; what is held or locked did not
        // change with the keymap.
        let (depressed, latched, locked, group) = self.mods;
        self.on_modifiers(depressed, latched, locked, group);
    }

    /// `wl_keyboard.modifiers`.
    pub fn on_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        self.mods = (depressed, latched, locked, group);
        match self.xkb.as_mut() {
            Some(x) => {
                x.state.update_mask(depressed, latched, locked, 0, 0, group);
                let on = |name: &str| x.state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
                self.ctrl = on(xkb::MOD_NAME_CTRL);
                self.shift = on(xkb::MOD_NAME_SHIFT);
                self.alt = on(xkb::MOD_NAME_ALT);
                self.logo = on(xkb::MOD_NAME_LOGO);
            }
            // No keymap: the core modifiers sit at fixed bits.
            None => {
                self.shift = depressed & 1 != 0;
                self.ctrl = depressed & 4 != 0;
                self.alt = depressed & 8 != 0;
                self.logo = depressed & 64 != 0;
            }
        }
    }

    /// `wl_keyboard.key`. A press is translated and queued.
    pub fn on_key(&mut self, raw: u32, pressed: bool, now: Instant) {
        if !pressed {
            if self.held.as_ref().is_some_and(|h| h.raw == raw) {
                self.held = None;
            }
            return;
        }
        let first = self.queue.len();
        self.translate(raw);
        // Hold the key for repeat when the keymap says it repeats (letters
        // do, Shift does not) and it is not one that commits or dismisses.
        let repeats = match &self.xkb {
            Some(x) => x.keymap.key_repeats(xkb::Keycode::new(raw + 8)),
            None => true,
        };
        if let Some(press) = self.queue.get(first).copied() {
            let one_shot = matches!(press.key, code::ESC | code::TAB | code::ENTER);
            if repeats && !one_shot && self.queue.len() == first + 1 {
                self.held = Some(Held { raw, press });
                self.repeat_deadline = now + REPEAT_DELAY;
                self.repeat_started = false;
            } else if one_shot {
                self.held = None;
            }
        }
    }

    /// `wl_keyboard.leave`: the releases of keys still down go to whoever
    /// has the focus now, and the compositor re-sends the modifiers on the
    /// next enter. Queued presses stay: they were made while focused.
    pub fn on_leave(&mut self) {
        self.held = None;
        self.repeat_started = false;
        // Nothing is held down any more as far as this window knows; what
        // is locked (Caps Lock) and the layout stay as they are.
        let (_, _, locked, group) = self.mods;
        self.on_modifiers(0, 0, locked, group);
        self.reset_compose();
    }

    /// The next queued press, oldest first.
    pub fn pop(&mut self) -> Option<KeyPress> {
        self.queue.pop_front()
    }

    /// Presses were just handled: a held key starts its repeat delay from
    /// here, so a handler that took long does not find it already expired.
    pub fn defer_repeat(&mut self, now: Instant) {
        if self.held.is_some() && !self.repeat_started {
            self.repeat_deadline = now + REPEAT_DELAY;
        }
    }

    /// When the held key repeats next, if one is held.
    pub fn repeat_at(&self) -> Option<Instant> {
        self.held.as_ref().map(|_| self.repeat_deadline)
    }

    /// The held key's press again, once its deadline has passed.
    pub fn take_repeat(&mut self, now: Instant) -> Option<KeyPress> {
        let press = self.held.as_ref()?.press;
        if now < self.repeat_deadline {
            return None;
        }
        self.repeat_deadline = now + REPEAT_INTERVAL;
        self.repeat_started = true;
        Some(press)
    }

    fn reset_compose(&mut self) {
        if let Compose::Ready(state) = &mut self.compose {
            state.reset();
        }
    }

    fn push(&mut self, key: u32, ch: Option<char>) {
        let chord = self.ctrl || self.alt || self.logo;
        let ch = ch.filter(|c| !chord && !c.is_control());
        if key == code::NONE && ch.is_none() {
            // A modifier, a function key nothing is bound to: not a press
            // anything here acts on (or should repeat).
            return;
        }
        self.queue.push_back(KeyPress {
            key,
            ch,
            ctrl: self.ctrl,
            shift: self.shift,
            logo: self.logo,
        });
    }

    fn translate(&mut self, raw: u32) {
        let Some(x) = &self.xkb else {
            // No keymap to read the key with: the editing keys still work
            // by position, nothing types.
            if let Some(key) = positional_special(raw) {
                self.push(key, None);
            }
            return;
        };
        let keycode = xkb::Keycode::new(raw + 8);
        let sym = x.state.key_get_one_sym(keycode);
        let ch = char::from_u32(x.state.key_get_utf32(keycode)).filter(|c| *c != '\0');
        // The letter this key has in a Latin layout of the keymap, for
        // shortcuts while a non-Latin layout is active.
        let latin = latin_letter(sym.raw()).or_else(|| {
            let layouts = x.keymap.num_layouts_for_key(keycode);
            (0..layouts).find_map(|layout| {
                match x.keymap.key_get_syms_by_level(keycode, layout, 0) {
                    [only] => latin_letter(only.raw()),
                    _ => None,
                }
            })
        });
        let key = logical_key(sym.raw(), latin, self.ctrl, raw);

        // A chord is a command, never part of a compose sequence: after a
        // dead key, Ctrl+X must still cut (dropped, it left the clipboard
        // holding whatever was cut before).
        if self.ctrl || self.alt || self.logo {
            self.reset_compose();
            self.push(key, ch);
            return;
        }
        // Enter, Esc, Delete, F2, the arrows: they end a sequence that was
        // waiting and still do what they say.
        let command = special_key(sym.raw()).is_some_and(|key| key != code::SPACE);
        match self.feed_compose(sym, command) {
            Composed::Passed => self.push(key, ch),
            Composed::Swallowed => {}
            Composed::Text(text) => {
                for c in text.chars() {
                    self.push(code::NONE, Some(c));
                }
            }
        }
    }

    /// Run the key through the compose sequences (dead keys, the Compose
    /// key): "´" then "e" types "é".
    fn feed_compose(&mut self, sym: xkb::Keysym, command: bool) -> Composed {
        if matches!(self.compose, Compose::Untried) {
            if !starts_compose(sym.raw()) {
                return Composed::Passed;
            }
            self.compose = load_compose(&self.context);
        }
        let Compose::Ready(state) = &mut self.compose else {
            return Composed::Passed;
        };
        if state.feed(sym) == xkb::compose::FeedResult::Ignored {
            // A modifier: not part of any sequence.
            return Composed::Passed;
        }
        match state.status() {
            xkb::compose::Status::Nothing => Composed::Passed,
            xkb::compose::Status::Composing => Composed::Swallowed,
            xkb::compose::Status::Composed => {
                let text = state.utf8().unwrap_or_default();
                state.reset();
                Composed::Text(text)
            }
            xkb::compose::Status::Cancelled => {
                state.reset();
                if command {
                    Composed::Passed
                } else {
                    Composed::Swallowed
                }
            }
        }
    }
}

enum Composed {
    /// Not part of a sequence: the key means what it says.
    Passed,
    /// In the middle of a sequence (or one that led nowhere).
    Swallowed,
    Text(String),
}

fn load_compose(context: &xkb::Context) -> Compose {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|var| std::env::var_os(var).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "C".into());
    match xkb::compose::Table::new_from_locale(context, &locale, xkb::compose::COMPILE_NO_FLAGS) {
        Ok(table) => Compose::Ready(xkb::compose::State::new(
            &table,
            xkb::compose::STATE_NO_FLAGS,
        )),
        Err(()) => Compose::Unavailable,
    }
}

/// Dead keys and the Compose key: the keysyms a compose sequence starts
/// with.
fn starts_compose(sym: u32) -> bool {
    const DEAD_FIRST: u32 = xkb::keysyms::KEY_dead_grave;
    const DEAD_LAST: u32 = xkb::keysyms::KEY_dead_longsolidusoverlay;
    (DEAD_FIRST..=DEAD_LAST).contains(&sym) || sym == xkb::keysyms::KEY_Multi_key
}

/// The editing and command keys, by keysym.
fn special_key(sym: u32) -> Option<u32> {
    use xkb::keysyms as ks;
    const KEYS: [(u32, u32); 27] = [
        (ks::KEY_Escape, code::ESC),
        (ks::KEY_BackSpace, code::BACKSPACE),
        (ks::KEY_Tab, code::TAB),
        (ks::KEY_ISO_Left_Tab, code::TAB),
        (ks::KEY_Return, code::ENTER),
        (ks::KEY_KP_Enter, code::ENTER),
        (ks::KEY_space, code::SPACE),
        (ks::KEY_F2, code::F2),
        (ks::KEY_F11, code::F11),
        (ks::KEY_Home, code::HOME),
        (ks::KEY_KP_Home, code::HOME),
        (ks::KEY_Left, code::LEFT),
        (ks::KEY_KP_Left, code::LEFT),
        (ks::KEY_Right, code::RIGHT),
        (ks::KEY_KP_Right, code::RIGHT),
        (ks::KEY_End, code::END),
        (ks::KEY_KP_End, code::END),
        (ks::KEY_Up, code::UP),
        (ks::KEY_KP_Up, code::UP),
        (ks::KEY_Down, code::DOWN),
        (ks::KEY_KP_Down, code::DOWN),
        (ks::KEY_Page_Up, code::PAGE_UP),
        (ks::KEY_KP_Page_Up, code::PAGE_UP),
        (ks::KEY_Page_Down, code::PAGE_DOWN),
        (ks::KEY_KP_Page_Down, code::PAGE_DOWN),
        (ks::KEY_Delete, code::DELETE),
        // Keypad Delete with Num Lock off; with it on the key types ".".
        (ks::KEY_KP_Delete, code::DELETE),
    ];
    KEYS.iter().find(|(s, _)| *s == sym).map(|(_, key)| *key)
}

/// The same keys by position, for a keyboard without a keymap.
fn positional_special(raw: u32) -> Option<u32> {
    matches!(
        raw,
        code::ESC
            | code::BACKSPACE
            | code::TAB
            | code::ENTER
            | code::SPACE
            | code::F2
            | code::F11
            | code::HOME
            | code::LEFT
            | code::RIGHT
            | code::END
            | code::DELETE
    )
    .then_some(raw)
}

/// `sym` as a lowercase Latin letter, if it is one.
fn latin_letter(sym: u32) -> Option<char> {
    let c = char::from_u32(sym)?;
    c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase())
}

/// The evdev code a Latin letter has on a US keyboard.
fn letter_code(letter: char) -> Option<u32> {
    const ROWS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
    ROWS.iter().find_map(|(row, first)| {
        row.find(letter.to_ascii_lowercase())
            .map(|i| first + i as u32)
    })
}

/// True for the positions that hold letters on a US keyboard.
fn is_letter_position(raw: u32) -> bool {
    matches!(raw, 16..=25 | 30..=38 | 44..=50)
}

/// The shortcut key of a press. `latin` is the Latin letter the key carries
/// in the keymap (the active layout first, then any other).
fn logical_key(sym: u32, latin: Option<char>, ctrl: bool, raw: u32) -> u32 {
    if let Some(key) = special_key(sym) {
        return key;
    }
    // A letter of the active layout is that letter's key wherever it sits
    // (Ctrl+A on AZERTY is the key labelled A).
    if let Some(code) = latin_letter(sym).and_then(letter_code) {
        return code;
    }
    if ctrl {
        // A layout without Latin letters (Cyrillic, Greek): the chord goes
        // by the letter another layout of the keymap puts on the key, and
        // failing that by position. Only for chords, and only for letter
        // positions: a key that just types must never turn into Escape.
        if let Some(code) = latin.and_then(letter_code) {
            return code;
        }
        // By position only for a key that carries a letter of another
        // script. On Dvorak the positions of Q, W and Z type ' , and ; :
        // those are not second bindings for Ctrl+Q, Ctrl+W and Ctrl+Z.
        let own = char::from_u32(xkb::keysym_to_utf32(xkb::Keysym::new(sym)));
        let foreign_letter = own.is_some_and(|c| c.is_alphabetic() && !c.is_ascii());
        if foreign_letter && is_letter_position(raw) {
            return raw;
        }
    }
    code::NONE
}

#[cfg(test)]
mod tests;

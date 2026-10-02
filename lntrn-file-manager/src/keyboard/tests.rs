use super::*;

impl Keyboard {
    /// A keyboard with a keymap built from layout names ("us", "us,ru").
    /// `None` when the machine has no xkb data to build it from.
    fn with_layout(layout: &str) -> Option<Self> {
        let mut kb = Self::new();
        let keymap = xkb::Keymap::new_from_names(
            &kb.context,
            "",
            "",
            layout,
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        kb.set_keymap(keymap);
        Some(kb)
    }

    fn with_keymap_text(text: &str) -> Option<Self> {
        let mut kb = Self::new();
        let keymap = xkb::Keymap::new_from_string(
            &kb.context,
            text.to_string(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        kb.set_keymap(keymap);
        Some(kb)
    }

    /// The modifier mask of `names` in the current keymap.
    fn mask(&self, names: &[&str]) -> u32 {
        let keymap = &self.xkb.as_ref().unwrap().keymap;
        names
            .iter()
            .fold(0, |mask, name| mask | 1 << keymap.mod_get_index(*name))
    }
}

// evdev positions on a US keyboard.
const POS_ESC: u32 = 1;
const POS_1: u32 = 2;
const POS_Q: u32 = 16;
const POS_A: u32 = 30;
const POS_C: u32 = 46;
const POS_LSHIFT: u32 = 42;
const POS_ENTER: u32 = 28;
const POS_F5: u32 = 63;

/// The tests that read a real layout need the system's xkb data; a machine
/// without it skips them.
macro_rules! keyboard {
    ($layout:expr) => {
        match Keyboard::with_layout($layout) {
            Some(kb) => kb,
            None => {
                eprintln!("no xkb data for layout {:?}, test skipped", $layout);
                return;
            }
        }
    };
}

fn press(kb: &mut Keyboard, raw: u32) -> Option<KeyPress> {
    kb.on_key(raw, true, Instant::now());
    kb.on_key(raw, false, Instant::now());
    kb.pop()
}

fn mods(kb: &mut Keyboard, depressed: &[&str], locked: &[&str]) {
    let (d, l) = (kb.mask(depressed), kb.mask(locked));
    kb.on_modifiers(d, 0, l, 0);
}

#[test]
fn letters_follow_shift_and_caps_lock() {
    let mut kb = keyboard!("us");
    let plain = press(&mut kb, POS_A).unwrap();
    assert_eq!((plain.key, plain.ch), (POS_A, Some('a')));

    mods(&mut kb, &[xkb::MOD_NAME_SHIFT], &[]);
    let shifted = press(&mut kb, POS_A).unwrap();
    assert_eq!(shifted.ch, Some('A'));
    assert!(shifted.shift);

    // Caps Lock types capitals without counting as Shift.
    mods(&mut kb, &[], &[xkb::MOD_NAME_CAPS]);
    let caps = press(&mut kb, POS_A).unwrap();
    assert_eq!(caps.ch, Some('A'));
    assert!(!caps.shift);
    // ... and leaves the digits alone.
    assert_eq!(press(&mut kb, POS_1).unwrap().ch, Some('1'));
}

#[test]
fn a_ctrl_chord_names_its_key_and_types_nothing() {
    let mut kb = keyboard!("us");
    mods(&mut kb, &[xkb::MOD_NAME_CTRL], &[]);
    let chord = press(&mut kb, POS_C).unwrap();
    assert_eq!(chord.key, POS_C);
    assert_eq!(chord.ch, None);
    assert!(chord.ctrl);
    // Ctrl+1 is no shortcut and no text: not a press at all.
    assert_eq!(press(&mut kb, POS_1), None);
}

#[test]
fn editing_keys_are_recognised_and_type_nothing() {
    let mut kb = keyboard!("us");
    let enter = press(&mut kb, POS_ENTER).unwrap();
    assert_eq!((enter.key, enter.ch), (code::ENTER, None));
    let esc = press(&mut kb, POS_ESC).unwrap();
    assert_eq!((esc.key, esc.ch), (code::ESC, None));
    let space = press(&mut kb, code::SPACE).unwrap();
    assert_eq!((space.key, space.ch), (code::SPACE, Some(' ')));
    // Modifiers and unbound function keys are not presses.
    assert_eq!(press(&mut kb, POS_LSHIFT), None);
    assert_eq!(press(&mut kb, POS_F5), None);
}

#[test]
fn shortcuts_go_by_the_letter_not_the_position() {
    // AZERTY: the key in the US "Q" position types "a".
    let mut kb = keyboard!("fr");
    let typed = press(&mut kb, POS_Q).unwrap();
    assert_eq!(typed.ch, Some('a'));
    mods(&mut kb, &[xkb::MOD_NAME_CTRL], &[]);
    let chord = press(&mut kb, POS_Q).unwrap();
    assert_eq!(chord.key, POS_A, "Ctrl+A is the key labelled A");
}

#[test]
fn a_non_latin_layout_types_its_letters_and_keeps_the_chords() {
    let mut kb = keyboard!("ru");
    let typed = press(&mut kb, POS_C).unwrap();
    assert_eq!(typed.ch, Some('с')); // Cyrillic es
    assert_eq!(typed.key, code::NONE);
    mods(&mut kb, &[xkb::MOD_NAME_CTRL], &[]);
    let chord = press(&mut kb, POS_C).unwrap();
    assert_eq!(
        chord.key, POS_C,
        "Ctrl+C by position when no layout has a C"
    );
}

/// The keymap the compositor's text injection builds (text_inject.rs): one
/// key per character, the first on xkb keycode 9, which is evdev 1, the
/// position of Escape.
fn injected_keymap(text: &str) -> String {
    let mut codes = String::new();
    let mut syms = String::new();
    for (i, c) in text.chars().enumerate() {
        codes.push_str(&format!("    <K{i}> = {};\n", i + 9));
        syms.push_str(&format!("    key <K{i}> {{ [ U{:04X} ] }};\n", c as u32));
    }
    format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"(t)\" {{\n    minimum = 8;\n    maximum = {};\n{codes}}};\n\
         xkb_types \"(t)\" {{ include \"complete\" }};\n\
         xkb_compatibility \"(t)\" {{ include \"complete\" }};\n\
         xkb_symbols \"(t)\" {{\n{syms}}};\n}};\n",
        text.chars().count() + 9
    )
}

#[test]
fn injected_text_is_read_as_text_not_as_escape_and_digits() {
    let text = "\u{1F44D}\u{2764}\u{FE0F}";
    let Some(mut kb) = Keyboard::with_keymap_text(&injected_keymap(text)) else {
        eprintln!("no xkb data, test skipped");
        return;
    };
    // The whole batch arrives before anything is handled.
    for raw in 1..=3 {
        kb.on_key(raw, true, Instant::now());
        kb.on_key(raw, false, Instant::now());
    }
    let got: Vec<KeyPress> = std::iter::from_fn(|| kb.pop()).collect();
    assert_eq!(got.len(), 3);
    assert!(got.iter().all(|p| p.key == code::NONE));
    let typed: String = got.iter().filter_map(|p| p.ch).collect();
    assert_eq!(typed, text);
}

#[test]
fn presses_queue_in_order_with_the_modifiers_of_their_moment() {
    let mut kb = keyboard!("us");
    let now = Instant::now();
    // Shift+A, Shift released, then b: all dispatched in one batch.
    mods(&mut kb, &[xkb::MOD_NAME_SHIFT], &[]);
    kb.on_key(POS_A, true, now);
    kb.on_key(POS_A, false, now);
    mods(&mut kb, &[], &[]);
    kb.on_key(48, true, now);
    kb.on_key(POS_C, true, now);
    let typed: String = std::iter::from_fn(|| kb.pop())
        .filter_map(|p| p.ch)
        .collect();
    assert_eq!(typed, "Abc");
}

#[test]
fn a_held_key_repeats_after_the_delay_and_a_modifier_does_not_take_over() {
    let mut kb = keyboard!("us");
    let t0 = Instant::now();
    kb.on_key(POS_A, true, t0);
    kb.on_key(POS_LSHIFT, true, t0);
    assert_eq!(kb.repeat_at(), Some(t0 + REPEAT_DELAY));
    assert_eq!(kb.take_repeat(t0 + Duration::from_millis(100)), None);

    let first = kb.take_repeat(t0 + REPEAT_DELAY).unwrap();
    assert_eq!(first.ch, Some('a'));
    // From then on at the repeat rate.
    let t1 = t0 + REPEAT_DELAY + REPEAT_INTERVAL;
    assert_eq!(kb.repeat_at(), Some(t1));
    assert!(kb.take_repeat(t1).is_some());
    assert_eq!(kb.repeat_at(), Some(t1 + REPEAT_INTERVAL));

    kb.on_key(POS_A, false, t1);
    assert_eq!(kb.repeat_at(), None);
    assert_eq!(kb.take_repeat(t1 + Duration::from_secs(5)), None);
}

#[test]
fn keys_that_commit_or_dismiss_never_repeat() {
    let mut kb = keyboard!("us");
    let t0 = Instant::now();
    kb.on_key(POS_A, true, t0);
    kb.on_key(POS_ENTER, true, t0);
    assert_eq!(
        kb.repeat_at(),
        None,
        "Enter also ends the repeat of a held letter"
    );
}

#[test]
fn losing_focus_drops_the_held_key_but_not_what_was_typed() {
    let mut kb = keyboard!("us");
    kb.on_key(POS_A, true, Instant::now());
    kb.on_leave();
    assert_eq!(kb.repeat_at(), None);
    assert_eq!(kb.pop().and_then(|p| p.ch), Some('a'));

    // Ctrl was down when the focus went (Ctrl+Alt+something): it is not
    // down for the click that brings the window back.
    mods(&mut kb, &[xkb::MOD_NAME_CTRL], &[xkb::MOD_NAME_CAPS]);
    assert!(kb.ctrl());
    kb.on_leave();
    assert!(!kb.ctrl());
    assert_eq!(
        press(&mut kb, POS_A).unwrap().ch,
        Some('A'),
        "Caps Lock stays"
    );
}

#[test]
fn without_a_keymap_only_the_editing_keys_work() {
    let mut kb = Keyboard::new();
    assert_eq!(press(&mut kb, POS_A), None);
    assert_eq!(press(&mut kb, POS_ESC).map(|p| p.key), Some(code::ESC));
    kb.on_modifiers(4, 0, 0, 0);
    assert!(kb.ctrl());
}

#[test]
fn the_shortcut_key_of_a_press() {
    use xkb::keysyms::*;
    assert_eq!(logical_key(KEY_Escape, None, false, 9), code::ESC);
    assert_eq!(logical_key(KEY_KP_Enter, None, false, 96), code::ENTER);
    assert_eq!(logical_key(KEY_ISO_Left_Tab, None, false, 15), code::TAB);
    assert_eq!(logical_key('Z' as u32, Some('z'), true, 44), 44);
    // A key that only types never becomes a command, whatever its position.
    assert_eq!(
        logical_key(0x0100_0000 + 0x1F44D, None, false, 1),
        code::NONE
    );
    assert_eq!(
        logical_key(0x0100_0000 + 0x1F44D, None, true, 1),
        code::NONE
    );
    assert_eq!(letter_code('q'), Some(16));
    assert_eq!(letter_code('m'), Some(50));
    assert_eq!(letter_code('1'), None);
}

#[test]
fn caps_lock_survives_a_keymap_change() {
    let mut kb = keyboard!("us");
    mods(&mut kb, &[], &[xkb::MOD_NAME_CAPS]);
    // The compositor swaps keymaps (text injection and back) without
    // sending the modifiers again.
    let keymap = xkb::Keymap::new_from_names(
        &kb.context,
        "",
        "",
        "us",
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .unwrap();
    kb.set_keymap(keymap);
    assert_eq!(press(&mut kb, POS_A).unwrap().ch, Some('A'));
}

#[test]
fn the_keymap_is_read_from_the_fd_the_compositor_sends() {
    use std::io::Write;
    use std::os::fd::FromRawFd;
    let source = keyboard!("us");
    let mut text = source
        .xkb
        .as_ref()
        .unwrap()
        .keymap
        .get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
        .into_bytes();
    // As on the wire: NUL-terminated, the size counting the NUL.
    text.push(0);
    let fd = unsafe { libc::memfd_create(c"fox-keymap-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(&text).unwrap();

    let mut kb = Keyboard::new();
    kb.on_keymap(true, file.into(), text.len() as u32);
    assert_eq!(press(&mut kb, POS_A).and_then(|p| p.ch), Some('a'));
    // The same keymap by the second way in (read instead of mapped).
    let fd = unsafe { libc::memfd_create(c"fox-keymap-test".as_ptr(), libc::MFD_CLOEXEC) };
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(&text).unwrap();
    assert!(kb.keymap_by_reading(file.into(), text.len()).is_some());

    // A format nobody can read leaves the keyboard without a keymap
    // rather than with the old one.
    let fd = unsafe { libc::memfd_create(c"fox-keymap-test".as_ptr(), libc::MFD_CLOEXEC) };
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    kb.on_keymap(false, file.into(), 16);
    assert_eq!(press(&mut kb, POS_A), None);
}

/// After a dead key the next press is still a command when it is one: a
/// chord, or a key like Enter, Esc, Delete. (Dropped, Ctrl+X after a stray
/// "^" left the clipboard holding what was cut before.)
#[test]
fn a_dead_key_does_not_swallow_the_command_that_follows() {
    use xkb::keysyms as ks;
    let mut kb = keyboard!("de");
    // Find the key that is a dead key at level 0.
    let dead = (1..120u32).find(|raw| {
        let x = kb.xkb.as_ref().unwrap();
        let sym = x.state.key_get_one_sym(xkb::Keycode::new(raw + 8)).raw();
        (ks::KEY_dead_grave..=ks::KEY_dead_longsolidusoverlay).contains(&sym)
    });
    let Some(dead) = dead else {
        eprintln!("the 'de' layout here has no dead key: skipped");
        return;
    };
    // The dead key alone types nothing yet.
    assert_eq!(press(&mut kb, dead), None);
    // Ctrl+X right after it is Ctrl+X.
    mods(&mut kb, &[xkb::MOD_NAME_CTRL], &[]);
    let chord = press(&mut kb, 45).expect("the chord was swallowed");
    assert_eq!((chord.key, chord.ctrl, chord.ch), (45, true, None));
    mods(&mut kb, &[], &[]);
    // And the sequence did not survive the chord: a letter is a letter.
    assert_eq!(press(&mut kb, 30).unwrap().ch, Some('a'));

    // A command key ends a waiting sequence and still does what it says.
    assert_eq!(press(&mut kb, dead), None);
    let enter = press(&mut kb, 28).expect("Enter after a dead key was swallowed");
    assert_eq!(enter.key, code::ENTER);
    // A dead key followed by its letter still composes.
    assert_eq!(press(&mut kb, dead), None);
    let composed = press(&mut kb, 30).expect("the sequence did not compose");
    assert!(composed.ch.is_some_and(|c| !c.is_ascii()), "{composed:?}");
}

/// On Dvorak the US positions of Q, W, E and Z type punctuation. Those keys
/// are not second, unlabelled bindings for the letter shortcuts.
#[test]
fn punctuation_at_a_letter_position_is_not_a_letter_shortcut() {
    let mut kb = keyboard!("us(dvorak)");
    mods(&mut kb, &[xkb::MOD_NAME_CTRL], &[]);
    // The key at the US Z position types ";": not Undo.
    assert_eq!(press(&mut kb, 44), None);
    // The key at the US W position types ",": does not close the tab.
    assert_eq!(press(&mut kb, 17), None);
    // The key labelled W (US "," position) is Ctrl+W.
    assert_eq!(press(&mut kb, 51).map(|p| p.key), Some(17));
}

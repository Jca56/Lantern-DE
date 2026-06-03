//! The "Lantern layer" — a momentary, hold-to-activate key layer that turns
//! ordinary keys into navigation keys while a trigger key is held. Built for
//! 60% keyboards (like the Razer Huntsman Mini) whose firmware `fn` key never
//! reaches the OS: instead of `fn`, we use the **Menu** key (right of `fn`,
//! reachable with the right hand) as the trigger, and remap WASD etc. under
//! the left hand to arrows / navigation.
//!
//! Injection works at the **keycode** level, not keysym: while the layer is
//! held we intercept the source key and forward the *target's* evdev keycode
//! to the focused client (via `KeyboardHandle::input_forward`). The client
//! re-derives the keysym from its own keymap, so this is layout-agnostic for
//! the standard navigation keys (arrows/Home/End/PageUp/Down/Delete/Backspace
//! all have fixed evdev codes present in every keymap).
//!
//! Config lives in `[keybinds]` (shared) with optional per-machine overrides
//! in `[keybinds.machine.<hostname>]`, so the desktop can enable the layer
//! without affecting the laptop. See `crate::read_keybind`.

use smithay::input::keyboard::keysyms as xkb;

/// Smithay/xkb keycodes are evdev codes shifted by 8 (the historic X11
/// offset). `event.key_code()` already returns evdev+8 (see the libinput
/// backend), so our injection table is authored in the same space.
const EVDEV_OFFSET: u32 = 8;

// Standard evdev key codes for the navigation targets. These are fixed in
// `linux/input-event-codes.h` and present in every xkb keymap, so injecting
// them is robust regardless of the physical keyboard's layout.
const KEY_BACKSPACE: u32 = 14;
const KEY_HOME: u32 = 102;
const KEY_UP: u32 = 103;
const KEY_PAGEUP: u32 = 104;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_END: u32 = 107;
const KEY_DOWN: u32 = 108;
const KEY_PAGEDOWN: u32 = 109;
const KEY_DELETE: u32 = 111;

/// One remap entry: while the layer is held, pressing/releasing the key whose
/// keysym is `from_sym` instead forwards the keycode `to_keycode` (evdev+8).
/// Modifier requirement bits for a layer map. A map only fires when every
/// required modifier is held (in addition to the trigger key). Bare maps
/// (`mods == 0`) require no modifiers and act as the fallback.
pub mod modbits {
    pub const SHIFT: u8 = 1 << 0;
    pub const CTRL: u8 = 1 << 1;
    pub const ALT: u8 = 1 << 2;
    pub const SUPER: u8 = 1 << 3;
}

#[derive(Clone, Copy, Debug)]
pub struct LayerMap {
    pub from_sym: u32,
    pub to_keycode: u32,
    /// Required modifiers (bitset of `modbits`). 0 = none required.
    pub mods: u8,
}

/// Resolved layer configuration for this machine.
#[derive(Clone, Debug)]
pub struct LanternLayer {
    pub enabled: bool,
    /// Keysym of the trigger key that activates the layer while held.
    pub trigger_sym: u32,
    pub maps: Vec<LayerMap>,
}

impl Default for LanternLayer {
    fn default() -> Self {
        Self { enabled: false, trigger_sym: xkb::KEY_Menu, maps: Vec::new() }
    }
}

impl LanternLayer {
    /// Build the layer from `lantern.toml`, honoring per-machine overrides.
    ///
    /// Keys read from `[keybinds]` (or `[keybinds.machine.<host>]`):
    ///   - `layer_enabled` : "true" / "false"  (default false)
    ///   - `layer_key`     : trigger keysym name, default "Menu"
    ///   - `layer_maps`    : TOML array of "from=To" pairs, where `from` is a
    ///                       single character (the source key) and `To` is a
    ///                       target name (Up/Down/Left/Right/Home/End/PageUp/
    ///                       PageDown/Delete/Backspace). Default: WASD→arrows,
    ///                       q/e→Home/End, r/f→PageUp/PageDown, x→Delete,
    ///                       z→Backspace.
    pub fn load() -> Self {
        let enabled = crate::read_keybind("layer_enabled", "false") == "true";
        let key_name = crate::read_keybind("layer_key", "Menu");
        let trigger_sym = trigger_sym_from_name(&key_name);

        let raw = read_keybind_list("layer_maps");
        let maps: Vec<LayerMap> = if raw.is_empty() {
            default_maps()
        } else {
            raw.iter().filter_map(|pair| parse_map(pair)).collect()
        };

        Self { enabled, trigger_sym, maps }
    }

    /// Resolve a source keysym + currently-held modifiers to a target keycode.
    /// Among maps for `from_sym` whose required modifiers are all held, the
    /// most-specific (most modifiers) wins, so `shift+w` beats bare `w` when
    /// Shift is down, but bare `w` still fires when no modifiers are held.
    pub fn lookup(&self, from_sym: u32, held: u8) -> Option<u32> {
        self.maps
            .iter()
            .filter(|m| m.from_sym == from_sym && (m.mods & held) == m.mods)
            .max_by_key(|m| m.mods.count_ones())
            .map(|m| m.to_keycode)
    }
}

/// Build the held-modifier bitset from Smithay's modifier state.
pub fn held_mods(m: &smithay::input::keyboard::ModifiersState) -> u8 {
    let mut bits = 0u8;
    if m.shift { bits |= modbits::SHIFT; }
    if m.ctrl  { bits |= modbits::CTRL; }
    if m.alt   { bits |= modbits::ALT; }
    if m.logo  { bits |= modbits::SUPER; }
    bits
}

/// The bundled-default layer map: WASD→arrows plus the navigation extras the
/// user picked (Q/E→Home/End, R/F→PageUp/PageDown, X→Delete, Z→Backspace).
fn default_maps() -> Vec<LayerMap> {
    [
        ('w', KEY_UP),
        ('a', KEY_LEFT),
        ('s', KEY_DOWN),
        ('d', KEY_RIGHT),
        ('q', KEY_HOME),
        ('e', KEY_END),
        ('r', KEY_PAGEUP),
        ('f', KEY_PAGEDOWN),
        ('x', KEY_DELETE),
        ('z', KEY_BACKSPACE),
    ]
    .into_iter()
    .filter_map(|(ch, code)| {
        char_to_keysym(ch).map(|from_sym| LayerMap {
            from_sym,
            to_keycode: code + EVDEV_OFFSET,
            mods: 0,
        })
    })
    .collect()
}

/// Parse a config entry into a [`LayerMap`]. Accepts an optional modifier
/// prefix on the source: `"from=To"` (no mods) or `"mod+mod+from=To"`, e.g.
/// `"shift+w=Up"` or `"ctrl+shift+a=Home"`. Modifier names: shift/ctrl/alt/
/// super (super also: logo/meta/win).
fn parse_map(pair: &str) -> Option<LayerMap> {
    let (lhs, to) = pair.split_once('=')?;
    // Split the LHS on '+': all but the last token are modifiers, the last is
    // the source key. Guard the lone "+" key case by checking emptiness.
    let mut parts: Vec<&str> = lhs.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    let key_tok = parts.pop()?;
    let mut mods = 0u8;
    for m in parts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "shift" => modbits::SHIFT,
            "ctrl" | "control" => modbits::CTRL,
            "alt" => modbits::ALT,
            "super" | "logo" | "meta" | "win" => modbits::SUPER,
            _ => return None, // unknown modifier → drop the whole entry
        };
    }
    if key_tok.chars().count() != 1 {
        return None;
    }
    let from_sym = char_to_keysym(key_tok.chars().next()?)?;
    let to_keycode = target_keycode_from_name(to.trim())? + EVDEV_OFFSET;
    Some(LayerMap { from_sym, to_keycode, mods })
}

/// Map a single source character to its (lowercase) keysym. We match on the
/// *unshifted* keysym so the layer fires whether or not Shift happens to be
/// held — the source keys are letters, and `a..z` keysyms are contiguous.
fn char_to_keysym(ch: char) -> Option<u32> {
    let lower = ch.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        // xkb KEY_a is contiguous a..z; offset from 'a'.
        Some(xkb::KEY_a + (lower as u32 - 'a' as u32))
    } else {
        None
    }
}

/// Resolve a navigation target name to its evdev keycode.
fn target_keycode_from_name(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    let code = match n.as_str() {
        "up" => KEY_UP,
        "down" => KEY_DOWN,
        "left" => KEY_LEFT,
        "right" => KEY_RIGHT,
        "home" => KEY_HOME,
        "end" => KEY_END,
        "pageup" | "pgup" => KEY_PAGEUP,
        "pagedown" | "pgdn" | "pgdown" => KEY_PAGEDOWN,
        "delete" | "del" => KEY_DELETE,
        "backspace" | "bksp" => KEY_BACKSPACE,
        _ => return None,
    };
    Some(code)
}

/// Resolve a trigger-key name to its keysym. Defaults to Menu on anything
/// unrecognised so a typo can't silently disable the trigger entirely.
fn trigger_sym_from_name(name: &str) -> u32 {
    match name.to_ascii_lowercase().as_str() {
        "menu" | "compose" | "apps" | "application" => xkb::KEY_Menu,
        "super" | "super_l" | "meta" | "logo" => xkb::KEY_Super_L,
        "super_r" => xkb::KEY_Super_R,
        "caps" | "capslock" | "caps_lock" => xkb::KEY_Caps_Lock,
        "ralt" | "alt_r" | "altgr" | "iso_level3_shift" => xkb::KEY_ISO_Level3_Shift,
        _ => xkb::KEY_Menu,
    }
}

/// Read a keybind list value (`key = ["a", "b"]`) honoring per-machine
/// override. Mirrors `crate::read_keybind` but for array values: tries
/// `[keybinds.machine.<host>]` first, then `[keybinds]`.
fn read_keybind_list(key: &str) -> Vec<String> {
    let scoped = format!("keybinds.machine.{}", crate::machine_hostname());
    let v = crate::read_config_list(&scoped, key);
    if !v.is_empty() {
        return v;
    }
    crate::read_config_list("keybinds", key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(maps: &[&str]) -> LanternLayer {
        LanternLayer {
            enabled: true,
            trigger_sym: xkb::KEY_Menu,
            maps: maps.iter().filter_map(|s| parse_map(s)).collect(),
        }
    }

    #[test]
    fn bare_map_fires_with_no_mods() {
        let l = layer(&["w=Up"]);
        let w = char_to_keysym('w').unwrap();
        assert_eq!(l.lookup(w, 0), Some(KEY_UP + EVDEV_OFFSET));
    }

    #[test]
    fn bare_map_still_fires_when_unrelated_mod_held() {
        // Holding Shift shouldn't suppress a bare map — the layer is for nav.
        let l = layer(&["w=Up"]);
        let w = char_to_keysym('w').unwrap();
        assert_eq!(l.lookup(w, modbits::SHIFT), Some(KEY_UP + EVDEV_OFFSET));
    }

    #[test]
    fn modified_map_wins_when_its_mod_is_held() {
        // shift+w → Home should beat bare w → Up when Shift is down.
        let l = layer(&["w=Up", "shift+w=Home"]);
        let w = char_to_keysym('w').unwrap();
        assert_eq!(l.lookup(w, 0), Some(KEY_UP + EVDEV_OFFSET));
        assert_eq!(l.lookup(w, modbits::SHIFT), Some(KEY_HOME + EVDEV_OFFSET));
    }

    #[test]
    fn most_specific_match_wins() {
        let l = layer(&["w=Up", "shift+w=Home", "ctrl+shift+w=End"]);
        let w = char_to_keysym('w').unwrap();
        assert_eq!(
            l.lookup(w, modbits::SHIFT | modbits::CTRL),
            Some(KEY_END + EVDEV_OFFSET)
        );
    }

    #[test]
    fn unknown_modifier_drops_entry() {
        assert!(parse_map("hyper+w=Up").is_none());
    }

    #[test]
    fn parses_super_aliases() {
        let m = parse_map("super+a=Left").unwrap();
        assert_eq!(m.mods, modbits::SUPER);
        assert_eq!(parse_map("logo+a=Left").unwrap().mods, modbits::SUPER);
    }
}

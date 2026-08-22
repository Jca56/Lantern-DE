//! Synthetic text injection — types a string into the window that
//! currently holds (or last held) keyboard focus.
//!
//! Used by the Command Center's emoji picker: clicking an emoji asks the
//! compositor (over the cc-thumbs socket, see `cc_thumbs.rs`) to type the
//! glyph into the window the user was last working in. The CC is a layer
//! surface with Exclusive keyboard interactivity, so while it's open it
//! owns the seat keyboard — but `focused_window()` still points at the
//! toplevel underneath (CC grabs focus via the raw keyboard handle and
//! never touches `focused_surface`).
//!
//! ## How "type the actual glyph" works (the wtype trick)
//!
//! There's no key on a real keyboard for 😀. So we build a throwaway xkb
//! keymap whose keys map 1:1 to the Unicode scalar values of the string,
//! hand the target window keyboard focus, swap the seat to that keymap,
//! tap each synthetic key, swap the keymap back, then hand focus back to
//! the CC — all synchronously inside one event-loop turn, so the user
//! never sees the focus flicker. The target decodes each keycode to its
//! Unicode keysym and inserts the character, exactly as if typed. Works
//! in terminals and GUI text fields alike (unlike a paste, which needs
//! Ctrl+V vs Ctrl+Shift+V per app, or text-input-v3, which terminals
//! don't speak).
//!
//! Multi-scalar emoji (ZWJ sequences, flags, skin-tone modifiers) are
//! just several scalars typed in order — the client's text shaper
//! recombines them into the single grapheme.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keycode, XkbConfig};
use smithay::utils::SERIAL_COUNTER;

use crate::keyboard_focus::KeyboardFocusTarget;
use crate::state::Lantern;
use crate::window_ext::WindowExt;

/// First xkb keycode we hand out. xkb keycodes are evdev codes + 8 and the
/// keymap's `minimum` is 8, so the lowest usable code is 9. The i-th scalar
/// gets keycode `i + FIRST_KEYCODE`.
const FIRST_KEYCODE: u32 = 9;

impl Lantern {
    /// Type `text` into the currently-focused toplevel window. No-op if
    /// there's no window to type into or the string is empty.
    pub fn type_text_into_focused_window(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let Some(window) = self.focused_window() else {
            tracing::debug!("type_text: no focused window to type into");
            return;
        };
        // Build a keyboard-focus target for the window (X11-aware so native
        // X11 clients get a real SetInputFocus, mirroring set_focus_surface).
        let target = match window.x11_surface() {
            Some(x11) => KeyboardFocusTarget::X11(x11.clone()),
            None => match window.get_wl_surface() {
                Some(wls) => KeyboardFocusTarget::Wayland(wls),
                None => return,
            },
        };

        let Some((keymap, keycodes)) = build_unicode_keymap(text) else {
            return;
        };

        let keyboard = self.seat.get_keyboard().unwrap();
        // Whoever owns the keyboard right now (the CC layer surface). We
        // hand focus back to it once we're done so the picker keeps working.
        let saved_focus = keyboard.current_focus();

        // 1. Focus the target so it receives the upcoming keymap + keys.
        keyboard.set_focus(self, Some(target), SERIAL_COUNTER.next_serial());

        // 2. Swap the seat to the throwaway emoji keymap. This broadcasts the
        //    new keymap to the now-focused target so it decodes our keycodes.
        if keyboard.set_keymap_from_string(self, keymap).is_err() {
            tracing::warn!("type_text: failed to compile emoji keymap; aborting");
            keyboard.set_focus(self, saved_focus, SERIAL_COUNTER.next_serial());
            return;
        }

        // 3. Tap each synthetic key (press + immediate release, paired so no
        //    key is left "down" and autorepeat never kicks in).
        let time = self.start_time.elapsed().as_millis() as u32;
        for kc in keycodes {
            let code = Keycode::new(kc);
            keyboard.input_forward(
                self,
                code,
                KeyState::Pressed,
                SERIAL_COUNTER.next_serial(),
                time,
                false,
            );
            keyboard.input_forward(
                self,
                code,
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                time,
                false,
            );
        }

        // 4. Restore the real keymap (still broadcasting to the focused
        //    target) before we let focus go. Default config recompiles the
        //    exact keymap we booted with — the compositor uses no custom
        //    layout, so this round-trips losslessly.
        if let Err(err) = keyboard.set_xkb_config(self, XkbConfig::default()) {
            tracing::error!(?err, "type_text: failed to restore keymap");
        }

        // 5. Hand focus back to the Command Center.
        keyboard.set_focus(self, saved_focus, SERIAL_COUNTER.next_serial());
    }
}

/// Build a throwaway xkb keymap (XKB_KEYMAP_FORMAT_TEXT_V1) binding one key
/// per Unicode scalar in `text`, plus the list of xkb keycodes to tap in
/// order. Returns `None` if `text` has no scalars.
fn build_unicode_keymap(text: &str) -> Option<(String, Vec<u32>)> {
    let scalars: Vec<u32> = text.chars().map(|c| c as u32).collect();
    if scalars.is_empty() {
        return None;
    }

    let mut keycodes = Vec::with_capacity(scalars.len());
    let mut keycode_decls = String::new();
    let mut symbol_decls = String::new();
    for (i, cp) in scalars.iter().enumerate() {
        let kc = i as u32 + FIRST_KEYCODE;
        keycodes.push(kc);
        keycode_decls.push_str(&format!("    <K{i}> = {kc};\n"));
        // xkb reads `U<hex>` as the Unicode keysym for that scalar value.
        symbol_decls.push_str(&format!("    key <K{i}> {{ [ U{cp:04X} ] }};\n"));
    }
    let max = scalars.len() as u32 + FIRST_KEYCODE;

    let keymap = format!(
        "xkb_keymap {{
xkb_keycodes \"(lantern_emoji)\" {{
    minimum = 8;
    maximum = {max};
{keycode_decls}}};
xkb_types \"(lantern_emoji)\" {{ include \"complete\" }};
xkb_compatibility \"(lantern_emoji)\" {{ include \"complete\" }};
xkb_symbols \"(lantern_emoji)\" {{
{symbol_decls}}};
}};
"
    );
    Some((keymap, keycodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_one_key_per_scalar() {
        // 👍 is a single scalar (U+1F44D).
        let (km, codes) = build_unicode_keymap("👍").unwrap();
        assert_eq!(codes, vec![9]);
        assert!(km.contains("U1F44D"));
        assert!(km.contains("<K0> = 9;"));
    }

    #[test]
    fn keymap_multi_scalar_grapheme() {
        // Family emoji = 4 people joined by ZWJ (U+200D) → 7 scalars.
        let (km, codes) = build_unicode_keymap("👨‍👩‍👧‍👦").unwrap();
        assert_eq!(codes.len(), 7);
        assert_eq!(codes, vec![9, 10, 11, 12, 13, 14, 15]);
        assert!(km.contains("U200D")); // the ZWJ joiners
        assert!(km.contains("maximum = 16;"));
    }

    #[test]
    fn empty_is_none() {
        assert!(build_unicode_keymap("").is_none());
    }
}

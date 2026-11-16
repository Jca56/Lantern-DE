//! Keys and keyboard focus: no window is left believing a key is held.
//!
//! ── A window that loses the keyboard gets its keys released first ──
//! Every key a window was told went down is released to it before the
//! keyboard goes elsewhere. Wayland clients read `wl_keyboard.leave` as "all
//! keys up", but an X11 window never hears of it: XWayland drops the keys
//! from its own state on leave and sends the window nothing. A Proton game
//! stays live while another monitor has the keyboard, so there the key
//! stayed held. Elden Ring walks while Left Alt is down: after Alt+Tab the
//! character walked instead of ran.
//!
//! ── A key that went down in a window comes up in it ──
//! A binding may act on a key's release (Alt up commits the window
//! switcher) but must not swallow it when the press was forwarded. Smithay
//! keeps every forwarded press as "held" until its release is forwarded too,
//! and hands that list to each window that gets the keyboard next: Alt,
//! eaten on release, was reported as held to every window focused after an
//! Alt+Tab.
//!
//! `forwarded_keys` mirrors Smithay's own list, which it keeps private.

use smithay::{
    backend::input::KeyState,
    input::keyboard::Keycode,
    utils::{Serial, SERIAL_COUNTER},
};

use crate::keyboard_focus::KeyboardFocusTarget;
use crate::state::Lantern;

impl Lantern {
    // ── A key that went down in a window comes up in it ──

    /// Record what the keyboard filter did with a key event. A release it
    /// swallowed still goes to the window if that window saw the press.
    pub(super) fn note_key(
        &mut self,
        keycode: Keycode,
        state: KeyState,
        intercepted: bool,
        serial: Serial,
        time: u32,
    ) {
        match (state, intercepted) {
            (_, false) => self.note_forwarded_key(keycode, state),
            (KeyState::Released, true) => {
                // Still listed: the focus didn't change under the binding
                // (a focus change releases every listed key itself).
                if self.forwarded_keys.remove(&keycode) {
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.input_forward(self, keycode, state, serial, time, true);
                    }
                }
            }
            (KeyState::Pressed, true) => {}
        }
    }

    /// A key event went to the focused window (a real one the filter let
    /// through, or one the Lantern layer injected).
    pub(super) fn note_forwarded_key(&mut self, keycode: Keycode, state: KeyState) {
        match state {
            KeyState::Pressed => {
                self.forwarded_keys.insert(keycode);
            }
            KeyState::Released => {
                self.forwarded_keys.remove(&keycode);
            }
        }
    }

    // ── A window that loses the keyboard gets its keys released first ──

    /// Give the keyboard to `target` (or to nobody). Every keyboard focus
    /// change goes through here, so the window losing it has its keys
    /// released. Never call it with the keyboard locked (inside
    /// `SeatHandler::focus_changed` or a `KeyboardTarget` callback).
    pub(crate) fn set_keyboard_focus(
        &mut self,
        target: Option<KeyboardFocusTarget>,
        serial: Serial,
    ) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        self.release_keys_before_focus_change(target.as_ref());
        keyboard.set_focus(self, target, serial);
    }

    /// Release the focused window's held keys if the keyboard is about to
    /// leave it for `target`. `set_keyboard_focus` does this itself; a caller
    /// that sends X11 requests before it (raising the window, its activated
    /// state) calls this ahead of them.
    ///
    /// The order matters for an X11 window: its key releases travel to
    /// XWayland as Wayland events, the loss of X input focus as X11 requests
    /// on another socket, and an X11 window without the input focus gets no
    /// key events. So the releases are flushed out before the first X11
    /// request of the focus change is written: XWayland handles input it has
    /// already received before it reads requests.
    pub(crate) fn release_keys_before_focus_change(&mut self, target: Option<&KeyboardFocusTarget>) {
        if self.forwarded_keys.is_empty() {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let current = keyboard.current_focus();
        if current.as_ref() == target {
            return;
        }
        // libinput's clock, as real key events carry.
        let time = self.clock.now().as_millis();
        let held: Vec<Keycode> = self.forwarded_keys.drain().collect();
        let from_x11 = matches!(current, Some(KeyboardFocusTarget::X11(_)));
        // Rare (a key is down across a focus change), and the one line that
        // says whether a "stuck key" report got this far.
        tracing::info!(keys = held.len(), from_x11, "released held keys before a focus change");
        for keycode in held {
            let serial = SERIAL_COUNTER.next_serial();
            keyboard.input_forward(self, keycode, KeyState::Released, serial, time, true);
        }
        if from_x11 {
            if let Err(err) = self.display_handle.flush_clients() {
                tracing::warn!("flushing key releases before a focus change failed: {err}");
            }
        }
    }
}

//! Unified keyboard focus target for Wayland and X11 surfaces.
//!
//! Smithay's `keyboard.set_focus` accepts `<D::SeatHandler as KeyboardFocus>`.
//! If we used bare `WlSurface` there, X11 surfaces would only get a
//! `wl_keyboard.enter` event — *not* the actual `XSetInputFocus` call that
//! native X11 games (SDL2, XInput2, XKBlib) need to receive key events.
//!
//! `X11Surface` implements `KeyboardTarget` with both behaviours; this
//! enum lets us pick the right variant per surface so the X11 path also
//! runs the X11 `set_input_focus` call.

use std::borrow::Cow;

use smithay::input::keyboard::{KeyboardTarget, KeysymHandle, ModifiersState};
use smithay::input::Seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Serial};
use smithay::wayland::seat::WaylandFocus;
use smithay::xwayland::X11Surface;

use crate::state::Lantern;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocusTarget {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl From<WlSurface> for KeyboardFocusTarget {
    fn from(s: WlSurface) -> Self {
        Self::Wayland(s)
    }
}

impl From<X11Surface> for KeyboardFocusTarget {
    fn from(s: X11Surface) -> Self {
        Self::X11(s)
    }
}

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(s) => s.alive(),
            Self::X11(s) => s.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(s) => Some(Cow::Borrowed(s)),
            Self::X11(s) => s.wl_surface().map(Cow::Owned),
        }
    }
}

impl KeyboardTarget<Lantern> for KeyboardFocusTarget {
    fn enter(
        &self,
        seat: &Seat<Lantern>,
        data: &mut Lantern,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            Self::X11(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<Lantern>, data: &mut Lantern, serial: Serial) {
        match self {
            Self::Wayland(s) => KeyboardTarget::leave(s, seat, data, serial),
            Self::X11(s) => KeyboardTarget::leave(s, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<Lantern>,
        data: &mut Lantern,
        key: KeysymHandle<'_>,
        state: smithay::backend::input::KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            Self::Wayland(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
            Self::X11(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<Lantern>,
        data: &mut Lantern,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
            Self::X11(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
        }
    }
}

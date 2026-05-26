//! ext-session-lock-v1 server handler.
//!
//! When a trusted client (the lockscreen) requests a lock, we clear the
//! screen, send each output's lock surface a configure with the output size,
//! route keyboard focus to the lock surface, and refuse pointer/keybind input
//! to everything else (see `input::dispatch` / `input::keyboard`). The actual
//! `confirmation.lock()` call — which tells the client the session is sealed —
//! happens in the render path once every output has presented a cleared frame
//! (see `render::surface`).

use std::collections::{HashMap, HashSet};

use smithay::delegate_session_lock;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use crate::keyboard_focus::KeyboardFocusTarget;
use crate::state::Lantern;

/// State held while the session is locked.
pub struct SessionLockData {
    /// Per-output lock surfaces, keyed by output name (Output's PartialEq uses
    /// `Arc::ptr_eq` which is unreliable across code paths — names are stable).
    pub surfaces: HashMap<String, LockSurface>,
    /// The confirmation handle. Taken + `.lock()`ed once all outputs have
    /// presented a cleared frame. `None` after the lock is confirmed.
    pub pending_locker: Option<SessionLocker>,
    /// Output names that have rendered a locked frame at least once.
    pub presented: HashSet<String>,
    /// Keyboard focus to restore on unlock.
    pub prev_focus: Option<KeyboardFocusTarget>,
}

impl SessionLockHandler for Lantern {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        let prev_focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        self.session_lock = Some(SessionLockData {
            surfaces: HashMap::new(),
            pending_locker: Some(confirmation),
            presented: HashSet::new(),
            prev_focus,
        });
        // Clear the screen on every output immediately.
        self.schedule_render_forced();
    }

    fn unlock(&mut self) {
        let prev_focus = self.session_lock.take().and_then(|d| d.prev_focus);
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, prev_focus, serial);
        }
        self.schedule_render_forced();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(out) = Output::from_resource(&output) else {
            return;
        };
        let Some(geo) = self.workspaces.output_geometry(&out) else {
            return;
        };
        surface.with_pending_state(|state| {
            state.size = Some((geo.size.w as u32, geo.size.h as u32).into());
        });
        surface.send_configure();

        // Hand keyboard focus to the lock surface so typed passwords reach it.
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(
                self,
                Some(KeyboardFocusTarget::Wayland(surface.wl_surface().clone())),
                serial,
            );
            tracing::info!(output = %out.name(), "session lock: keyboard focus → lock surface");
        }

        if let Some(data) = &mut self.session_lock {
            data.surfaces.insert(out.name(), surface);
        }
        self.schedule_render_forced();
    }
}

delegate_session_lock!(Lantern);

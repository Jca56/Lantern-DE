//! ext-session-lock-v1 server handler.
//!
//! When a trusted client (the lockscreen) requests a lock, we clear the
//! screen, send each output's lock surface a configure with the output size,
//! route keyboard focus to the lock surface, and refuse pointer/keybind input
//! to everything else (see `input::dispatch` / `input::keyboard`). The actual
//! `confirmation.lock()` call — which tells the client the session is sealed —
//! happens in the render path once every output has presented a cleared frame
//! (see `render::surface`).
//!
//! A locked session must never be left without a way to unlock it, so the
//! lock is also looked after once a second (`supervise_lock`): a locker that
//! died is relaunched, and a confirmation held up by an output that draws
//! nothing is sent after a grace period.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use smithay::delegate_session_lock;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use crate::keyboard_focus::KeyboardFocusTarget;
use crate::state::Lantern;

/// How long the lock confirmation waits for every output to draw a locked
/// frame. One that isn't drawing at all (parked, powered down) would hold it
/// up forever, and the locker can't unlock a session that was never confirmed.
const CONFIRM_GRACE: Duration = Duration::from_secs(2);

/// A locker that has lived this long is healthy: relaunching starts over
/// without a wait if it dies later.
const LOCKER_STABLE: Duration = Duration::from_secs(30);

/// Longest wait between relaunches of a locker that keeps dying.
const RESPAWN_MAX_DELAY: Duration = Duration::from_secs(30);

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
    /// The locker's own protocol object. Dead while the session is still
    /// locked means the locker is gone.
    lock_object: ExtSessionLockV1,
    /// When this locker asked for the lock.
    since: Instant,
    /// Wait before the next relaunch; doubles each time one is needed.
    respawn_delay: Duration,
    /// Earliest moment for the next relaunch.
    respawn_at: Instant,
}

impl SessionLockHandler for Lantern {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        let now = Instant::now();
        let lock_object = confirmation.ext_session_lock().clone();
        // A relaunched locker takes over a session that is already locked. It
        // inherits what to restore on unlock (focus is on a dead lock surface
        // by now) and how patient to be should it die as well.
        let (prev_focus, respawn_delay) = match self.session_lock.take() {
            Some(old) => (old.prev_focus, old.respawn_delay),
            None => (
                self.seat.get_keyboard().and_then(|k| k.current_focus()),
                Duration::ZERO,
            ),
        };
        self.session_lock = Some(SessionLockData {
            surfaces: HashMap::new(),
            pending_locker: Some(confirmation),
            presented: HashSet::new(),
            prev_focus,
            lock_object,
            since: now,
            respawn_delay,
            respawn_at: now,
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

impl Lantern {
    /// Called once a second. Keeps a locked session from getting stuck.
    pub fn supervise_lock(&mut self) {
        let Some(data) = self.session_lock.as_mut() else {
            return;
        };
        let now = Instant::now();

        if !data.lock_object.is_alive() {
            // Locked, and the one client that could unlock is gone (crashed or
            // killed): a blank screen with no way in. Start another locker.
            if now >= data.respawn_at {
                data.respawn_delay =
                    (data.respawn_delay * 2).clamp(Duration::from_secs(1), RESPAWN_MAX_DELAY);
                data.respawn_at = now + data.respawn_delay;
                tracing::warn!("session lock: the locker is gone while locked, relaunching it");
                crate::power::spawn_lockscreen();
            }
            return;
        }
        let age = now.duration_since(data.since);
        if age >= LOCKER_STABLE {
            data.respawn_delay = Duration::ZERO;
        }

        if age >= CONFIRM_GRACE {
            if let Some(locker) = data.pending_locker.take() {
                tracing::warn!(
                    "session lock: not every output drew a locked frame in time, confirming anyway"
                );
                locker.lock();
                self.sleep_lock.lock_confirmed();
            }
        }
    }

    /// While locked, keyboard focus belongs on a live lock surface and nowhere
    /// else. The surface that had it can go away under us (its monitor was
    /// unplugged, or the locker died and a new one is starting); the keys must
    /// then reach one that's left, and never a window behind the lock.
    pub fn ensure_lock_focus(&mut self) {
        let Some(data) = self.session_lock.as_ref() else {
            return;
        };
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let focus = keyboard.current_focus();
        let on_lock_surface = match &focus {
            Some(KeyboardFocusTarget::Wayland(focused)) => data
                .surfaces
                .values()
                .any(|ls| ls.alive() && ls.wl_surface() == focused),
            _ => false,
        };
        if on_lock_surface {
            return;
        }
        let target = data
            .surfaces
            .values()
            .find(|ls| ls.alive())
            .map(|ls| KeyboardFocusTarget::Wayland(ls.wl_surface().clone()));
        if target.is_none() && focus.is_none() {
            return;
        }
        keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
    }
}

delegate_session_lock!(Lantern);

//! Process-wide "is the panel on screen?" flag shared with the worker
//! threads.
//!
//! Every backend poller (audio via `wpctl`, BlueZ, iwd, MPRIS) used to
//! run its full cadence around the clock even though nothing renders
//! while the Command Center is hidden — measured at ~9% of a core in
//! spawned helper processes alone. The main loop publishes visibility
//! here once per iteration; workers read it to pick a slow "keep the
//! cache lukewarm" cadence while hidden and burst-poll the instant the
//! panel comes back so the first visible frame is fresh.
//!
//! Plain atomic, no channel plumbing: the workers already sleep-loop on
//! a short tick, so a relaxed load per iteration is the cheapest way to
//! get the signal across without touching every command enum.

use std::sync::atomic::{AtomicBool, Ordering};

static VISIBLE: AtomicBool = AtomicBool::new(false);

/// Publish the current visibility. Called by the main loop every
/// iteration; `visible` is true for Opening / Visible / Closing so the
/// tiles stay live through both animations.
pub fn set(visible: bool) {
    VISIBLE.store(visible, Ordering::Relaxed);
}

/// Current visibility as last published by the main loop.
pub fn get() -> bool {
    VISIBLE.load(Ordering::Relaxed)
}

/// Per-worker edge detector over [`get`]. Call `poll()` once per loop
/// iteration; `just_shown` is true on the single iteration where the
/// panel went hidden → visible, which is the cue to poll immediately
/// instead of waiting out the current interval.
pub struct VisGate {
    last: bool,
}

impl VisGate {
    pub fn new() -> Self {
        Self { last: get() }
    }

    /// Returns `(visible, just_shown)`.
    pub fn poll(&mut self) -> (bool, bool) {
        let now = get();
        let just_shown = now && !self.last;
        self.last = now;
        (now, just_shown)
    }
}

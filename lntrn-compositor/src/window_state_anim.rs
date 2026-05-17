//! Rect-based animations for window state changes (maximize, unmaximize,
//! fullscreen, unfullscreen, snap, unsnap).
//!
//! Each surface gets at most one in-flight rect animation. New triggers on the
//! same surface redirect the existing animation from its current interpolated
//! rect — so e.g. spamming maximize/unmaximize never visually snaps.

use std::collections::HashMap;
use std::time::Duration;

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
};

use crate::rect_anim::{Curve, RectAnim};

/// Duration + curve for window-state transitions. Both pull from the user's
/// `[animations]` settings (speed scale + active preset).
pub fn state_duration() -> Duration { crate::animations::state_duration() }
pub fn state_curve() -> Curve { crate::animations::state_curve() }

/// Per-surface state for the "smooth resize" (trusted-client) path.
///
/// Trusted clients get a configure on EVERY frame of the animation with
/// the current interpolated rect — so they redraw at every intermediate
/// size. The compositor renders each fresh buffer at scale ~1.0 (the
/// 1-frame client lag produces a barely-perceptible <2% stretch). When
/// the rect animation ends, the smooth-hold persists with `held_target`
/// only until the client commits a buffer at the matching size — that
/// bridges the 1-2 frame gap between "anim done" and "client redrew at
/// target". See `docs/native-smooth-resize-plan.md`.
pub struct SmoothHold {
    /// Final rect we're animating to. Used both as the destination for
    /// the per-frame configure stream and as the held-visual rect during
    /// the brief post-anim gap.
    pub target: Rectangle<i32, Logical>,
}

pub struct WindowStateAnimState {
    animations: HashMap<WlSurface, RectAnim>,
    smooth_holds: HashMap<WlSurface, SmoothHold>,
}

impl WindowStateAnimState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
            smooth_holds: HashMap::new(),
        }
    }

    /// Start (or redirect) a rect animation for a surface.
    /// If already animating, the current interpolated rect is used as the new
    /// start so there's no visual snap.
    ///
    /// Also drops any pending smooth-hold for this surface. Callers that
    /// want the smooth-resize path go through `animate_smooth`, which
    /// calls this and then re-inserts the hold. Any other animate caller
    /// (workspace slide-off, etc.) implicitly cancels a smooth resize
    /// already in progress, so we have to drop its deferred configure
    /// here — otherwise it would later fire spuriously.
    pub fn animate(
        &mut self,
        surface: &WlSurface,
        start: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
        duration: Duration,
        curve: Curve,
    ) {
        self.smooth_holds.remove(surface);
        if start == target {
            self.animations.remove(surface);
            return;
        }
        if let Some(existing) = self.animations.get_mut(surface) {
            existing.redirect(target, duration, curve);
            return;
        }
        self.animations.insert(
            surface.clone(),
            RectAnim::new(start, target, duration, curve),
        );
    }

    /// Convenience wrapper using the cinematic defaults.
    pub fn animate_default(
        &mut self,
        surface: &WlSurface,
        start: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
    ) {
        self.animate(surface, start, target, state_duration(), state_curve());
    }

    pub fn current_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.animations.get(surface).map(|a| a.current())
    }

    pub fn target_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.animations.get(surface).map(|a| a.target())
    }

    /// Eased animation progress, 0.0 → 1.0. Used by the resize-anim
    /// crossfade to mix the pre-anim snapshot with the live (post-resize)
    /// surface so content reflow (font scaling, etc.) settles smoothly
    /// instead of popping in at the end.
    pub fn eased_progress(&self, surface: &WlSurface) -> Option<f64> {
        self.animations.get(surface).map(|a| a.eased())
    }

    /// Drop finished rect animations. Smooth-holds are retired by
    /// `clear_held_scale_if_matched` when the client commits at the
    /// matching size, not by tick — the post-anim hold has to outlive
    /// the rect anim by 1-2 frames so the client has time to catch up.
    pub fn tick(&mut self) -> bool {
        self.animations.retain(|_, a| !a.is_finished());
        !self.animations.is_empty()
    }

    pub fn has_active(&self) -> bool {
        !self.animations.is_empty()
    }

    /// True if a smooth-hold entry exists (deferred configure pending, or
    /// held-visual awaiting client catch-up). Needed by the renderer to
    /// stay scheduled even when no RectAnim is in flight, so the held-scale
    /// branch keeps drawing the stretched buffer until the client commits.
    pub fn has_smooth_holds(&self) -> bool {
        !self.smooth_holds.is_empty()
    }

    pub fn remove(&mut self, surface: &WlSurface) {
        self.animations.remove(surface);
        self.smooth_holds.remove(surface);
    }

    // ── Smooth-resize path (trusted clients) ─────────────────────────────

    /// Start (or redirect) a smooth-resize animation. Standard rect anim
    /// PLUS a smooth-hold marker so the per-frame tick can stream
    /// configures to the client (one per frame, with the current
    /// interpolated rect). Trusted clients redraw at every intermediate
    /// size; the renderer just blits each fresh buffer at scale ~1.0.
    ///
    /// Mid-animation re-trigger: the target is overwritten with the new
    /// destination so the post-anim held visual + final configure both
    /// land at the correct rect.
    pub fn animate_smooth(
        &mut self,
        surface: &WlSurface,
        start: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
    ) {
        self.animate(surface, start, target, state_duration(), state_curve());
        self.smooth_holds.insert(surface.clone(), SmoothHold { target });
    }

    /// Drain the next batch of per-frame configures to send. For each
    /// active smooth surface: if its anim is still running, return the
    /// current interpolated rect; if the anim has finished, return the
    /// final target rect ONCE so the client lands exactly on target
    /// (the entry stays so `held_target_rect` keeps bridging the post-
    /// anim → client-commit gap; it's retired by
    /// `clear_held_scale_if_matched` when the matching commit lands).
    ///
    /// Returned as a Vec so the caller can release its borrow on
    /// `window_state_anim` before touching the window list.
    pub fn drain_per_frame_configures(
        &mut self,
    ) -> Vec<(WlSurface, Rectangle<i32, Logical>)> {
        let mut configures = Vec::new();
        for (surface, hold) in self.smooth_holds.iter() {
            let rect = match self.animations.get(surface) {
                Some(anim) if !anim.is_finished() => anim.current(),
                _ => hold.target,
            };
            configures.push((surface.clone(), rect));
        }
        configures
    }

    /// Called from the surface commit handler with the newly-committed
    /// buffer size. Once the client has redrawn at the smooth-hold's
    /// target size, drop the hold so combined_scale collapses to 1.0 and
    /// normal rendering resumes. Mismatched commits (client lagged at a
    /// stale size) are ignored.
    pub fn clear_held_scale_if_matched(
        &mut self,
        surface: &WlSurface,
        committed_size: smithay::utils::Size<i32, Logical>,
    ) {
        let drop = self.smooth_holds.get(surface)
            .map_or(false, |h| h.target.size == committed_size)
            && !self.animations.contains_key(surface);
        if drop {
            self.smooth_holds.remove(surface);
        }
    }

    /// Returns the smooth-hold's target rect — used by the renderer to
    /// pin `effective_size` to the final rect during the brief window
    /// between "rect anim finished" and "client committed at new size".
    pub fn held_target_rect(
        &self,
        surface: &WlSurface,
    ) -> Option<Rectangle<i32, Logical>> {
        Some(self.smooth_holds.get(surface)?.target)
    }

    /// True iff this surface is being driven by the smooth-resize path.
    /// Renderer uses this to skip the snapshot crossfade entirely — the
    /// per-frame configure stream produces crisp content directly.
    pub fn is_smooth(&self, surface: &WlSurface) -> bool {
        self.smooth_holds.contains_key(surface)
    }
}

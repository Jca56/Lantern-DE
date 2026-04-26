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

/// Cinematic ease-in-out used for window-state transitions.
pub const STATE_DURATION: Duration = Duration::from_millis(520);
pub const STATE_CURVE: Curve = Curve::EaseInOutQuint;

pub struct WindowStateAnimState {
    animations: HashMap<WlSurface, RectAnim>,
}

impl WindowStateAnimState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
        }
    }

    /// Start (or redirect) a rect animation for a surface.
    /// If already animating, the current interpolated rect is used as the new
    /// start so there's no visual snap.
    pub fn animate(
        &mut self,
        surface: &WlSurface,
        start: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
        duration: Duration,
        curve: Curve,
    ) {
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
        self.animate(surface, start, target, STATE_DURATION, STATE_CURVE);
    }

    pub fn current_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.animations.get(surface).map(|a| a.current())
    }

    pub fn target_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.animations.get(surface).map(|a| a.target())
    }

    /// Drop finished animations. Returns true if any are still active.
    pub fn tick(&mut self) -> bool {
        self.animations.retain(|_, a| !a.is_finished());
        !self.animations.is_empty()
    }

    pub fn has_active(&self) -> bool {
        !self.animations.is_empty()
    }

    pub fn remove(&mut self, surface: &WlSurface) {
        self.animations.remove(surface);
    }
}

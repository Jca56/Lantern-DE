/// Tiling layout animation: per-window rect interpolation.
///
/// When the tiling layout changes (insert, remove, resize), each window
/// smoothly animates from its current position/size to the new target.
/// Uses spring physics for a natural bouncy feel.

use std::{collections::HashMap, time::Duration};

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
};

use crate::rect_anim::{Curve, RectAnim};

/// Cinematic spring duration for tiling reflows.
const ANIM_DURATION: Duration = Duration::from_millis(520);
/// Spring damping (lower = more bouncy, 1.0 = critically damped).
/// Bumped up from 0.7 → 0.82 so the longer cinematic duration doesn't wobble.
const SPRING_DAMPING: f64 = 0.82;
/// Spring oscillation frequency. Lowered from 5.0 → 4.0 to match cinematic feel.
const SPRING_FREQUENCY: f64 = 4.0;

const TILING_CURVE: Curve = Curve::Spring {
    damping: SPRING_DAMPING,
    frequency: SPRING_FREQUENCY,
};

pub struct TilingAnimationState {
    animations: HashMap<WlSurface, RectAnim>,
}

impl TilingAnimationState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
        }
    }

    /// Start or update an animation for a surface.
    /// If already animating, uses the current interpolated position as the new start.
    pub fn animate_to(
        &mut self,
        surface: &WlSurface,
        current_rect: Rectangle<i32, Logical>,
        target_rect: Rectangle<i32, Logical>,
    ) {
        // If already animating, redirect from current interpolated rect.
        if let Some(existing) = self.animations.get_mut(surface) {
            if existing.target() == target_rect {
                return;
            }
            existing.redirect(target_rect, ANIM_DURATION, TILING_CURVE);
            return;
        }

        // Don't animate if start == target (no visible change).
        if current_rect == target_rect {
            self.animations.remove(surface);
            return;
        }

        self.animations.insert(
            surface.clone(),
            RectAnim::new(current_rect, target_rect, ANIM_DURATION, TILING_CURVE),
        );
    }

    /// Get the current interpolated rect for a surface, if animating.
    pub fn current_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.animations.get(surface).map(|a| a.current())
    }

    /// Tick: remove finished animations. Returns true if any are still active.
    pub fn tick(&mut self) -> bool {
        self.animations.retain(|_, a| !a.is_finished());
        !self.animations.is_empty()
    }

    pub fn has_active(&self) -> bool {
        !self.animations.is_empty()
    }

    /// Remove animation for a surface (e.g. on close).
    pub fn remove(&mut self, surface: &WlSurface) {
        self.animations.remove(surface);
    }
}

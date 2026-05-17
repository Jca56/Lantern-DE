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

use crate::rect_anim::RectAnim;

fn anim_duration() -> Duration { crate::animations::tiling_duration() }
fn tiling_curve() -> crate::rect_anim::Curve { crate::animations::tiling_curve() }

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
            existing.redirect(target_rect, anim_duration(), tiling_curve());
            return;
        }

        // Don't animate if start == target (no visible change).
        if current_rect == target_rect {
            self.animations.remove(surface);
            return;
        }

        self.animations.insert(
            surface.clone(),
            RectAnim::new(current_rect, target_rect, anim_duration(), tiling_curve()),
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

/// Tiling layout animation: per-window rect interpolation.
///
/// When the tiling layout changes (insert, remove, resize), each window
/// smoothly animates from its current position/size to the new target.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
};

const ANIM_DURATION: Duration = Duration::from_millis(250);

pub struct RectAnimation {
    start_rect: Rectangle<i32, Logical>,
    target_rect: Rectangle<i32, Logical>,
    start_time: Instant,
    duration: Duration,
}

impl RectAnimation {
    fn new(start: Rectangle<i32, Logical>, target: Rectangle<i32, Logical>) -> Self {
        Self {
            start_rect: start,
            target_rect: target,
            start_time: Instant::now(),
            duration: ANIM_DURATION,
        }
    }

    fn progress(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let t = (elapsed / self.duration.as_secs_f64()).clamp(0.0, 1.0);
        // Ease-out cubic: 1 - (1-t)^3
        let inv = 1.0 - t;
        1.0 - inv * inv * inv
    }

    fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Returns the interpolated rectangle at the current time.
    pub fn current_rect(&self) -> Rectangle<i32, Logical> {
        let p = self.progress();
        let lerp = |a: i32, b: i32| -> i32 { a + ((b - a) as f64 * p).round() as i32 };

        Rectangle::new(
            Point::from((
                lerp(self.start_rect.loc.x, self.target_rect.loc.x),
                lerp(self.start_rect.loc.y, self.target_rect.loc.y),
            )),
            Size::from((
                lerp(self.start_rect.size.w, self.target_rect.size.w),
                lerp(self.start_rect.size.h, self.target_rect.size.h),
            )),
        )
    }
}

pub struct TilingAnimationState {
    animations: HashMap<WlSurface, RectAnimation>,
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
        // If already animating, start from current interpolated position
        let start = self
            .animations
            .get(surface)
            .map(|a| a.current_rect())
            .unwrap_or(current_rect);

        // Don't animate if start == target (no visible change)
        if start == target_rect {
            self.animations.remove(surface);
            return;
        }

        self.animations
            .insert(surface.clone(), RectAnimation::new(start, target_rect));
    }

    /// Get the current interpolated rect for a surface, if animating.
    pub fn current_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.animations.get(surface).map(|a| a.current_rect())
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

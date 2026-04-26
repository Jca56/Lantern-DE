//! Generic rectangle interpolation animation with smooth interrupt/redirect.
//!
//! Used as the primitive for tiling, maximize, fullscreen, snap, and minimize
//! transitions. Each animation interpolates a logical Rectangle over time using
//! a configurable easing Curve, and supports being redirected mid-flight from
//! the current interpolated rect to a new target without snapping.

use std::time::{Duration, Instant};

use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::easing;

/// Easing curve options for rect animations.
#[derive(Debug, Clone, Copy)]
pub enum Curve {
    /// Damped spring oscillation. Bouncy, settles toward 1.0.
    Spring { damping: f64, frequency: f64 },
    /// Ease-out cubic — fast start, decelerating finish.
    EaseOutCubic,
    /// Ease-in-out cubic — symmetric smooth-smooth.
    EaseInOutCubic,
    /// Ease-in-out quintic — slow ends, snappy middle. Cinematic.
    EaseInOutQuint,
}

impl Curve {
    /// Evaluate the curve at linear progress `t` ∈ [0,1].
    pub fn eval(self, t: f64) -> f64 {
        match self {
            Curve::Spring { damping, frequency } => easing::spring(t, damping, frequency),
            Curve::EaseOutCubic => easing::ease_out_cubic(t),
            Curve::EaseInOutCubic => easing::ease_in_out_cubic(t),
            Curve::EaseInOutQuint => easing::ease_in_out_quint(t),
        }
    }
}

/// A single rect interpolation in flight.
#[derive(Debug, Clone)]
pub struct RectAnim {
    start: Rectangle<i32, Logical>,
    target: Rectangle<i32, Logical>,
    start_time: Instant,
    duration: Duration,
    curve: Curve,
}

impl RectAnim {
    pub fn new(
        start: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
        duration: Duration,
        curve: Curve,
    ) -> Self {
        Self {
            start,
            target,
            start_time: Instant::now(),
            duration,
            curve,
        }
    }

    pub fn target(&self) -> Rectangle<i32, Logical> {
        self.target
    }

    pub fn start_rect(&self) -> Rectangle<i32, Logical> {
        self.start
    }

    /// Linear progress 0..=1.
    fn raw_progress(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        (elapsed / self.duration.as_secs_f64()).clamp(0.0, 1.0)
    }

    /// Eased progress (may overshoot 1.0 for spring curves mid-flight).
    pub fn eased(&self) -> f64 {
        self.curve.eval(self.raw_progress())
    }

    pub fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Interpolated rectangle at the current time.
    pub fn current(&self) -> Rectangle<i32, Logical> {
        let p = self.eased();
        let lerp = |a: i32, b: i32| -> i32 { a + ((b - a) as f64 * p).round() as i32 };
        Rectangle::new(
            Point::from((
                lerp(self.start.loc.x, self.target.loc.x),
                lerp(self.start.loc.y, self.target.loc.y),
            )),
            Size::from((
                lerp(self.start.size.w, self.target.size.w).max(1),
                lerp(self.start.size.h, self.target.size.h).max(1),
            )),
        )
    }

    /// Redirect this animation to a new target, starting from the current
    /// interpolated rect. Use when the user retriggers a state change mid-flight
    /// (e.g. maximize → unmaximize before the first finishes).
    pub fn redirect(&mut self, new_target: Rectangle<i32, Logical>, duration: Duration, curve: Curve) {
        let from = self.current();
        self.start = from;
        self.target = new_target;
        self.start_time = Instant::now();
        self.duration = duration;
        self.curve = curve;
    }
}

// ── Duration presets (seconds) ────────────────────────────────────────────────

pub const DURATION_FAST: f32 = 0.1;
pub const DURATION_NORMAL: f32 = 0.2;
pub const DURATION_SLOW: f32 = 0.35;
pub const DURATION_ENTER: f32 = 0.25;
pub const DURATION_EXIT: f32 = 0.15;

// ── Easing functions ─────────────────────────────────────────────────────────

/// Linear interpolation (no easing).
pub fn linear(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// Ease-out cubic — fast start, slow finish (great for enter animations).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

/// Ease-in cubic — slow start, fast finish (great for exit animations).
pub fn ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

/// Ease-in-out cubic — smooth start and finish.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Spring-like overshoot ease-out (bouncy feel, subtle).
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

// ── Animation progress helper ────────────────────────────────────────────────

/// Calculate animation progress from elapsed time and total duration.
/// Returns a 0.0..=1.0 value. Apply an easing function to the result.
pub fn progress(elapsed: f32, duration: f32) -> f32 {
    if duration <= 0.0 {
        return 1.0;
    }
    (elapsed / duration).clamp(0.0, 1.0)
}

/// Interpolate between two values using a progress (0.0 to 1.0).
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}


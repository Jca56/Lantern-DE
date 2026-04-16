/// Easing functions for animations.
///
/// All functions take `t` in 0.0..=1.0 and return an eased value.
/// Input is clamped so out-of-range t is safe.

/// Ease-out cubic: fast start, decelerating finish.
/// Good default for most UI transitions.
pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

/// Ease-in cubic: slow start, accelerating finish.
/// Good for elements leaving the screen (closes, dismissals).
pub fn ease_in_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

/// Ease-in-out cubic: smooth start and finish.
/// Good for transitions that should feel balanced.
pub fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Ease-out back: overshoots target slightly, then settles.
/// `overshoot` controls how far past 1.0 it goes (~1.70158 is standard).
/// Great for springy open animations.
pub fn ease_out_back(t: f64, overshoot: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let c = overshoot + 1.0;
    let inv = t - 1.0;
    1.0 + c * inv * inv * inv + overshoot * inv * inv
}

/// Damped spring: critically/under-damped oscillation that settles to 1.0.
///
/// - `damping`: 0.3..0.8 typical. Lower = more bouncy. 1.0 = critically damped.
/// - `frequency`: oscillation speed. 4.0..8.0 typical.
///
/// Returns eased value (may overshoot 1.0 during bounce).
pub fn spring(t: f64, damping: f64, frequency: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 {
        return 1.0;
    }
    let omega_n = frequency * std::f64::consts::TAU;
    let omega_d = omega_n * (1.0 - damping * damping).max(0.0).sqrt();
    let decay = (-damping * omega_n * t).exp();
    if omega_d < 1e-10 {
        // Critically damped: no oscillation
        1.0 - decay * (1.0 + omega_n * t)
    } else {
        1.0 - decay * ((omega_d * t).cos() + (damping * omega_n / omega_d) * (omega_d * t).sin())
    }
}

/// f32 version of ease_out_cubic for switcher/UI code.
pub fn ease_out_cubic_f32(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_easings_start_at_zero() {
        assert!((ease_out_cubic(0.0)).abs() < 1e-10);
        assert!((ease_in_cubic(0.0)).abs() < 1e-10);
        assert!((ease_in_out_cubic(0.0)).abs() < 1e-10);
        assert!((ease_out_back(0.0, 1.70158)).abs() < 1e-10);
        assert!((spring(0.0, 0.5, 6.0)).abs() < 1e-10);
    }

    #[test]
    fn all_easings_end_at_one() {
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-10);
        assert!((ease_in_cubic(1.0) - 1.0).abs() < 1e-10);
        assert!((ease_in_out_cubic(1.0) - 1.0).abs() < 1e-10);
        assert!((ease_out_back(1.0, 1.70158) - 1.0).abs() < 1e-10);
        assert!((spring(1.0, 0.5, 6.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn ease_out_back_overshoots() {
        // Should exceed 1.0 somewhere in the middle
        let mid = ease_out_back(0.7, 1.70158);
        assert!(mid > 1.0, "ease_out_back should overshoot: got {mid}");
    }

    #[test]
    fn spring_bounces() {
        // With low damping, spring should overshoot 1.0
        let mid = spring(0.3, 0.4, 6.0);
        assert!(mid > 1.0 || mid < 0.0 || (mid - 1.0).abs() > 0.01,
            "spring should oscillate: got {mid}");
    }

    #[test]
    fn clamped_inputs() {
        // Negative and >1.0 inputs should be safe
        assert!((ease_out_cubic(-0.5)).abs() < 1e-10);
        assert!((ease_out_cubic(1.5) - 1.0).abs() < 1e-10);
        assert!((ease_in_cubic(-0.5)).abs() < 1e-10);
        assert!((spring(2.0, 0.5, 6.0) - 1.0).abs() < 1e-10);
    }
}

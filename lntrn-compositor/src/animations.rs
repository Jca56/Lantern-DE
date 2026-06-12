//! Animation timing helpers.
//!
//! All window-motion durations route through this module so the user's
//! `[animations]` settings (enabled toggle + speed multiplier + named preset
//! + per-category toggles) take effect across the compositor. Config reads
//! use the existing mtime cache, so these are essentially free.
//!
//! Convention: `speed` > 1.0 = faster (shorter duration), < 1.0 = slower.
//! `duration = base / speed`. When animations are disabled, every helper
//! returns 1ms so the animation completes in one frame without divide-by-
//! zero hazards in the easing/progress code.

use std::time::Duration;

use crate::rect_anim::Curve;

/// Master enable. Default true.
pub fn enabled() -> bool {
    crate::read_config("animations", "enabled", "true") == "true"
}

/// Speed multiplier. Higher = faster. Clamped to a sensible range.
pub fn speed() -> f64 {
    crate::read_config("animations", "speed", "1.0")
        .parse::<f64>()
        .unwrap_or(1.0)
        .clamp(0.25, 4.0)
}

/// Active preset name. One of: cinematic, snappy, springy, linear.
pub fn preset() -> String {
    crate::read_config("animations", "preset", "cinematic")
}

/// Per-category toggle. Keys live alongside `enabled`/`speed`/`preset` in
/// `[animations]`. AND'd with the master toggle.
fn category_enabled(key: &str) -> bool {
    if !enabled() { return false; }
    crate::read_config("animations", key, "true") == "true"
}

/// Scale a base duration by current speed. Returns 1ms when the master
/// toggle is off so animation code that interpolates against the duration
/// short-circuits to "done" on the very next tick.
pub fn scaled(base: Duration) -> Duration {
    if !enabled() {
        return Duration::from_millis(1);
    }
    let ms = (base.as_millis() as f64 / speed()).max(1.0) as u64;
    Duration::from_millis(ms)
}

/// Same as `scaled` but also honors a per-category toggle.
fn scaled_cat(base: Duration, key: &str) -> Duration {
    if !category_enabled(key) {
        return Duration::from_millis(1);
    }
    scaled(base)
}

// Per-category base durations. Kept private so call sites have to go through
// the named helpers below.

const BASE_OPEN: Duration = Duration::from_millis(1000);
const BASE_CLOSE: Duration = Duration::from_millis(1000);
const BASE_STATE: Duration = Duration::from_millis(1000);
const BASE_MINIMIZE: Duration = Duration::from_millis(1000);
const BASE_UNMINIMIZE: Duration = Duration::from_millis(1000);
const BASE_WORKSPACE_SLIDE: Duration = Duration::from_millis(1000);

pub fn open_duration() -> Duration { scaled_cat(BASE_OPEN, "open_close") }
pub fn close_duration() -> Duration { scaled_cat(BASE_CLOSE, "open_close") }
pub fn state_duration() -> Duration { scaled_cat(BASE_STATE, "state") }
pub fn minimize_duration() -> Duration { scaled_cat(BASE_MINIMIZE, "minimize") }
pub fn unminimize_duration() -> Duration { scaled_cat(BASE_UNMINIMIZE, "minimize") }
pub fn workspace_slide_duration() -> Duration { scaled_cat(BASE_WORKSPACE_SLIDE, "workspace") }

// ── Preset → curve lookup ─────────────────────────────────────────────────

// Unified "springy growth" curve shared by open, state (maximize/restore),
// and unminimize. EaseOutBack uses the full duration so the bounce reads at
// the same pace as the other animations.
const SPRINGY_BOUNCE: Curve = Curve::EaseOutBack { overshoot: 1.70158 };

pub fn open_curve() -> Curve {
    match preset().as_str() {
        "snappy" => Curve::EaseOutCubic,
        "springy" => SPRINGY_BOUNCE,
        "linear" => Curve::Linear,
        _ => Curve::EaseInOutQuint,
    }
}

pub fn close_curve() -> Curve {
    match preset().as_str() {
        "snappy" => Curve::EaseInCubic,
        "springy" => Curve::EaseInCubic,
        "linear" => Curve::Linear,
        _ => Curve::EaseInOutQuint,
    }
}

pub fn state_curve() -> Curve {
    match preset().as_str() {
        "snappy" => Curve::EaseOutCubic,
        "springy" => SPRINGY_BOUNCE,
        "linear" => Curve::Linear,
        _ => Curve::EaseInOutQuint,
    }
}

pub fn minimize_curve() -> Curve {
    match preset().as_str() {
        "snappy" => Curve::EaseInCubic,
        "springy" => Curve::EaseInCubic,
        "linear" => Curve::Linear,
        _ => Curve::EaseInOutQuint,
    }
}

/// Curve used when restoring a window FROM the minimize tray (Unminimize).
/// Distinct from `minimize_curve` because restore is a "growth" motion and
/// benefits from a different feel per preset — Springy in particular gets a
/// proper bounce instead of EaseInCubic.
pub fn unminimize_curve() -> Curve {
    match preset().as_str() {
        "snappy" => Curve::EaseOutCubic,
        "springy" => SPRINGY_BOUNCE,
        "linear" => Curve::Linear,
        _ => Curve::EaseInOutQuint,
    }
}

pub fn workspace_curve() -> Curve {
    match preset().as_str() {
        "snappy" => Curve::EaseOutCubic,
        "linear" => Curve::Linear,
        _ => Curve::EaseInOutQuint,
    }
}

/// True when the active preset opens windows with a grow-from-center slide
/// instead of pure alpha fade. Today: springy only.
pub fn open_uses_grow() -> bool {
    preset() == "springy"
}

//! Animation timing helpers.
//!
//! All window-motion durations route through this module so the user's
//! `[animations]` settings (enabled toggle + speed multiplier) take effect
//! across the compositor. Config reads use the existing mtime cache, so
//! these are essentially free.
//!
//! Convention: `speed` > 1.0 = faster (shorter duration), < 1.0 = slower.
//! `duration = base / speed`. When animations are disabled, every helper
//! returns 1ms so the animation completes in one frame without divide-by-
//! zero hazards in the easing/progress code.

use std::time::Duration;

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

/// Scale a base duration by current speed. Returns 1ms when disabled so
/// animation code that interpolates against the duration short-circuits to
/// "done" on the very next tick instead of dividing by zero.
pub fn scaled(base: Duration) -> Duration {
    if !enabled() {
        return Duration::from_millis(1);
    }
    let ms = (base.as_millis() as f64 / speed()).max(1.0) as u64;
    Duration::from_millis(ms)
}

// Per-category base durations. Kept private so call sites have to go through
// the named helpers below — that way it's obvious where each comes from when
// tweaking timing.

const BASE_OPEN: Duration = Duration::from_millis(1000);
const BASE_CLOSE: Duration = Duration::from_millis(1000);
const BASE_STATE: Duration = Duration::from_millis(1000);
const BASE_MINIMIZE: Duration = Duration::from_millis(1000);
const BASE_UNMINIMIZE: Duration = Duration::from_millis(1000);
const BASE_TILING: Duration = Duration::from_millis(1000);
const BASE_WORKSPACE_SLIDE: Duration = Duration::from_millis(1000);

pub fn open_duration() -> Duration { scaled(BASE_OPEN) }
pub fn close_duration() -> Duration { scaled(BASE_CLOSE) }
pub fn state_duration() -> Duration { scaled(BASE_STATE) }
pub fn minimize_duration() -> Duration { scaled(BASE_MINIMIZE) }
pub fn unminimize_duration() -> Duration { scaled(BASE_UNMINIMIZE) }
pub fn tiling_duration() -> Duration { scaled(BASE_TILING) }
pub fn workspace_slide_duration() -> Duration { scaled(BASE_WORKSPACE_SLIDE) }

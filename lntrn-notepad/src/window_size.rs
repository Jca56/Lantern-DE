//! The notepad's initial window size: a portrait rectangle sized as a share of
//! the active output. Requested once at creation and re-asserted once after
//! (Lantern pre-empts the attribute with its `default_size_pct` suggestion) —
//! no feedback loop, no persistence.
//!
//! Two gotchas drove this design, both verified live via the window-query IPC:
//!  1. winit reports no current/primary monitor on this compositor and lists
//!     outputs smallest-first, so the caller must pick the intended output (the
//!     LARGEST one — see `largest_monitor` in main).
//!  2. a `MonitorHandle`'s `scale_factor()` is the INTEGER-rounded scale (2.0 on
//!     the 4K), while the mapped window's real `scale_factor()` is the true
//!     fractional one (1.4). So we size in PHYSICAL pixels here (a share of the
//!     monitor's physical size, which is accurate) and let the caller convert to
//!     the logical request using the *window's* real scale. `request_inner_size`
//!     then lands at `logical × real_scale` = the physical target we wanted —
//!     correct on both the laptop (1.0) and the 4K desktop (1.4).

use winit::monitor::MonitorHandle;

/// Width / height — portrait, clearly taller than wide.
const ASPECT: f64 = 0.75;
/// Fraction of the output's physical height the window should occupy.
const HEIGHT_FRAC: f64 = 0.85;
/// Don't let the (clamped) width exceed this fraction of the output width.
const MAX_WIDTH_FRAC: f64 = 0.9;
/// Floor so a tiny/unknown output can't yield an unusable sliver.
const MIN_SIDE: f64 = 480.0;

/// The desired window size in PHYSICAL pixels for the given monitor. The caller
/// divides by the real window scale factor to get the logical request. Falls
/// back to a fixed portrait size when no output is known.
pub fn portrait_physical(monitor: Option<&MonitorHandle>) -> (f64, f64) {
    let Some(m) = monitor.filter(|m| m.size().width > 0 && m.size().height > 0) else {
        return (900.0, 1200.0);
    };
    let (mon_w, mon_h) = (m.size().width as f64, m.size().height as f64);
    let mut h = mon_h * HEIGHT_FRAC;
    let mut w = h * ASPECT;
    let max_w = mon_w * MAX_WIDTH_FRAC;
    if w > max_w {
        w = max_w;
        h = w / ASPECT;
    }
    (w.max(MIN_SIDE), h.max(MIN_SIDE))
}

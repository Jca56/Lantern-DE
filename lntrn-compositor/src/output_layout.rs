//! Output layout normalization.
//!
//! Invariant: the top-left corner of the bounding box of all enabled outputs
//! is (0, 0) in compositor-global coordinates. The user's `[[monitors]]`
//! arrangement is kept exactly as configured *relative to itself* — the whole
//! layout is just translated so its leftmost/topmost edge lands on the origin.
//! Mutter and KWin do the same with their monitor layouts.
//!
//! Why this matters (2026-08-24, Last Epoch / Borderlands 4 under Proton):
//! XWayland mirrors our global coordinates 1:1 into its X root window and
//! does NOT normalize, so a layout whose leftmost monitor starts at x=215
//! leaves a 215px strip of dead root space on the left. Wine assumes the X
//! root origin coincides with the top-left of its virtual screen (the union
//! of all monitors): `virtual_screen_to_root(x) = x - virtual.left`. Every
//! conversion is then off by the dead-zone width:
//!   - fullscreen windows get placed at root x=0 (we drag them back with
//!     `sync_x11_position`, but Wine keeps re-requesting *its* rect through
//!     ConfigureRequest and blocks its Win32 state sync while that's pending
//!     → game comes back Win32-minimized after Alt+Tab = black/flicker),
//!   - the cursor-clip InputOnly window lands 215px left of the game,
//!   - clip-relative motion is reported 215px right of the visible cursor
//!     ("the cursor is to the LEFT of where the game thinks it is").
//! Normalizing removes the disagreement at the source.
//!
//! Two entry points, both pure functions over the data that carries
//! positions: `[[monitors]]` entries as they are read, and the head batch a
//! wlr-output-management client (System Settings) applies.

use crate::handlers::output_management::OutputChange;
use crate::MonitorConfig;

/// Top-left corner of the bounding box of `positions`, or (0, 0) when empty.
pub(crate) fn layout_origin(positions: impl IntoIterator<Item = (i32, i32)>) -> (i32, i32) {
    let mut it = positions.into_iter();
    let Some(first) = it.next() else {
        return (0, 0);
    };
    it.fold(first, |(mx, my), (x, y)| (mx.min(x), my.min(y)))
}

/// Translate `monitors` so the bounding box of the *enabled* entries starts
/// at (0, 0). Disabled entries ride along with the same translation so a
/// monitor switched back on re-appears in its configured spot relative to
/// the others. Returns the origin that was removed — (0, 0) means the config
/// was already normalized.
pub(crate) fn normalize_monitor_configs(monitors: &mut [MonitorConfig]) -> (i32, i32) {
    let origin = layout_origin(monitors.iter().filter(|m| m.enabled).map(|m| (m.x, m.y)));
    if origin != (0, 0) {
        for m in monitors.iter_mut() {
            m.x -= origin.0;
            m.y -= origin.1;
        }
    }
    origin
}

/// Normalize a wlr-output-management batch before it is applied.
///
/// A configuration must cover every head (the protocol requires each head to
/// be either enabled or disabled), so the batch *is* the whole new layout.
/// For every head that stays enabled, its effective position is the explicit
/// one from the client, else its current position (`live`). The batch is
/// translated so the bounding box of those positions starts at (0, 0); heads
/// that only had a live position get an explicit one when the translation is
/// non-zero, so they move together with the rest.
///
/// If an enabled head has no known position at all we leave the batch alone
/// — better to apply it verbatim than to guess.
pub(crate) fn normalize_output_changes(
    changes: &mut [OutputChange],
    live: impl Fn(&str) -> Option<(i32, i32)>,
) -> (i32, i32) {
    let mut effective: Vec<Option<(i32, i32)>> = Vec::with_capacity(changes.len());
    for c in changes.iter() {
        if c.enabled == Some(false) {
            effective.push(None);
            continue;
        }
        let pos = c.position.or_else(|| live(&c.output_name));
        if pos.is_none() {
            return (0, 0);
        }
        effective.push(pos);
    }

    let origin = layout_origin(effective.iter().flatten().copied());
    if origin == (0, 0) {
        return origin;
    }
    for (c, pos) in changes.iter_mut().zip(effective) {
        if let Some((x, y)) = pos {
            c.position = Some((x - origin.0, y - origin.1));
        }
    }
    origin
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, x: i32, y: i32, enabled: bool) -> MonitorConfig {
        MonitorConfig {
            name: name.into(),
            x,
            y,
            resolution: None,
            refresh_rate: None,
            scale: None,
            wallpaper: None,
            primary: false,
            vrr: false,
            hdr: false,
            sdr_brightness: None,
            enabled,
        }
    }

    fn change(name: &str, position: Option<(i32, i32)>, enabled: Option<bool>) -> OutputChange {
        OutputChange {
            output_name: name.into(),
            drm_mode_index: None,
            position,
            scale: None,
            enabled,
        }
    }

    #[test]
    fn origin_of_empty_is_zero() {
        assert_eq!(layout_origin(Vec::<(i32, i32)>::new()), (0, 0));
    }

    #[test]
    fn configs_translate_as_a_whole() {
        let mut m = vec![cfg("DP-1", 215, 0, true), cfg("HDMI-A-1", 2961, 0, true)];
        assert_eq!(normalize_monitor_configs(&mut m), (215, 0));
        assert_eq!((m[0].x, m[0].y), (0, 0));
        assert_eq!((m[1].x, m[1].y), (2746, 0));
    }

    #[test]
    fn disabled_entries_do_not_anchor_but_ride_along() {
        let mut m = vec![cfg("DP-2", 0, 0, false), cfg("eDP-1", 1920, 40, true)];
        assert_eq!(normalize_monitor_configs(&mut m), (1920, 40));
        assert_eq!((m[0].x, m[0].y), (-1920, -40));
        assert_eq!((m[1].x, m[1].y), (0, 0));
    }

    #[test]
    fn already_normalized_config_is_untouched() {
        let mut m = vec![cfg("DP-1", 0, 0, true), cfg("HDMI-A-1", 2746, 0, true)];
        assert_eq!(normalize_monitor_configs(&mut m), (0, 0));
        assert_eq!((m[1].x, m[1].y), (2746, 0));
    }

    #[test]
    fn batch_uses_live_positions_for_unmoved_heads() {
        let mut c = vec![
            change("DP-1", Some((300, 0)), Some(true)),
            change("HDMI-A-1", None, Some(true)),
            change("DP-2", Some((-500, -500)), Some(false)),
        ];
        let live = |n: &str| (n == "HDMI-A-1").then_some((2746, 0));
        assert_eq!(normalize_output_changes(&mut c, live), (300, 0));
        assert_eq!(c[0].position, Some((0, 0)));
        assert_eq!(c[1].position, Some((2446, 0)));
        // Disabled heads are neither anchors nor moved.
        assert_eq!(c[2].position, Some((-500, -500)));
    }

    #[test]
    fn batch_left_alone_when_a_head_has_no_position() {
        let mut c = vec![
            change("DP-1", Some((300, 0)), Some(true)),
            change("HDMI-A-1", None, Some(true)),
        ];
        assert_eq!(normalize_output_changes(&mut c, |_| None), (0, 0));
        assert_eq!(c[0].position, Some((300, 0)));
        assert_eq!(c[1].position, None);
    }
}

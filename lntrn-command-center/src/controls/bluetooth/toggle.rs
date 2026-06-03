//! The iOS-style on/off pill toggle shared by the Bluetooth view's power
//! switch and the Discoverable / Scan switches. Kept in one place so the
//! visual doesn't drift between callers.

use lntrn_render::{Color, Painter, Rect};

/// iOS-style on/off toggle. Same shape as the battery's charge-limit toggle.
pub(super) fn draw_toggle(painter: &mut Painter, rect: Rect, on: bool, alpha: f32, scale: f32) {
    let radius = rect.h * 0.5;
    let track_color = if on {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(alpha)
    } else {
        Color::from_rgb8(0x44, 0x44, 0x44).with_alpha(alpha)
    };
    painter.rect_filled(rect, radius, track_color);
    let inset = 3.0 * scale;
    let knob_r = (rect.h - inset * 2.0) * 0.5;
    let knob_cy = rect.y + rect.h * 0.5;
    let knob_cx = if on {
        rect.x + rect.w - inset - knob_r
    } else {
        rect.x + inset + knob_r
    };
    painter.circle_filled(
        knob_cx,
        knob_cy,
        knob_r,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
    );
}

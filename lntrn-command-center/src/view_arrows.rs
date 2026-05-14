//! Floating chevron buttons on the left/right of the panel that cycle
//! through the top-level [`crate::app::PanelView`] variants.

use lntrn_render::{Color, Painter, Rect};

/// Button rect width/height in logical px.
pub const BTN_W: f32 = 32.0;
pub const BTN_H: f32 = 64.0;
/// Horizontal gap between the panel edge and the arrow.
pub const SIDE_GAP: f32 = 14.0;
pub const RADIUS: f32 = 14.0;
const BG_RGB: (u8, u8, u8) = (24, 24, 24);
const BG_ALPHA_IDLE: f32 = 0.55;
const BG_ALPHA_HOVER: f32 = 0.90;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Anchored to the *top* of the panel so the arrows stay reachable
/// even while the panel is collapsed to just the controls row.
pub fn button_rect(panel: Rect, scale: f32, side: Side) -> Rect {
    let w = BTN_W * scale;
    let h = BTN_H * scale;
    let gap = SIDE_GAP * scale;
    // Center vertically against the controls row so the arrow looks
    // glued to the bar (works for both collapsed and full panel).
    let row_h = crate::controls::total_logical_height() * scale;
    let y = panel.y + (row_h - h) / 2.0;
    let x = match side {
        Side::Left => panel.x - gap - w,
        Side::Right => panel.x + panel.w + gap,
    };
    Rect::new(x, y, w, h)
}

pub fn hit_test(panel: Rect, scale: f32, px: f32, py: f32) -> Option<Side> {
    for side in [Side::Left, Side::Right] {
        let r = button_rect(panel, scale, side);
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(side);
        }
    }
    None
}

pub fn draw(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: Option<Side>,
) {
    let radius = RADIUS * scale;
    for side in [Side::Left, Side::Right] {
        let r = button_rect(panel, scale, side);
        let is_hovered = hovered == Some(side);
        let bg_a = if is_hovered { BG_ALPHA_HOVER } else { BG_ALPHA_IDLE };
        let bg = Color::from_rgb8(BG_RGB.0, BG_RGB.1, BG_RGB.2).with_alpha(bg_a * alpha);
        painter.rect_filled(r, radius, bg);
        if is_hovered {
            painter.rect_stroke_sdf(
                r,
                radius,
                2.0 * scale,
                Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.65 * alpha),
            );
        }
        draw_chevron(painter, r, side, scale, alpha);
    }
}

fn draw_chevron(painter: &mut Painter, btn: Rect, side: Side, scale: f32, alpha: f32) {
    let cx = btn.x + btn.w / 2.0;
    let cy = btn.y + btn.h / 2.0;
    let arm = 9.0 * scale;
    let stroke = 2.5 * scale;
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.92 * alpha);
    match side {
        Side::Left => {
            painter.line_round(cx + arm * 0.5, cy - arm, cx - arm * 0.5, cy, stroke, color);
            painter.line_round(cx - arm * 0.5, cy, cx + arm * 0.5, cy + arm, stroke, color);
        }
        Side::Right => {
            painter.line_round(cx - arm * 0.5, cy - arm, cx + arm * 0.5, cy, stroke, color);
            painter.line_round(cx + arm * 0.5, cy, cx - arm * 0.5, cy + arm, stroke, color);
        }
    }
}

//! Square button that floats under the right view arrow, mirroring
//! `clock_toggle.rs` on the opposite side. Opens the Claude-usage panel.
//!
//! Unlike the clock toggle (which is a primitive vector glyph), this
//! button uses the colored `Claude.svg` asset shipped in
//! `~/.lantern/icons/`, routed through the existing icon-request pipeline.

use lntrn_render::{Color, Painter, Rect};

use crate::render::IconRequest;
use crate::view_arrows::{button_rect as arrow_rect, Side};

/// Square button. Larger than the arrows above it so the colored
/// Claude logo reads at a glance — the brand mark is the visual hook.
pub const BTN_SIZE: f32 = 52.0;
/// Vertical gap between the arrow's bottom edge and this button.
pub const GAP_BELOW_ARROW: f32 = 14.0;
const RADIUS: f32 = 16.0;
const BG_RGB: (u8, u8, u8) = (24, 24, 24);
const BG_ALPHA_IDLE: f32 = 0.55;
const BG_ALPHA_HOVER: f32 = 0.90;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

/// Absolute path to the colored Claude logo we ship in `~/.lantern/icons/`.
pub const CLAUDE_ICON_PATH: &str = "/home/alva/.lantern/icons/Claude.svg";
/// `app_id` we register the Claude icon under in the icon cache.
pub const CLAUDE_ICON_APP_ID: &str = "lntrn-claude-usage";

/// Button rect (physical pixels) anchored beneath the right arrow.
/// Centered horizontally on the arrow so the wider button still hangs
/// off the same column instead of drifting left.
pub fn button_rect(panel: Rect, scale: f32) -> Rect {
    let arrow = arrow_rect(panel, scale, Side::Right);
    let size = BTN_SIZE * scale;
    let gap = GAP_BELOW_ARROW * scale;
    let arrow_cx = arrow.x + arrow.w * 0.5;
    Rect::new(arrow_cx - size * 0.5, arrow.y + arrow.h + gap, size, size)
}

pub fn hit_test(panel: Rect, scale: f32, px: f32, py: f32) -> bool {
    let r = button_rect(panel, scale);
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

pub fn draw(
    painter: &mut Painter,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: bool,
    active: bool,
    icons: &mut Vec<IconRequest>,
) {
    let r = button_rect(panel, scale);
    let radius = RADIUS * scale;
    let bg_a = if hovered || active {
        BG_ALPHA_HOVER
    } else {
        BG_ALPHA_IDLE
    };
    let bg = Color::from_rgb8(BG_RGB.0, BG_RGB.1, BG_RGB.2).with_alpha(bg_a * alpha);
    painter.rect_filled(r, radius, bg);

    let accent = Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2);
    if hovered || active {
        painter.rect_stroke_sdf(r, radius, 2.0 * scale, accent.with_alpha(0.65 * alpha));
    }

    // Inset the Claude glyph slightly so it doesn't kiss the rounded
    // corners.
    let inset = 6.0 * scale;
    let icon_rect = Rect::new(
        r.x + inset,
        r.y + inset,
        (r.w - inset * 2.0).max(0.0),
        (r.h - inset * 2.0).max(0.0),
    );
    icons.push(IconRequest {
        app_id: CLAUDE_ICON_APP_ID.to_string(),
        icon_name: Some(CLAUDE_ICON_PATH.to_string()),
        x: icon_rect.x,
        y: icon_rect.y,
        size: icon_rect.w,
        opacity: alpha,
        clip: None,
    });
}

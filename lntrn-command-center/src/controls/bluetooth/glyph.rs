//! The inline Bluetooth tile — the small bluetooth rune drawn in the
//! controls row, dimmed when the controller is powered off and tinted
//! gold when the tile is the lit/active one. While a connected device
//! reports a battery level, the lowest one shows to the rune's right.

use lntrn_render::{Color, Painter, TextRenderer};

use super::battery::figure_color;
use super::Bluetooth;
use crate::controls::tile::TileLayout;

const ICON_SIZE: f32 = 28.0;
const ICON_LEFT_PAD: f32 = 16.0;

/// Font for the battery figure. Matches the laptop battery tile's
/// percentage so the row's two charge readouts are the same size.
const BATTERY_FONT: f32 = 22.0;
/// Logical px between the rune and the battery figure.
const ICON_BATTERY_GAP: f32 = 6.0;
/// Room held for the figure: enough for "100%" at `BATTERY_FONT`. A fixed
/// estimate (text can't be measured when slots are sized), and the same
/// for every level so the row doesn't shuffle as a battery drains.
const BATTERY_FIGURE_W: f32 = 62.0;

/// Logical width of the rune alone. The `TileId::logical_width` fallback;
/// the live width comes from [`tile_width`].
pub const TILE_WIDTH: f32 = ICON_LEFT_PAD + ICON_SIZE;

/// The tile's logical width right now: the rune, plus room for the
/// battery figure while a connected device reports one.
pub fn tile_width(bt: &Bluetooth) -> f32 {
    if bt.lowest_connected_battery().is_some() {
        TILE_WIDTH + ICON_BATTERY_GAP + BATTERY_FIGURE_W
    } else {
        TILE_WIDTH
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    bt: &Bluetooth,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    lit: bool,
) {
    if !bt.is_present() {
        return;
    }
    let icon_size = ICON_SIZE * scale;
    let icon_x = layout.x + ICON_LEFT_PAD * scale;
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;
    let icon_alpha = if bt.is_powered() { alpha } else { 0.30 * alpha };
    let color = if lit {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(icon_alpha)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(icon_alpha)
    };
    draw_bt_glyph_colored(painter, icon_x, icon_y, icon_size, icon_size, color);

    if let Some(pct) = bt.lowest_connected_battery() {
        let font = BATTERY_FONT * scale;
        let figure = format!("{pct}%");
        let figure_w = text.measure_width(&figure, font);
        // Slack past the measured width: an exact max_width gets quantized
        // down, which wraps the figure and clips all but its last glyph.
        text.queue(
            &figure,
            font,
            icon_x + icon_size + ICON_BATTERY_GAP * scale,
            layout.y + (layout.h - font) / 2.0,
            figure_color(pct, alpha),
            figure_w + 8.0 * scale,
            surface_w,
            surface_h,
        );
    }
}

#[allow(dead_code)]
fn draw_bt_glyph(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    draw_bt_glyph_colored(painter, x, y, w, h, color);
}

fn draw_bt_glyph_colored(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, color: Color) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
    // We render the bluetooth rune as a stroked path: top-bottom spine
    // line + four diagonal lines forming the two bowed-out shapes.
    // Stroked shapes look better than triangle-fanning a non-convex
    // polygon (lesson from the lightning bolt).
    let stroke = w * 0.12;
    let top = pt(0.50, 0.05);
    let bot = pt(0.50, 0.95);
    let mid_left = pt(0.20, 0.30);
    let mid_left_b = pt(0.20, 0.70);
    let upper_right = pt(0.80, 0.30);
    let lower_right = pt(0.80, 0.70);
    let center = pt(0.50, 0.50);

    // Spine (vertical line top → bottom).
    painter.line_round(top.0, top.1, bot.0, bot.1, stroke, color);
    // Top diamond: top → upper-right → center.
    painter.line_round(top.0, top.1, upper_right.0, upper_right.1, stroke, color);
    painter.line_round(
        upper_right.0,
        upper_right.1,
        center.0,
        center.1,
        stroke,
        color,
    );
    // Bottom diamond: center → lower-right → bottom.
    painter.line_round(
        center.0,
        center.1,
        lower_right.0,
        lower_right.1,
        stroke,
        color,
    );
    painter.line_round(lower_right.0, lower_right.1, bot.0, bot.1, stroke, color);
    // The two left-side cross strokes that complete the bowtie.
    painter.line_round(top.0, top.1, mid_left.0, mid_left.1, stroke, color);
    painter.line_round(mid_left.0, mid_left.1, center.0, center.1, stroke, color);
    painter.line_round(
        center.0,
        center.1,
        mid_left_b.0,
        mid_left_b.1,
        stroke,
        color,
    );
    painter.line_round(mid_left_b.0, mid_left_b.1, bot.0, bot.1, stroke, color);
}

//! Temperature tile. Draws a stylized thermometer (bulb + stem) + the
//! current CPU package temperature in °C. State is sourced from
//! [`crate::controls::sysmon::SysMon`] so we don't duplicate the
//! /proc-scanning worker.
//!
//! Painted with Painter primitives (filled circle + rounded rect) — no
//! SVG textures — so the icon's exact pixel bounds are unambiguous and
//! the number can sit cleanly to its right.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::sysmon::SysMon;
use crate::controls::tile::TileLayout;
use crate::render::IconRequest;

/// Logical width of the tile, sized tight to its content: left pad +
/// thermometer + gap + up to a 3-digit number.
pub const TILE_WIDTH: f32 = 64.0;

/// Thermometer geometry (all logical px).
const STEM_W: f32 = 6.0;
const STEM_H: f32 = 30.0;
const BULB_R: f32 = 8.0;
/// Vertical overlap of the bulb into the stem so they form one shape.
const STEM_BULB_OVERLAP: f32 = 3.0;

/// Logical px from tile left edge to the thermometer's left edge.
const ICON_LEFT_PAD: f32 = 10.0;

/// Font for the temperature number. Matches the GPU tile's temp text
/// (gpu.rs `TOP_FONT`) so the two thermals read at the same size.
const NUMBER_FONT: f32 = 18.0;

/// Logical px gap between the bulb's right edge and the digits. Tight so
/// the number tucks in close to the thermometer.
const ICON_NUMBER_GAP: f32 = 4.0;

/// Logical px the number is nudged upward from vertical center so its top
/// lines up with the top of the thermometer icon.
const NUMBER_RISE: f32 = 12.0;

/// Temperature thresholds (°C). Below COOL → blue; between → orange;
/// ≥ HOT → red.
const TEMP_COOL_C: f32 = 55.0;
const TEMP_HOT_C: f32 = 75.0;

const COOL_RGB: (u8, u8, u8) = (0x4c, 0xd1, 0x64);
const WARM_RGB: (u8, u8, u8) = (0xff, 0xd6, 0x3c);
const HOT_RGB: (u8, u8, u8) = (0xff, 0x50, 0x40);

pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    _icons: &mut Vec<IconRequest>,
    sysmon: &SysMon,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let Some(temp_c) = sysmon.last_temp_c else {
        return;
    };

    let stem_w = STEM_W * scale;
    let stem_h = STEM_H * scale;
    let bulb_r = BULB_R * scale;
    let overlap = STEM_BULB_OVERLAP * scale;
    let number_font = NUMBER_FONT * scale;
    let gap = ICON_NUMBER_GAP * scale;
    let left_pad = ICON_LEFT_PAD * scale;

    // Total icon footprint.
    let icon_w = bulb_r * 2.0;
    let icon_h = stem_h + bulb_r * 2.0 - overlap;

    // Position: anchored left, centered vertically.
    let icon_left = layout.x + left_pad;
    let icon_top = layout.y + (layout.h - icon_h) / 2.0;

    // Stem (rounded rect, centered over the bulb).
    let stem_x = icon_left + (icon_w - stem_w) / 2.0;
    let stem_y = icon_top;
    let bulb_cx = icon_left + icon_w / 2.0;
    let bulb_cy = stem_y + stem_h + bulb_r - overlap;

    let (r, g, b) = temp_color(temp_c);
    let color = Color::from_rgb8(r, g, b).with_alpha(alpha);

    painter.rect_filled(
        Rect::new(stem_x, stem_y, stem_w, stem_h),
        stem_w * 0.5,
        color,
    );
    painter.circle_filled(bulb_cx, bulb_cy, bulb_r, color);

    // Number sits to the right of the bulb.
    let number = format!("{}", temp_c.round() as i32);
    let number_w = text.measure_width(&number, number_font);
    let number_x = icon_left + icon_w + gap;
    let number_y = layout.y + (layout.h - number_font) / 2.0 - NUMBER_RISE * scale;
    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.92);
    // Wrap box gets a little slack past the measured width — feeding the
    // exact width back as max_width lets quantization round the box a hair
    // too narrow, which wraps the digits and clips all but the last one.
    text.queue(
        &number,
        number_font,
        number_x,
        number_y,
        white,
        number_w + 8.0 * scale,
        surface_w,
        surface_h,
    );
}

fn temp_color(c: f32) -> (u8, u8, u8) {
    if c < TEMP_COOL_C {
        COOL_RGB
    } else if c < TEMP_HOT_C {
        WARM_RGB
    } else {
        HOT_RGB
    }
}

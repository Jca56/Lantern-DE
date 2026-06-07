//! GPU tile: utilization + temperature on the top line, VRAM usage
//! beneath. Data comes from [`crate::controls::sysmon::SysMon`] (the
//! sysmon worker polls the GPU) so we don't run a second poller.

use lntrn_render::{Color, Painter, TextRenderer};

use crate::controls::sysmon::SysMon;
use crate::controls::tile::TileLayout;
use crate::render::IconRequest;

/// Logical width reserved for the GPU tile in the controls row.
pub const TILE_WIDTH: f32 = 134.0;

const TOP_FONT: f32 = 18.0;
const BOTTOM_FONT: f32 = 14.0;
const LINE_GAP: f32 = 2.0;
const LEFT_PAD: f32 = 10.0;
/// Logical gap between the "GPU 45%" label and the temperature reading.
const TEMP_GAP: f32 = 8.0;

const SECONDARY_ALPHA: f32 = 0.7;

/// Temp thresholds (°C). Cool < 60 green, warm < 80 amber, hot ≥ 80 red.
const COOL_RGB: (u8, u8, u8) = (0x4c, 0xd1, 0x64);
const WARM_RGB: (u8, u8, u8) = (0xff, 0xd6, 0x3c);
const HOT_RGB: (u8, u8, u8) = (0xff, 0x50, 0x40);

pub fn draw_inline(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    _icons: &mut Vec<IconRequest>,
    sysmon: &SysMon,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let Some(g) = sysmon.last_gpu else {
        return;
    };

    let top_font = TOP_FONT * scale;
    let bottom_font = BOTTOM_FONT * scale;
    let gap = LINE_GAP * scale;
    let left_pad = LEFT_PAD * scale;

    let stack_h = top_font + gap + bottom_font;
    let cy = layout.y + (layout.h - stack_h) / 2.0;
    let x = layout.x + left_pad;

    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    let secondary = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * SECONDARY_ALPHA);

    // Top line: "GPU 45%" + temperature (colored by heat).
    let util = format!("GPU {}%", g.util_pct.round() as i32);
    let util_w = text.measure_width(&util, top_font);
    text.queue(&util, top_font, x, cy, white, util_w + 8.0 * scale, surface_w, surface_h);

    let temp = format!("{}°", g.temp_c.round() as i32);
    let temp_w = text.measure_width(&temp, top_font);
    let (tr, tg, tb) = temp_color(g.temp_c);
    let temp_col = Color::from_rgb8(tr, tg, tb).with_alpha(alpha);
    text.queue(
        &temp,
        top_font,
        x + util_w + TEMP_GAP * scale,
        cy,
        temp_col,
        temp_w + 8.0 * scale,
        surface_w,
        surface_h,
    );

    // Bottom line: VRAM used / total in GB.
    let vram = format!(
        "{:.1}/{:.0} GB",
        g.vram_used_mb as f32 / 1024.0,
        g.vram_total_mb as f32 / 1024.0,
    );
    let vram_w = text.measure_width(&vram, bottom_font);
    text.queue(
        &vram,
        bottom_font,
        x,
        cy + top_font + gap,
        secondary,
        vram_w + 8.0 * scale,
        surface_w,
        surface_h,
    );
}

fn temp_color(c: f32) -> (u8, u8, u8) {
    if c < 60.0 {
        COOL_RGB
    } else if c < 80.0 {
        WARM_RGB
    } else {
        HOT_RGB
    }
}

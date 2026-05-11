//! Temperature tile. A small companion to the sysmon tile that
//! surfaces just the CPU package temperature as a thermometer icon +
//! number. State is sourced from [`crate::controls::sysmon::SysMon`]
//! so we don't duplicate the /proc-scanning worker.
//!
//! The icon comes from `~/.lantern/icons/spark-temp-{cool,warm,hot}.svg`.
//! Source art's viewBox is 20x48 (tall thermometer). IconCache
//! rasterizes into a *square* texture, so resvg preserves aspect and
//! centers the thermometer inside that square — the actual visible
//! thermometer ends up occupying only the middle ~42% of the texture
//! width, with transparent margins on the left and right. We have to
//! account for that here: the number sits next to the visible
//! thermometer's right edge, not the texture's right edge, and the
//! visible content (thermometer + number) is what's centered in the
//! tile.

use lntrn_render::{Color, Painter, TextRenderer};

use crate::controls::sysmon::SysMon;
use crate::controls::tile::TileLayout;
use crate::render::IconRequest;

/// Logical width of the tile. Wide enough for the centered visible
/// thermometer + a 2- or 3-digit temperature reading.
pub const TILE_WIDTH: f32 = 88.0;

/// Square box for the icon texture, as a fraction of the tile row
/// height. Effectively 1.0 = use the full row height for the icon.
const ICON_BOX_FRAC: f32 = 1.0;

/// Where the visible thermometer's left/right edges sit inside the
/// rasterized square texture, expressed as fractions of the box width.
/// Derived from the source SVG aspect (20/48) — resvg centers the
/// rendered art, so the visible art lives in `[0.5 - half, 0.5 + half]`
/// where `half = (20/48)/2 ≈ 0.208`. Hardcoded so we don't recompute
/// every frame.
const VISIBLE_FRAC_LEFT: f32 = 0.5 - (20.0 / 48.0) / 2.0;
const VISIBLE_FRAC_RIGHT: f32 = 0.5 + (20.0 / 48.0) / 2.0;

/// Font for the temperature number. Sized to read clearly at a glance.
const NUMBER_FONT: f32 = 22.0;

/// Visual gap between the bulb's right edge and the digits.
const ICON_NUMBER_GAP: f32 = 6.0;

/// Temperature thresholds (°C). Below COOL → `spark-temp-cool.svg`;
/// between → `spark-temp-warm.svg`; ≥ HOT → `spark-temp-hot.svg`.
const TEMP_COOL_C: f32 = 55.0;
const TEMP_HOT_C: f32 = 75.0;

pub fn draw_inline(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    sysmon: &SysMon,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    // No temperature reading yet → leave the slot blank rather than
    // draw a placeholder; on first sample (~100ms after panel opens)
    // the tile renders for real.
    let Some(temp_c) = sysmon.last_temp_c else {
        return;
    };

    let number_font = NUMBER_FONT * scale;
    let gap = ICON_NUMBER_GAP * scale;
    let icon_box = layout.h * ICON_BOX_FRAC;

    let visible_left_off = icon_box * VISIBLE_FRAC_LEFT;
    let visible_right_off = icon_box * VISIBLE_FRAC_RIGHT;
    let visible_w = visible_right_off - visible_left_off;

    let number = format!("{}", temp_c.round() as i32);
    let number_w = text.measure_width(&number, number_font);

    // Center the visible content (thermometer + gap + number) — NOT
    // the texture, since the texture has wide transparent margins.
    let visible_content_w = visible_w + gap + number_w;
    let visible_content_left = layout.x + (layout.w - visible_content_w) / 2.0;

    // Place the icon texture so that the *visible* thermometer's left
    // edge lands at `visible_content_left`. The texture extends a bit
    // further left into transparent space, which is fine.
    let icon_x = visible_content_left - visible_left_off;
    let icon_y = layout.y + (layout.h - icon_box) / 2.0;

    let icon_name = temp_icon_name(temp_c);
    icons.push(IconRequest {
        app_id: icon_name.to_string(),
        icon_name: Some(icon_name.to_string()),
        x: icon_x,
        y: icon_y,
        size: icon_box,
        opacity: alpha,
        clip: None,
    });

    let number_x = visible_content_left + visible_w + gap;
    let number_y = layout.y + (layout.h - number_font) / 2.0;
    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.92);
    text.queue(
        &number,
        number_font,
        number_x,
        number_y,
        white,
        number_w,
        surface_w,
        surface_h,
    );
}

/// Map a temperature in °C to the icon name we want to display.
fn temp_icon_name(c: f32) -> &'static str {
    if c < TEMP_COOL_C {
        "spark-temp-cool"
    } else if c < TEMP_HOT_C {
        "spark-temp-warm"
    } else {
        "spark-temp-hot"
    }
}

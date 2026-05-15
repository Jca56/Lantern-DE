//! Rainbow gradient widget — a draggable 800×600 panel with vertical
//! bands of color whose hue rotates over time. No gradient primitive in
//! the painter, so we fake one with many thin rect fills.

use lntrn_render::{Color, Painter, Rect};

/// Logical-pixel size of the widget. Medium-ish, not screen-dominating.
pub const WIDGET_W: f32 = 800.0;
pub const WIDGET_H: f32 = 600.0;

const CORNER_RADIUS: f32 = 18.0;
const BAND_COUNT: usize = 64;
/// How fast the hue sweeps. One full wheel rotation every ~12 s.
const HUE_RATE_HZ: f32 = 1.0 / 12.0;

/// Default top-left position when the user hasn't dragged the widget yet —
/// center it in the surface.
pub fn default_position(surface_w: u32, surface_h: u32) -> (f32, f32) {
    let sw = surface_w as f32;
    let sh = surface_h as f32;
    ((sw - WIDGET_W) * 0.5, (sh - WIDGET_H) * 0.5)
}

pub fn hit_test(x: f32, y: f32, cx: f32, cy: f32) -> bool {
    cx >= x && cx <= x + WIDGET_W && cy >= y && cy <= y + WIDGET_H
}

pub fn draw(painter: &mut Painter, x: f32, y: f32, scale: f32, elapsed_secs: f32) {
    let sx = x * scale;
    let sy = y * scale;
    let sw = WIDGET_W * scale;
    let sh = WIDGET_H * scale;
    let corner = CORNER_RADIUS * scale;

    // Drop shadow underneath — gives the window some lift.
    painter.shadow(
        Rect::new(sx, sy, sw, sh),
        corner,
        24.0 * scale,
        Color::from_rgba8(0, 0, 0, 140),
        0.0,
        8.0 * scale,
    );

    // Hue rotation offset advances with time.
    let hue_offset = (elapsed_secs * HUE_RATE_HZ).fract();

    // Vertical bands. Each band's hue = (position_along_x + hue_offset).
    // First band uses the rounded corner radius; middle bands are
    // square; last band uses the corner radius again. Drawing them all
    // with the same radius is fine because the painter SDFs them and
    // adjacent bands' interior corners don't show.
    let band_w = sw / BAND_COUNT as f32;
    for b in 0..BAND_COUNT {
        let t = b as f32 / BAND_COUNT as f32;
        let hue = (t + hue_offset).fract();
        let color = hsl_to_color(hue, 0.65, 0.55, 1.0);
        let bx = sx + b as f32 * band_w;
        // Use a per-band corner radius only at the two outermost bands so
        // the overall panel has rounded corners but the inner edges are
        // butted together. For interior bands we pass 0 corner radius and
        // overdraw a hair to hide subpixel seams.
        let is_edge = b == 0 || b == BAND_COUNT - 1;
        let r = if is_edge { corner } else { 0.0 };
        // Slight horizontal overdraw to kill seams between bands.
        let overdraw = if is_edge { 0.0 } else { 0.5 };
        painter.rect_filled(
            Rect::new(bx - overdraw, sy, band_w + overdraw * 2.0, sh),
            r,
            color,
        );
    }

    // Soft inner border so it reads as a window.
    painter.rect_stroke(
        Rect::new(sx, sy, sw, sh),
        corner,
        1.5 * scale,
        Color::from_rgba8(255, 255, 255, 60),
    );
}

/// HSL → linear-ish Color. Hue in [0, 1), s/l in [0, 1].
fn hsl_to_color(h: f32, s: f32, l: f32, a: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h6 = h * 6.0;
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h6 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c * 0.5;
    Color {
        r: r1 + m,
        g: g1 + m,
        b: b1 + m,
        a,
    }
}

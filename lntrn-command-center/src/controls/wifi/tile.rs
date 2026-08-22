//! Inline-tile drawing and the shared signal-bars icon. The icon is
//! drawn in two places (the inline tile and each row of the expanded
//! view) so it lives here as a shared helper.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

use super::{Wifi, WifiState};

const ICON_SIZE: f32 = 28.0;
const ICON_LEFT_PAD: f32 = 16.0;

pub const TILE_WIDTH: f32 = ICON_LEFT_PAD + ICON_SIZE;

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    wifi: &Wifi,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
    lit: bool,
) {
    if !wifi.is_present() {
        return;
    }
    let icon_size = ICON_SIZE * scale;
    let icon_x = layout.x + ICON_LEFT_PAD * scale;
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;

    let bars = match wifi.state() {
        WifiState::Connected { signal, .. } => signal_to_bars(*signal),
        WifiState::Disconnected => 0,
        WifiState::Off => 0,
    };
    let on_color = if lit {
        Color::from_rgb8(0xc8, 0x86, 0x0a)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff)
    };
    draw_signal_icon_colored(
        painter, icon_x, icon_y, icon_size, icon_size, bars, alpha, on_color,
    );
}

/// Convert a 0-100 signal value into a 0-3 bar count. 0 means "no
/// connection" (we draw a faded placeholder).
pub(super) fn signal_to_bars(signal: u32) -> u32 {
    match signal {
        0 => 0,
        1..=33 => 1,
        34..=66 => 2,
        _ => 3,
    }
}

/// Draw a 3-bar signal indicator at the given rect. `bars` in 0..=3
/// controls how many of the three bars are filled (rest are dim).
pub(super) fn draw_signal_icon(
    painter: &mut Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bars: u32,
    alpha: f32,
) {
    draw_signal_icon_colored(
        painter,
        x,
        y,
        w,
        h,
        bars,
        alpha,
        Color::from_rgb8(0xff, 0xff, 0xff),
    );
}

/// Same as `draw_signal_icon` but with an explicit "on" color so the
/// inline tile can show gold bars when hovered or active.
#[allow(clippy::too_many_arguments)]
fn draw_signal_icon_colored(
    painter: &mut Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bars: u32,
    alpha: f32,
    on_rgb: Color,
) {
    let on = Color {
        r: on_rgb.r,
        g: on_rgb.g,
        b: on_rgb.b,
        a: alpha,
    };
    let off = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.20 * alpha);

    // Three vertical bars, increasing height left → right.
    let gap = w * 0.10;
    let bar_w = (w - gap * 2.0) / 3.0;
    let bottom = y + h * 0.95;
    for (i, frac_h) in [0.45, 0.70, 0.95].iter().enumerate() {
        let bar_x = x + i as f32 * (bar_w + gap);
        let bar_h_actual = h * frac_h;
        let bar_y = bottom - bar_h_actual;
        let color = if (i as u32) < bars { on } else { off };
        painter.rect_filled(
            Rect::new(bar_x, bar_y, bar_w, bar_h_actual),
            bar_w * 0.25,
            color,
        );
    }
}

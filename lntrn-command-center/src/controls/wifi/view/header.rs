//! Header row of the WiFi panel: "Wi-Fi · <ssid>" title on the left,
//! refresh button pinned to the right edge. The button rect is shared
//! by the draw and hit-test paths.

use std::f32::consts::{PI, TAU};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::wifi::{Wifi, WifiState};

use super::{REFRESH_BTN_SIZE, REFRESH_SPIN_RPS, VIEW_HEADER_FONT, VIEW_TOP_PAD};

/// Round refresh button, vertically centred on the header text and
/// flush with the panel's right content edge.
pub(super) fn refresh_button_rect(panel: Rect, panel_top_y: f32, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let size = REFRESH_BTN_SIZE * scale;
    let mid_y = panel_top_y + VIEW_TOP_PAD * scale + VIEW_HEADER_FONT * scale / 2.0;
    Rect::new(panel.x + panel.w - pad - size, mid_y - size / 2.0, size, size)
}

pub(super) fn draw_header(
    painter: &mut Painter,
    text: &mut TextRenderer,
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let header_font = VIEW_HEADER_FONT * scale;
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    let header = match wifi.state() {
        WifiState::Connected { ssid, .. } => format!("Wi-Fi · {}", ssid),
        WifiState::Disconnected => "Wi-Fi · Disconnected".to_string(),
        WifiState::Off => "Wi-Fi · Off".to_string(),
    };
    let header_y = panel_top_y + VIEW_TOP_PAD * scale;
    text.queue(
        &header,
        header_font,
        inner_x,
        header_y,
        white.with_alpha(alpha),
        inner_w,
        surface_w,
        surface_h,
    );

    // Refresh button. Gold + spinning while a manual scan is in flight.
    let btn = refresh_button_rect(panel, panel_top_y, scale);
    let spin = wifi.scan_elapsed().map(|secs| (secs * REFRESH_SPIN_RPS * TAU) % TAU);
    let bg = if spin.is_some() {
        gold.with_alpha(0.25 * alpha)
    } else if wifi.hovered_refresh {
        white.with_alpha(0.18 * alpha)
    } else {
        white.with_alpha(0.08 * alpha)
    };
    painter.rect_filled(btn, btn.w / 2.0, bg);
    let icon_color = if spin.is_some() {
        gold.with_alpha(alpha)
    } else {
        white.with_alpha(0.90 * alpha)
    };
    draw_refresh_icon(painter, btn, spin.unwrap_or(0.0), icon_color);
}

/// Circular arrow: a 270° ring with an arrowhead on its clockwise end,
/// rotated by `rotation` radians.
fn draw_refresh_icon(painter: &mut Painter, rect: Rect, rotation: f32, color: Color) {
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let ring_r = rect.w * 0.22;
    let stroke = (rect.w * 0.075).max(1.5);
    let sweep = 1.5 * PI;
    // Gap sits at the top when unrotated: start up-right, run clockwise.
    let start = (-0.25 * PI + rotation) % TAU;

    // The arc shader strokes the ring at (outer + inner) / 2, where its
    // outer radius includes a `stroke / 2 + 2` quad expansion. Solve for
    // an inner radius that lands the ring centreline exactly on `ring_r`.
    let outer = ring_r + stroke / 2.0;
    let inner = 2.0 * ring_r - (outer + stroke / 2.0 + 2.0);
    painter.arc(cx, cy, outer, start, sweep, stroke, inner, color);

    let end = start + sweep;
    let (radial_x, radial_y) = (end.cos(), end.sin());
    // Clockwise tangent (screen y points down).
    let (tan_x, tan_y) = (-end.sin(), end.cos());
    let px = cx + ring_r * radial_x;
    let py = cy + ring_r * radial_y;
    let head_len = stroke * 2.4;
    let head_half = stroke * 1.7;
    let back_x = px - tan_x * head_len * 0.25;
    let back_y = py - tan_y * head_len * 0.25;
    painter.triangle(
        px + tan_x * head_len * 0.75,
        py + tan_y * head_len * 0.75,
        back_x + radial_x * head_half,
        back_y + radial_y * head_half,
        back_x - radial_x * head_half,
        back_y - radial_y * head_half,
        color,
    );
}

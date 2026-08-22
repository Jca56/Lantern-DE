//! Top-level drawing for the WiFi panel — header row, list, scrollbar,
//! per-row chrome, expanded body. Right-column cards live in [`cards`].

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::wifi::modal::draw_modal;
use crate::controls::wifi::tile::{draw_signal_icon, signal_to_bars};
use crate::controls::wifi::{Wifi, WifiState};

use super::cards::draw_right_column;
use super::layout::{
    band_pill_rect, band_row_top, connect_button_rect, detail_rows, expanded_extra_height,
    has_band_selector, left_col_width, max_scroll, row_list_top_y,
};
use super::{
    BAND_LABEL_FONT, BAND_PILL_FONT, BAND_PILL_H, BAND_ROW_TOP_GAP, EXPAND_BUTTON_FONT,
    EXPAND_DETAIL_FONT, EXPAND_LABEL_W_FRAC, EXPAND_LINE_GAP, EXPAND_PAD_TOP, LIST_BOTTOM_PAD,
    MAX_NETWORK_ROWS, ROW_FONT, ROW_HEIGHT, ROW_LOCK_SIZE, ROW_RIGHT_GAP, ROW_SIGNAL_GAP,
    ROW_SIGNAL_SIZE, VIEW_HEADER_BOTTOM_GAP, VIEW_HEADER_FONT, VIEW_TOP_PAD, VPN_LABEL_FONT,
};

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let header_font = VIEW_HEADER_FONT * scale;
    let header_gap = VIEW_HEADER_BOTTOM_GAP * scale;
    let row_h = ROW_HEIGHT * scale;
    let row_font = ROW_FONT * scale;
    let signal_size = ROW_SIGNAL_SIZE * scale;
    let signal_gap = ROW_SIGNAL_GAP * scale;
    let lock_size = ROW_LOCK_SIZE * scale;
    let right_gap = ROW_RIGHT_GAP * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

    // ── Header: "Wi-Fi" + connected SSID ──
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

    // VPN ON/OFF indicator on the far right of the header row. Only
    // rendered when the `mullvad` CLI is present; otherwise the slot is
    // simply empty.
    if let Some(connected) = wifi.vpn_connected {
        let vpn_label = if connected { "VPN: ON" } else { "VPN: OFF" };
        let vpn_color = if connected {
            Color::from_rgb8(0x4c, 0xd9, 0x64).with_alpha(alpha)
        } else {
            Color::from_rgb8(0xff, 0x4d, 0x4d).with_alpha(alpha)
        };
        let vpn_font = VPN_LABEL_FONT * scale;
        let lbl_w = text.measure_width(vpn_label, vpn_font);
        let lbl_x = panel.x + panel.w - pad - lbl_w;
        // Vertically center the VPN label against the header text's
        // visual mid-line (header_y is the text top edge).
        let lbl_y = header_y + (header_font - vpn_font) / 2.0;
        text.queue(
            vpn_label, vpn_font, lbl_x, lbl_y, vpn_color, lbl_w, surface_w, surface_h,
        );
    }

    // ── Network rows ──
    let list_top = header_y + header_font + header_gap;
    let _ = list_top; // keep symmetry with the layout helper above

    if wifi.networks().is_empty() {
        let msg_y = row_list_top_y(panel_top_y, scale);
        text.queue(
            "Scanning…",
            row_font,
            inner_x,
            msg_y,
            muted,
            inner_w,
            surface_w,
            surface_h,
        );
        return msg_y + row_font;
    }

    // List clip: from row_list_top_y down to the bottom of the panel
    // (minus a small bottom pad). Rows that scroll out of this rect
    // get clipped rather than bleeding into chrome below.
    let list_top = row_list_top_y(panel_top_y, scale);
    let list_bottom = panel.y + panel.h - LIST_BOTTOM_PAD * scale;
    let viewport_h = (list_bottom - list_top).max(0.0);
    let max = max_scroll(wifi, viewport_h, scale);
    let scroll_clamped = wifi.scroll.clamp(0.0, max);
    let scroll_px = scroll_clamped * scale;
    let list_clip = Rect::new(panel.x, list_top, panel.w, viewport_h);
    painter.push_clip(list_clip);
    text.push_clip([list_clip.x, list_clip.y, list_clip.w, list_clip.h]);

    let mut row_y = list_top - scroll_px;
    for (i, net) in wifi.networks().iter().take(MAX_NETWORK_ROWS).enumerate() {
        let is_expanded = wifi.expanded_ssid.as_deref() == Some(net.ssid.as_str());
        let extra_h = if is_expanded {
            expanded_extra_height(net, scale)
        } else {
            0.0
        };
        let total_h = row_h + extra_h;
        // The full container (header + expanded body) — used for
        // bg fill so the expanded body and header share a plate.
        let container_rect = Rect::new(inner_x, row_y, inner_w, total_h);
        // Header-only rect — used for the gold tint on the in-use
        // row (we don't tint the whole expanded panel).
        let row_rect = Rect::new(inner_x, row_y, inner_w, row_h);

        // Subtle row-stripe for readability + brighter highlight on the
        // currently-connected row.
        let is_hovered = wifi.hovered_ssid.as_deref() == Some(net.ssid.as_str());
        if is_expanded {
            // Darker grey plate behind the expanded card so the BSSID
            // grid + Connect button read clearly against the panel bg.
            painter.rect_filled(
                container_rect,
                10.0 * scale,
                Color::rgba(0.0, 0.0, 0.0, 0.35 * alpha),
            );
        } else if i % 2 == 0 {
            painter.rect_filled(row_rect, 8.0 * scale, white.with_alpha(0.04 * alpha));
        }
        if net.in_use {
            painter.rect_filled(row_rect, 8.0 * scale, gold.with_alpha(0.18 * alpha));
        }
        // Hover highlight sits on top of the stripe so it reads
        // consistently across odd/even rows. Skipped for the in-use
        // row since the gold tint already signals selection clearly,
        // and skipped while expanded since the container plate
        // already differentiates the row.
        if is_hovered && !net.in_use && !is_expanded {
            painter.rect_filled(row_rect, 8.0 * scale, white.with_alpha(0.10 * alpha));
        }

        // Signal % to the left of the bars icon. Sized smaller than
        // the SSID so it reads as metadata, not a focal element.
        let pct_str = format!("{}%", net.signal);
        let pct_font = row_font * 0.75;
        let pct_w = text.measure_width(&pct_str, pct_font);
        // Pass a slightly padded max_width so the trailing "%" never
        // wraps to a second line when the measured width sits right on
        // the glyph boundary.
        let pct_box = pct_w + 6.0 * scale;

        let pct_x = inner_x + signal_gap;
        text.queue(
            &pct_str,
            pct_font,
            pct_x,
            row_y + (row_h - pct_font) / 2.0,
            muted,
            pct_box,
            surface_w,
            surface_h,
        );

        // Signal icon, just to the right of the percent (use the
        // padded box width so spacing stays consistent).
        let icon_x = pct_x + pct_box + signal_gap * 0.6;
        let icon_y = row_y + (row_h - signal_size) / 2.0;
        let bars = signal_to_bars(net.signal);
        draw_signal_icon(
            painter,
            icon_x,
            icon_y,
            signal_size,
            signal_size,
            bars,
            alpha,
        );

        // SSID label. The connected network gets a larger, gold label
        // so it stands out at a glance even before reading the badge.
        let label_x = icon_x + signal_size + signal_gap;
        let (ssid_font, ssid_color) = if net.in_use {
            (row_font * 1.20, gold.with_alpha(alpha))
        } else {
            (row_font, white.with_alpha(0.86 * alpha))
        };
        let label_y = row_y + (row_h - ssid_font) / 2.0;
        text.queue(
            &net.ssid,
            ssid_font,
            label_x,
            label_y,
            ssid_color,
            inner_w * 0.7,
            surface_w,
            surface_h,
        );

        // Right side, walking R→L: lock first (right-most), then
        // Saved/Connected/Connecting status text to its left.
        let mut right_x = inner_x + inner_w - right_gap;
        if !net.security.is_empty() && net.security != "--" {
            let lock_x = right_x - lock_size;
            let lock_y = row_y + (row_h - lock_size) / 2.0;
            draw_lock(painter, lock_x, lock_y, lock_size, lock_size, alpha);
            right_x = lock_x - right_gap;
        }
        // Status text: "Connecting…" wins over Connected/Saved while
        // a connect attempt is in flight to that ssid.
        let connecting_now = wifi.is_connecting_to(&net.ssid);
        if connecting_now {
            let s = "Connecting…";
            let f = row_font * 0.8;
            let w = text.measure_width(s, f);
            right_x -= w;
            text.queue(
                s,
                f,
                right_x,
                row_y + (row_h - f) / 2.0,
                gold.with_alpha(alpha),
                w,
                surface_w,
                surface_h,
            );
        } else if net.in_use {
            let s = "Connected";
            let f = row_font * 0.8;
            let w = text.measure_width(s, f);
            right_x -= w;
            text.queue(
                s,
                f,
                right_x,
                row_y + (row_h - f) / 2.0,
                gold.with_alpha(alpha),
                w,
                surface_w,
                surface_h,
            );
        } else if net.saved {
            let s = "Saved";
            let f = row_font * 0.7;
            let w = text.measure_width(s, f);
            right_x -= w;
            text.queue(
                s,
                f,
                right_x,
                row_y + (row_h - f) / 2.0,
                muted,
                w,
                surface_w,
                surface_h,
            );
        }
        let _ = right_x;

        // Expanded section: details list + Connect button (left column)
        // and Top-BSSID list (right column).
        if is_expanded {
            let body_top = row_y + row_h;
            let detail_font = EXPAND_DETAIL_FONT * scale;
            let line_h = detail_font + EXPAND_LINE_GAP * scale;
            let left_w = left_col_width(inner_w);
            let label_w = left_w * EXPAND_LABEL_W_FRAC;
            let mut dy = body_top + EXPAND_PAD_TOP * scale;
            let label_x = inner_x + 16.0 * scale;
            let value_x = label_x + label_w;

            for (label, value) in detail_rows(net) {
                text.queue(
                    label,
                    detail_font,
                    label_x,
                    dy,
                    muted,
                    label_w,
                    surface_w,
                    surface_h,
                );
                text.queue(
                    &value,
                    detail_font,
                    value_x,
                    dy,
                    white.with_alpha(0.92 * alpha),
                    left_w - (value_x - inner_x) - 16.0 * scale,
                    surface_w,
                    surface_h,
                );
                dy += line_h;
            }

            // ── Right column: top BSSIDs ──
            draw_right_column(
                painter, text, net, inner_x, inner_w, body_top, scale, alpha, surface_w, surface_h,
            );

            // Band-selector pills (only when 2+ bands available).
            if has_band_selector(net) {
                let pill_label_font = BAND_LABEL_FONT * scale;
                let pill_font = BAND_PILL_FONT * scale;
                let pill_radius = (BAND_PILL_H * scale) * 0.5;
                let row_top = band_row_top(net, body_top, scale) + BAND_ROW_TOP_GAP * scale;
                let pill_h = BAND_PILL_H * scale;
                // "Band" label left of the pills.
                text.queue(
                    "Band",
                    pill_label_font,
                    label_x,
                    row_top + (pill_h - pill_label_font) / 2.0,
                    muted,
                    label_w,
                    surface_w,
                    surface_h,
                );
                for entry in &net.bands {
                    let Some(pill) =
                        band_pill_rect(net, entry.band, inner_x, inner_w, body_top, scale)
                    else {
                        continue;
                    };
                    let selected = entry.band == net.selected_band;
                    let bg = if selected {
                        gold.with_alpha(0.85 * alpha)
                    } else {
                        white.with_alpha(0.08 * alpha)
                    };
                    painter.rect_filled(pill, pill_radius, bg);
                    if !selected {
                        painter.rect_stroke_sdf(
                            pill,
                            pill_radius,
                            1.0 * scale,
                            white.with_alpha(0.18 * alpha),
                        );
                    }
                    let lbl = entry.band.short_label();
                    let lw = text.measure_width(lbl, pill_font);
                    let tx = pill.x + (pill.w - lw) / 2.0;
                    let ty = pill.y + (pill.h - pill_font) / 2.0;
                    let tcol = if selected {
                        Color::rgba(0.0, 0.0, 0.0, alpha)
                    } else {
                        white.with_alpha(0.90 * alpha)
                    };
                    text.queue(lbl, pill_font, tx, ty, tcol, pill.w, surface_w, surface_h);
                }
            }

            // Connect button.
            let btn = connect_button_rect(net, inner_x, inner_w, body_top, scale);
            let connecting_now = wifi.is_connecting_to(&net.ssid);
            let label = if connecting_now {
                "Connecting…"
            } else if net.in_use {
                "Connected"
            } else {
                "Connect"
            };
            let btn_font = EXPAND_BUTTON_FONT * scale;
            let btn_radius = 10.0 * scale;
            let bg = if connecting_now {
                gold.with_alpha(0.35 * alpha)
            } else if net.in_use {
                gold.with_alpha(0.30 * alpha)
            } else {
                gold.with_alpha(0.95 * alpha)
            };
            painter.rect_filled(btn, btn_radius, bg);
            let lw = text.measure_width(label, btn_font);
            text.queue(
                label,
                btn_font,
                btn.x + (btn.w - lw) / 2.0,
                btn.y + (btn.h - btn_font) / 2.0,
                Color::rgba(0.0, 0.0, 0.0, alpha),
                btn.w,
                surface_w,
                surface_h,
            );
        }

        row_y += row_h + extra_h;
    }

    if let Some(err) = wifi.last_error() {
        let err_y = row_y + row_font * 0.5;
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            row_font * 0.85,
            inner_x,
            err_y,
            red,
            inner_w,
            surface_w,
            surface_h,
        );
    }

    painter.pop_clip();
    text.pop_clip();

    // Scrollbar — thin track on the right side of the panel.
    if max > 0.0 {
        let track_w = 4.0 * scale;
        let track_x = panel.x + panel.w - track_w - 6.0 * scale;
        painter.rect_filled(
            Rect::new(track_x, list_top, track_w, viewport_h),
            track_w / 2.0,
            white.with_alpha(0.06 * alpha),
        );
        let thumb_h = (viewport_h * viewport_h / (viewport_h + max * scale)).max(24.0 * scale);
        let thumb_y =
            list_top + (viewport_h - thumb_h) * (scroll_px / (max * scale)).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::new(track_x, thumb_y, track_w, thumb_h),
            track_w / 2.0,
            white.with_alpha(0.30 * alpha),
        );
    }

    let bottom = list_bottom;

    // Draw the password prompt on top of the network list when active.
    // Layer 2 (the modal tier) so the modal's painter rects can occlude
    // already-queued text from the network list — see
    // lntrn-render/TEXT_OCCLUSION_FIX.
    if let Some(prompt) = &wifi.prompt {
        painter.set_layer(2);
        text.set_layer(2);
        draw_modal(
            painter,
            text,
            prompt,
            panel,
            panel_top_y,
            scale,
            alpha,
            wifi.last_error(),
            surface_w,
            surface_h,
        );
    }

    bottom
}

/// Tiny lock icon — body rect + shackle arch. Pure shapes.
pub(super) fn draw_lock(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.65 * alpha);
    // Body: bottom 60 % of the box, full width.
    let body_y = y + h * 0.40;
    let body_h = h * 0.60;
    painter.rect_filled(Rect::new(x, body_y, w, body_h), w * 0.16, color);
    // Shackle: U-shape on top, drawn as a stroked rounded rect with the
    // bottom hidden behind the body. We approximate by drawing a rect
    // outline above the body.
    let shackle_x = x + w * 0.18;
    let shackle_w = w * 0.64;
    let shackle_h = h * 0.50;
    painter.rect_stroke_sdf(
        Rect::new(shackle_x, y, shackle_w, shackle_h),
        shackle_w * 0.45,
        w * 0.13,
        color,
    );
}

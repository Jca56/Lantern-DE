//! Click-expand panel drawing, hit-testing, and layout for the WiFi
//! control. Pure-render code: all backend state lives on [`Wifi`],
//! which this module only reads from. (The inline tile + shared
//! signal icon live in `super::tile`.)

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::modal::draw_modal;
use super::tile::{draw_signal_icon, signal_to_bars};
use super::{Band, Network, Wifi, WifiState};

// ── Click-expand view ───────────────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 24.0;
const VIEW_HEADER_FONT: f32 = 22.0;
const VIEW_HEADER_BOTTOM_GAP: f32 = 12.0;
const ROW_HEIGHT: f32 = 56.0;
const ROW_FONT: f32 = 22.0;
const ROW_SIGNAL_SIZE: f32 = 28.0;
const ROW_SIGNAL_GAP: f32 = 16.0;
const ROW_LOCK_SIZE: f32 = 20.0;
const ROW_RIGHT_GAP: f32 = 12.0;
const MAX_NETWORK_ROWS: usize = 6;
/// Logical px reserved for the expanded detail+button area beneath
/// the row header. One detail line per displayed property.
const EXPAND_PAD_TOP: f32 = 8.0;
const EXPAND_PAD_BOTTOM: f32 = 12.0;
const EXPAND_LINE_GAP: f32 = 4.0;
const EXPAND_DETAIL_FONT: f32 = 16.0;
const EXPAND_LABEL_W_FRAC: f32 = 0.28;
const EXPAND_BUTTON_TOP_GAP: f32 = 12.0;
const EXPAND_BUTTON_H: f32 = 38.0;
const EXPAND_BUTTON_FONT: f32 = 18.0;
const EXPAND_BUTTON_W: f32 = 140.0;
/// Band-selector pills sit between the details list and the Connect
/// button. Shown only when an SSID is advertised on multiple bands.
const BAND_ROW_TOP_GAP: f32 = 12.0;
const BAND_PILL_H: f32 = 32.0;
const BAND_PILL_W: f32 = 64.0;
const BAND_PILL_GAP: f32 = 8.0;
const BAND_PILL_FONT: f32 = 16.0;
const BAND_LABEL_FONT: f32 = 16.0;

/// Y-coordinate (physical px) of the first network row.
fn row_list_top_y(panel_top_y: f32, scale: f32) -> f32 {
    panel_top_y + VIEW_TOP_PAD * scale + VIEW_HEADER_FONT * scale + VIEW_HEADER_BOTTOM_GAP * scale
}

/// All non-empty detail rows for an expanded network, as
/// (label, value) pairs. Pulls per-band fields from the currently
/// selected band so the panel updates when the user flips pills.
fn detail_rows(net: &Network) -> Vec<(&'static str, String)> {
    // The entry matching `selected_band`, falling back to the strongest
    // (which is `bands[0]` after `scan_networks` finalization).
    let entry = net
        .bands
        .iter()
        .find(|b| b.band == net.selected_band)
        .or_else(|| net.bands.first());

    let mut rows: Vec<(&'static str, String)> = Vec::new();
    rows.push((
        "Status",
        if net.in_use {
            "Connected".into()
        } else if net.saved {
            "Saved".into()
        } else {
            "Not saved".into()
        },
    ));
    let signal = entry.map(|e| e.signal).unwrap_or(net.signal);
    rows.push(("Signal", format!("{}%", signal)));
    let security = if net.security.is_empty() || net.security == "--" {
        "Open".to_string()
    } else {
        net.security.clone()
    };
    rows.push(("Security", security));
    if let Some(e) = entry {
        if !e.bssid.is_empty() {
            rows.push(("BSSID", e.bssid.clone()));
        }
    }
    if !net.mode.is_empty() {
        rows.push(("Mode", net.mode.clone()));
    }
    if let Some(e) = entry {
        if !e.channel.is_empty() {
            rows.push(("Channel", e.channel.clone()));
        }
        if !e.frequency.is_empty() {
            rows.push((
                "Frequency",
                format!("{} ({})", e.frequency, e.band.long_label()),
            ));
        }
        if !e.rate.is_empty() {
            rows.push(("Rate", e.rate.clone()));
        }
    }
    rows
}

/// True when the network has more than one band and the user should
/// see a band-selector pill row in the expanded view.
fn has_band_selector(net: &Network) -> bool {
    net.bands.len() > 1
}

/// Physical-px height contribution of the band-selector pill row.
/// Zero when there's no choice to make.
fn band_row_height(net: &Network, scale: f32) -> f32 {
    if has_band_selector(net) {
        BAND_ROW_TOP_GAP * scale + BAND_PILL_H * scale
    } else {
        0.0
    }
}

/// Total expanded-section height in physical px (header excluded —
/// this is purely the part that's added when the row opens).
fn expanded_extra_height(net: &Network, scale: f32) -> f32 {
    let n = detail_rows(net).len() as f32;
    let line_h = EXPAND_DETAIL_FONT * scale + EXPAND_LINE_GAP * scale;
    EXPAND_PAD_TOP * scale
        + n * line_h
        + band_row_height(net, scale)
        + EXPAND_BUTTON_TOP_GAP * scale
        + EXPAND_BUTTON_H * scale
        + EXPAND_PAD_BOTTOM * scale
}

/// What was clicked inside the WiFi network list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkHit {
    /// The header part of a row (any area outside the Connect button
    /// in the expanded section). Caller toggles the expanded ssid.
    Row(String),
    /// The Connect / Disconnect button inside the expanded section.
    ConnectButton(String),
    /// A band-selector pill inside the expanded section.
    BandPill(String, Band),
}

/// Hit-test a click against the network list. Walks rows top-to-
/// bottom honoring per-row variable height so the expanded section
/// doesn't shadow rows below it.
pub fn hit_test_network(
    wifi: &Wifi,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<NetworkHit> {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }

    let header_h = ROW_HEIGHT * scale;
    let mut row_y = row_list_top_y(panel_top_y, scale);
    for net in wifi.networks().iter().take(MAX_NETWORK_ROWS) {
        let is_expanded = wifi.expanded_ssid.as_deref() == Some(net.ssid.as_str());
        let extra = if is_expanded { expanded_extra_height(net, scale) } else { 0.0 };

        // Header area first.
        if y >= row_y && y <= row_y + header_h {
            return Some(NetworkHit::Row(net.ssid.clone()));
        }
        // Expanded area: check pill / button hits, otherwise treat as
        // a Row click so clicking inside the panel away from any
        // control doesn't leak through to the next row.
        if is_expanded {
            let body_top = row_y + header_h;
            let body_bottom = row_y + header_h + extra;
            if y >= body_top && y <= body_bottom {
                if has_band_selector(net) {
                    for entry in &net.bands {
                        if let Some(pill) =
                            band_pill_rect(net, entry.band, inner_x, body_top, scale)
                        {
                            if x >= pill.x
                                && x <= pill.x + pill.w
                                && y >= pill.y
                                && y <= pill.y + pill.h
                            {
                                return Some(NetworkHit::BandPill(
                                    net.ssid.clone(),
                                    entry.band,
                                ));
                            }
                        }
                    }
                }
                let btn = connect_button_rect(net, inner_x, inner_w, body_top, scale);
                if x >= btn.x && x <= btn.x + btn.w && y >= btn.y && y <= btn.y + btn.h {
                    return Some(NetworkHit::ConnectButton(net.ssid.clone()));
                }
                return Some(NetworkHit::Row(net.ssid.clone()));
            }
        }

        row_y += header_h + extra;
    }
    None
}

/// Y-coordinate of the top of the band-selector pill row (the row
/// itself starts after `BAND_ROW_TOP_GAP`). Used by both draw + hit
/// test so they share one truth.
fn band_row_top(net: &Network, body_top: f32, scale: f32) -> f32 {
    let n = detail_rows(net).len() as f32;
    let line_h = EXPAND_DETAIL_FONT * scale + EXPAND_LINE_GAP * scale;
    body_top + EXPAND_PAD_TOP * scale + n * line_h
}

/// Rect of a specific band pill within an expanded row body. Pills are
/// laid out left-to-right in the order they appear in `net.bands`
/// (strongest first), starting after a small "Band" label.
fn band_pill_rect(
    net: &Network,
    band: Band,
    inner_x: f32,
    body_top: f32,
    scale: f32,
) -> Option<Rect> {
    if !has_band_selector(net) {
        return None;
    }
    let idx = net.bands.iter().position(|b| b.band == band)?;
    let row_top = band_row_top(net, body_top, scale) + BAND_ROW_TOP_GAP * scale;
    let pill_w = BAND_PILL_W * scale;
    let pill_h = BAND_PILL_H * scale;
    let gap = BAND_PILL_GAP * scale;
    // Label column eats the same indent as the detail rows so the
    // pills line up under the values.
    let pills_x = inner_x + 16.0 * scale + 80.0 * scale;
    let x = pills_x + idx as f32 * (pill_w + gap);
    Some(Rect::new(x, row_top, pill_w, pill_h))
}

/// Compute the Connect button rect for an expanded row body that
/// starts at `body_top` (just under the row header). Mirrors the
/// layout in `draw_view`'s expanded-section code.
fn connect_button_rect(net: &Network, inner_x: f32, inner_w: f32, body_top: f32, scale: f32) -> Rect {
    let after_details = band_row_top(net, body_top, scale);
    let after_bands = after_details + band_row_height(net, scale);
    let btn_y = after_bands + EXPAND_BUTTON_TOP_GAP * scale;
    let btn_w = EXPAND_BUTTON_W * scale;
    let btn_h = EXPAND_BUTTON_H * scale;
    let btn_x = inner_x + inner_w - btn_w - EXPAND_PAD_TOP * scale;
    Rect::new(btn_x, btn_y, btn_w, btn_h)
}

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

    let mut row_y = row_list_top_y(panel_top_y, scale);
    for (i, net) in wifi.networks().iter().take(MAX_NETWORK_ROWS).enumerate() {
        let is_expanded = wifi.expanded_ssid.as_deref() == Some(net.ssid.as_str());
        let extra_h = if is_expanded { expanded_extra_height(net, scale) } else { 0.0 };
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
            painter.rect_filled(
                container_rect,
                10.0 * scale,
                white.with_alpha(0.06 * alpha),
            );
        } else if i % 2 == 0 {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                white.with_alpha(0.04 * alpha),
            );
        }
        if net.in_use {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                gold.with_alpha(0.18 * alpha),
            );
        }
        // Hover highlight sits on top of the stripe so it reads
        // consistently across odd/even rows. Skipped for the in-use
        // row since the gold tint already signals selection clearly,
        // and skipped while expanded since the container plate
        // already differentiates the row.
        if is_hovered && !net.in_use && !is_expanded {
            painter.rect_filled(
                row_rect,
                8.0 * scale,
                white.with_alpha(0.10 * alpha),
            );
        }

        // Signal % to the left of the bars icon. Sized smaller than
        // the SSID so it reads as metadata, not a focal element.
        let pct_str = format!("{}%", net.signal);
        let pct_font = row_font * 0.75;
        let pct_w = text.measure_width(&pct_str, pct_font);

        let pct_x = inner_x + signal_gap;
        text.queue(
            &pct_str,
            pct_font,
            pct_x,
            row_y + (row_h - pct_font) / 2.0,
            muted,
            pct_w,
            surface_w,
            surface_h,
        );

        // Signal icon, just to the right of the percent.
        let icon_x = pct_x + pct_w + signal_gap * 0.6;
        let icon_y = row_y + (row_h - signal_size) / 2.0;
        let bars = signal_to_bars(net.signal);
        draw_signal_icon(painter, icon_x, icon_y, signal_size, signal_size, bars, alpha);

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

        // Expanded section: details list + Connect button.
        if is_expanded {
            let body_top = row_y + row_h;
            let detail_font = EXPAND_DETAIL_FONT * scale;
            let line_h = detail_font + EXPAND_LINE_GAP * scale;
            let label_w = inner_w * EXPAND_LABEL_W_FRAC;
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
                    inner_w - (value_x - inner_x) - 16.0 * scale,
                    surface_w,
                    surface_h,
                );
                dy += line_h;
            }

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
                    let Some(pill) = band_pill_rect(net, entry.band, inner_x, body_top, scale)
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

    let bottom = row_list_top_y(panel_top_y, scale)
        + wifi.networks().len().min(MAX_NETWORK_ROWS) as f32 * row_h;

    // Draw the password prompt on top of the network list when active.
    // Layer 1 so the modal's painter rects can occlude already-queued
    // text from the network list — see lntrn-render/TEXT_OCCLUSION_FIX.
    if let Some(prompt) = &wifi.prompt {
        painter.set_layer(1);
        text.set_layer(1);
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
fn draw_lock(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.65 * alpha);
    // Body: bottom 60 % of the box, full width.
    let body_y = y + h * 0.40;
    let body_h = h * 0.60;
    painter.rect_filled(
        Rect::new(x, body_y, w, body_h),
        w * 0.16,
        color,
    );
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

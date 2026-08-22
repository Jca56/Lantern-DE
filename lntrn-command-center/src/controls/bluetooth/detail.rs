//! Expanded device-detail block — the panel that drops down when a device
//! row is tapped open. Renders the `(label, value)` detail lines
//! (address, type, battery, profiles, …) plus the Connect/Disconnect/Pair
//! and Send-file action buttons.
//!
//! The geometry helpers (`expanded_extra_height`, `connect_button_rect`,
//! `send_button_rect_expanded`) are shared with the parent `render`
//! module's hit-tester so layout and click targets stay in sync.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::render::{body_font, draw_pill_button, ROW_INNER_PAD, ROW_RIGHT_GAP};
use super::{Bluetooth, Device};

/// Padding inside the expanded panel.
const EXPAND_PAD_TOP: f32 = 10.0;
const EXPAND_PAD_BOTTOM: f32 = 14.0;
const EXPAND_LINE_GAP: f32 = 6.0;
const EXPAND_LABEL_W_FRAC: f32 = 0.30;
const EXPAND_BUTTON_TOP_GAP: f32 = 14.0;
const EXPAND_BUTTON_H: f32 = 44.0;
const EXPAND_BUTTON_W: f32 = 180.0;
const EXPAND_BUTTON_GAP: f32 = 12.0;

fn label_font(text_size: f32, scale: f32) -> f32 {
    (text_size.max(12.0)) * 0.82 * scale
}

/// Visible detail lines for `dev` in the order they're rendered.
/// Returns `(label, value)` pairs; only lines with a value are included.
fn detail_lines(dev: &Device) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    out.push(("Address", dev.mac.clone()));
    if !dev.alias.is_empty() && dev.alias != dev.name {
        out.push(("Alias", dev.alias.clone()));
    }
    if !dev.address_type.is_empty() {
        out.push(("Type", dev.address_type.clone()));
    }
    if !dev.icon.is_empty() {
        out.push(("Kind", dev.icon.clone()));
    }
    if !dev.class.is_empty() {
        out.push(("Class", dev.class.clone()));
    }
    if let Some(pct) = dev.battery_percent {
        out.push(("Battery", format!("{}%", pct)));
    }
    if let Some(r) = dev.rssi {
        out.push(("Signal", format!("{} dBm", r)));
    }
    out.push((
        "Status",
        format!(
            "{}{}{}{}",
            if dev.paired { "paired" } else { "unpaired" },
            if dev.connected { " · connected" } else { "" },
            if dev.trusted { " · trusted" } else { "" },
            if dev.blocked { " · blocked" } else { "" },
        ),
    ));
    if !dev.uuids.is_empty() {
        // Cap the profile list so a chatty device doesn't blow the
        // panel up — the user can still get the gist.
        let preview: Vec<&String> = dev.uuids.iter().take(8).collect();
        let mut joined = preview
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if dev.uuids.len() > 8 {
            joined.push_str(&format!(" + {} more", dev.uuids.len() - 8));
        }
        out.push(("Profiles", joined));
    }
    out
}

/// Height (in physical px) of the expanded detail block for a single
/// device. Shared by the renderer and the hit-tester so geometry stays
/// in sync.
pub(super) fn expanded_extra_height(dev: &Device, text_size: f32, scale: f32) -> f32 {
    let lines = detail_lines(dev);
    let body = body_font(text_size, scale);
    let gap = EXPAND_LINE_GAP * scale;
    let detail_h = lines.len() as f32 * (body + gap) - gap.max(0.0);
    let mut h = EXPAND_PAD_TOP * scale + detail_h.max(0.0);
    h += EXPAND_BUTTON_TOP_GAP * scale + EXPAND_BUTTON_H * scale;
    h += EXPAND_PAD_BOTTOM * scale;
    h
}

/// Compute the Connect button rect inside `dev`'s expanded panel.
pub(super) fn connect_button_rect(
    inner_x: f32,
    _inner_w: f32,
    expanded_top: f32,
    dev: &Device,
    text_size: f32,
    scale: f32,
) -> Rect {
    let extra = expanded_extra_height(dev, text_size, scale);
    let btn_h = EXPAND_BUTTON_H * scale;
    let btn_w = EXPAND_BUTTON_W * scale;
    let btn_y = expanded_top + extra - EXPAND_PAD_BOTTOM * scale - btn_h;
    Rect::new(inner_x, btn_y, btn_w, btn_h)
}

/// Compute the Send button rect inside `dev`'s expanded panel. `None`
/// when the device doesn't expose an OBEX push profile.
pub(super) fn send_button_rect_expanded(
    inner_x: f32,
    inner_w: f32,
    expanded_top: f32,
    dev: &Device,
    text_size: f32,
    scale: f32,
) -> Option<Rect> {
    if !dev.supports_file_transfer() {
        return None;
    }
    let connect = connect_button_rect(inner_x, inner_w, expanded_top, dev, text_size, scale);
    let btn_w = EXPAND_BUTTON_W * scale;
    let gap = EXPAND_BUTTON_GAP * scale;
    Some(Rect::new(
        connect.x + connect.w + gap,
        connect.y,
        btn_w,
        connect.h,
    ))
}

/// Draw the expanded detail block for `dev` at `top_y`. Returns its
/// height so the caller can advance its cursor.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_expanded(
    painter: &mut Painter,
    text: &mut TextRenderer,
    dev: &Device,
    bt: &Bluetooth,
    inner_x: f32,
    inner_w: f32,
    top_y: f32,
    scale: f32,
    alpha: f32,
    text_size: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let body = body_font(text_size, scale);
    let lbl_font = label_font(text_size, scale);
    let pad_l = ROW_INNER_PAD * scale;
    let pad_t = EXPAND_PAD_TOP * scale;
    let gap = EXPAND_LINE_GAP * scale;
    let label_w = inner_w * EXPAND_LABEL_W_FRAC;

    let mut cy = top_y + pad_t;
    for (label, value) in detail_lines(dev) {
        text.queue(
            label,
            lbl_font,
            inner_x + pad_l,
            cy + (body - lbl_font) / 2.0,
            muted,
            label_w - pad_l,
            surface_w,
            surface_h,
        );
        let value_x = inner_x + pad_l + label_w;
        text.queue(
            &value,
            body,
            value_x,
            cy,
            white.with_alpha(0.92 * alpha),
            inner_w - (label_w + pad_l + ROW_RIGHT_GAP * scale),
            surface_w,
            surface_h,
        );
        cy += body + gap;
    }

    // ── Action buttons ──
    let expanded_top = top_y;
    let connect = connect_button_rect(inner_x, inner_w, expanded_top, dev, text_size, scale);
    let connect_label = if !dev.paired {
        "Pair"
    } else if dev.connected {
        "Disconnect"
    } else {
        "Connect"
    };
    draw_pill_button(
        painter,
        text,
        connect,
        connect_label,
        body,
        gold,
        alpha,
        scale,
        surface_w,
        surface_h,
    );

    if let Some(send) =
        send_button_rect_expanded(inner_x, inner_w, expanded_top, dev, text_size, scale)
    {
        let label = if bt.send_state.contains_key(&dev.mac) {
            "Sending…"
        } else {
            "Send file"
        };
        draw_pill_button(
            painter, text, send, label, body, gold, alpha, scale, surface_w, surface_h,
        );
    }

    expanded_extra_height(dev, text_size, scale)
}

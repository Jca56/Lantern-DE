//! Bluetooth click-expand view — drawing for the full-content view shown
//! when the user opens the BT tile: a pinned header + toggles row, then
//! the scrolling list of paired/available device sections.
//!
//! The geometry it shares with hit-testing lives in `super::layout`, the
//! hit-testing itself in `super::hit`. The inline controls-row glyph
//! lives in `super::glyph`; the expanded device-detail block in
//! `super::detail`; inline request strips (pair / file Accept-Reject) in
//! `super::prompt`.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::detail;
use super::layout::{
    body_font, header_row_y, list_viewport, row_extra_height, toggle_rect, toggles_row_layout,
    toggles_row_y, ListViewport, MAX_SECTION_ROWS, ROW_HEIGHT, ROW_INNER_PAD, ROW_RIGHT_GAP,
    SECTION_GAP, SECTION_HEADER_BOTTOM_GAP, SECTION_HEADER_FONT, TOGGLES_ROW_FONT,
    VIEW_HEADER_FONT,
};
use super::prompt::{self, row_prompt, RowPrompt};
use super::{Bluetooth, Device, SendStatus};

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    text_size: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;

    let header_font = VIEW_HEADER_FONT * scale;
    // Body rows respect the user's Text Size setting; header stays at
    // its own constant so the "Bluetooth" title doesn't grow huge.
    let row_font = body_font(text_size, scale);

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);

    // ── Header: "Bluetooth" + power toggle on the right ──
    let header_y = header_row_y(panel_top_y, scale);
    text.queue(
        "Bluetooth",
        header_font,
        inner_x,
        header_y,
        white.with_alpha(alpha),
        inner_w,
        surface_w,
        surface_h,
    );
    let t_rect = toggle_rect(panel, panel_top_y, scale);
    super::toggle::draw_toggle(painter, t_rect, bt.is_powered(), alpha, scale);

    // Power off → bail with a simple message.
    if !bt.is_powered() {
        let msg_y = toggles_row_y(panel_top_y, scale);
        text.queue(
            "Bluetooth is off",
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

    // ── Toggles row (Discoverable / Scan) ──
    draw_toggles_row(
        painter,
        text,
        bt,
        panel,
        panel_top_y,
        scale,
        alpha,
        surface_w,
        surface_h,
    );

    // ── Device sections: everything from here down scrolls ──
    // Clipped to the list's viewport so rows that scroll out of it
    // don't bleed into the toggles row above or past the panel's edge.
    let vp = list_viewport(bt, panel, panel_top_y, text_size, scale);
    let list_clip = Rect::new(panel.x, vp.top, panel.w, vp.height);
    painter.push_clip(list_clip);
    text.push_clip([list_clip.x, list_clip.y, list_clip.w, list_clip.h]);

    let mut cy = vp.top - vp.scroll;
    let paired = bt.paired_devices();
    let unpaired = bt.unpaired_devices();

    if !paired.is_empty() {
        cy = draw_section(
            painter, text, bt, "Paired", &paired, &vp, inner_x, inner_w, cy, scale, alpha,
            text_size, surface_w, surface_h,
        );
        cy += SECTION_GAP * scale;
    }

    if !unpaired.is_empty() {
        let header = if bt.is_scanning() {
            "Available"
        } else {
            "Recently seen"
        };
        cy = draw_section(
            painter, text, bt, header, &unpaired, &vp, inner_x, inner_w, cy, scale, alpha,
            text_size, surface_w, surface_h,
        );
        // Note how many discovered-but-unnamed devices we hid, so a device
        // whose name BlueZ hasn't resolved yet isn't a silent mystery.
        let hidden = bt.hidden_unpaired_count();
        if hidden > 0 {
            let msg = format!(
                "+{hidden} unnamed device{} hidden",
                if hidden == 1 { "" } else { "s" }
            );
            text.queue(
                &msg,
                row_font * 0.8,
                inner_x,
                cy + row_font * 0.4,
                muted,
                inner_w,
                surface_w,
                surface_h,
            );
            cy += row_font;
        }
    } else if paired.is_empty() && bt.is_scanning() {
        text.queue(
            "Scanning for devices…",
            row_font,
            inner_x,
            cy,
            muted,
            inner_w,
            surface_w,
            surface_h,
        );
        cy += row_font;
    } else if paired.is_empty() {
        text.queue(
            "No paired devices — turn on Scan to find new ones",
            row_font,
            inner_x,
            cy,
            muted,
            inner_w,
            surface_w,
            surface_h,
        );
        cy += row_font;
    }

    if let Some(err) = bt.last_error() {
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            row_font * 0.85,
            inner_x,
            cy + row_font * 0.5,
            red,
            inner_w,
            surface_w,
            surface_h,
        );
        cy += row_font;
    }

    // Last received file (sticky until a new send/receive cycle).
    if let Some(rx) = &bt.last_received {
        let received_msg = format!("Received {} → {}", rx.filename, rx.path);
        text.queue(
            &received_msg,
            row_font * 0.8,
            inner_x,
            cy + row_font * 0.5,
            white.with_alpha(0.78 * alpha),
            inner_w,
            surface_w,
            surface_h,
        );
    }

    // Pair prompts, incoming-file requests, and incoming-pair requests
    // are now drawn inline on the relevant device row (see
    // `draw_prompt_strip`), so there's no floating modal to compose here.

    painter.pop_clip();
    text.pop_clip();

    draw_scrollbar(painter, panel, &vp, scale, alpha);

    vp.top + vp.height
}

/// Thin scroll indicator down the panel's right edge, shown only while
/// the list overflows its viewport. Same look as the Wi-Fi list's.
fn draw_scrollbar(painter: &mut Painter, panel: Rect, vp: &ListViewport, scale: f32, alpha: f32) {
    if vp.max_scroll <= 0.0 {
        return;
    }
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let track_w = 4.0 * scale;
    let track_x = panel.x + panel.w - track_w - 6.0 * scale;
    painter.rect_filled(
        Rect::new(track_x, vp.top, track_w, vp.height),
        track_w / 2.0,
        white.with_alpha(0.06 * alpha),
    );
    let thumb_h = (vp.height * vp.height / (vp.height + vp.max_scroll)).max(24.0 * scale);
    let thumb_y = vp.top + (vp.height - thumb_h) * (vp.scroll / vp.max_scroll).clamp(0.0, 1.0);
    painter.rect_filled(
        Rect::new(track_x, thumb_y, track_w, thumb_h),
        track_w / 2.0,
        white.with_alpha(0.30 * alpha),
    );
}

/// Draw the Discoverable + Scan toggles row.
fn draw_toggles_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let layout = toggles_row_layout(panel, panel_top_y, scale);
    let label_font = TOGGLES_ROW_FONT * scale;
    let white = Color::from_rgb8(0xff, 0xff, 0xff);

    text.queue(
        "Discoverable",
        label_font,
        layout.discoverable_label.x,
        layout.discoverable_label.y,
        white.with_alpha(0.78 * alpha),
        layout.discoverable_label.w,
        surface_w,
        surface_h,
    );
    super::toggle::draw_toggle(
        painter,
        layout.discoverable_toggle,
        bt.is_discoverable(),
        alpha,
        scale,
    );

    text.queue(
        "Scan",
        label_font,
        layout.scan_label.x,
        layout.scan_label.y,
        white.with_alpha(0.78 * alpha),
        layout.scan_label.w,
        surface_w,
        surface_h,
    );
    super::toggle::draw_toggle(painter, layout.scan_toggle, bt.is_scanning(), alpha, scale);
}

/// The right-aligned status badge text + colour for a device row. A live
/// request (pair / file) overrides the steady-state status; otherwise the
/// badge reflects send progress, an in-flight connect/pair, or the plain
/// connected/paired/available state.
fn row_badge(bt: &Bluetooth, dev: &Device, alpha: f32) -> (String, Color) {
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let red = Color::from_rgb8(0xe0, 0x40, 0x40);

    if let Some(p) = row_prompt(bt, dev) {
        let text = match p {
            RowPrompt::IncomingFile { .. } => "Incoming file",
            _ => "Pair request",
        };
        return (text.into(), gold.with_alpha(alpha));
    }

    if let Some(s) = bt.send_state.get(&dev.mac) {
        return match &s.status {
            SendStatus::Starting => ("Picking…".into(), white.with_alpha(alpha)),
            SendStatus::InProgress => {
                let pct = if s.bytes_total > 0 {
                    ((s.bytes_done as f32 / s.bytes_total as f32) * 100.0).round() as i32
                } else {
                    0
                };
                let name = if s.filename.is_empty() {
                    "file".to_string()
                } else {
                    truncate_name(&s.filename, 20)
                };
                (
                    format!("Sending {} · {}%", name, pct),
                    white.with_alpha(alpha),
                )
            }
            SendStatus::Done => ("Sent ✓".into(), gold.with_alpha(alpha)),
            SendStatus::Failed(msg) => (
                format!("Send failed: {}", truncate_name(msg, 30)),
                red.with_alpha(alpha),
            ),
        };
    }

    if Some(dev.mac.as_str()) == bt.pending() {
        let text = if !dev.paired {
            "Pairing…"
        } else if dev.connected {
            "Disconnecting…"
        } else {
            "Connecting…"
        };
        return (text.into(), white.with_alpha(alpha));
    }

    if dev.connected {
        ("Connected".into(), gold.with_alpha(alpha))
    } else if dev.paired {
        ("Paired".into(), white.with_alpha(0.65 * alpha))
    } else {
        ("Available".into(), gold.with_alpha(alpha))
    }
}

/// Draw one device-list section ("Paired" or "Available"). Returns the
/// y-coordinate where the section ends. Rows scrolled out of `vp` keep
/// their place in the layout but skip the paint.
#[allow(clippy::too_many_arguments)]
fn draw_section(
    painter: &mut Painter,
    text: &mut TextRenderer,
    bt: &Bluetooth,
    header: &str,
    devices: &[&Device],
    vp: &ListViewport,
    inner_x: f32,
    inner_w: f32,
    start_y: f32,
    scale: f32,
    alpha: f32,
    text_size: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let header_font = SECTION_HEADER_FONT * scale;
    let header_gap = SECTION_HEADER_BOTTOM_GAP * scale;
    let row_h = ROW_HEIGHT * scale;
    let row_font = body_font(text_size, scale);
    let row_pad = ROW_INNER_PAD * scale;
    let right_gap = ROW_RIGHT_GAP * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);

    // Section header (small, muted).
    text.queue(
        header,
        header_font,
        inner_x,
        start_y,
        muted,
        inner_w,
        surface_w,
        surface_h,
    );

    let mut cy = start_y + header_font + header_gap;

    for (i, dev) in devices.iter().take(MAX_SECTION_ROWS).enumerate() {
        let extra = row_extra_height(bt, dev, text_size, scale);
        if !vp.shows(cy, row_h + extra) {
            cy += row_h + extra;
            continue;
        }
        let is_expanded = bt.expanded_mac.as_deref() == Some(dev.mac.as_str());
        let is_hovered = bt.hovered_mac.as_deref() == Some(dev.mac.as_str());
        let has_prompt = row_prompt(bt, dev).is_some();
        let row_rect = Rect::new(inner_x, cy, inner_w, row_h);

        if is_expanded || has_prompt {
            // Container plate behind the header + strip + expanded body.
            // A live request gets a gold-tinted plate so it stands out.
            let plate = if has_prompt {
                Color::rgba(0.78, 0.52, 0.04, 0.16 * alpha)
            } else {
                Color::rgba(0.0, 0.0, 0.0, 0.35 * alpha)
            };
            painter.rect_filled(
                Rect::new(inner_x, cy, inner_w, row_h + extra),
                10.0 * scale,
                plate,
            );
        } else if i % 2 == 0 {
            painter.rect_filled(row_rect, 8.0 * scale, white.with_alpha(0.04 * alpha));
        }
        if is_hovered && !is_expanded && !has_prompt {
            painter.rect_filled(row_rect, 8.0 * scale, white.with_alpha(0.10 * alpha));
        }

        // ── Header row: device name + status badge ──
        let name_y = cy + (row_h - row_font) / 2.0;
        let name_color = if dev.connected {
            white.with_alpha(alpha)
        } else {
            white.with_alpha(0.88 * alpha)
        };
        let display_name = if dev.name.is_empty() {
            dev.mac.as_str()
        } else {
            dev.name.as_str()
        };
        text.queue(
            display_name,
            row_font,
            inner_x + row_pad,
            name_y,
            name_color,
            inner_w * 0.65,
            surface_w,
            surface_h,
        );

        let (badge_text, badge_color) = row_badge(bt, dev, alpha);
        let badge_font = row_font * 0.85;
        let badge_w = text.measure_width(&badge_text, badge_font);
        let badge_x = inner_x + inner_w - badge_w - right_gap;
        let badge_y = cy + (row_h - badge_font) / 2.0;
        text.queue(
            &badge_text,
            badge_font,
            badge_x,
            badge_y,
            badge_color,
            badge_w,
            surface_w,
            surface_h,
        );

        cy += row_h;

        if has_prompt {
            cy += prompt::draw_prompt_strip(
                painter, text, dev, bt, inner_x, inner_w, cy, scale, alpha, text_size, surface_w,
                surface_h,
            );
        }

        if is_expanded {
            cy += detail::draw_expanded(
                painter, text, dev, bt, inner_x, inner_w, cy, scale, alpha, text_size, surface_w,
                surface_h,
            );
        }
    }

    cy
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_pill_button(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    label: &str,
    font: f32,
    accent: Color,
    alpha: f32,
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let radius = rect.h * 0.5;
    let bg = Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha);
    painter.rect_filled(rect, radius, bg);
    painter.rect_stroke_sdf(rect, radius, 1.5 * scale, accent.with_alpha(0.55 * alpha));
    let lw = text.measure_width(label, font);
    let lx = rect.x + (rect.w - lw) / 2.0;
    let ly = rect.y + (rect.h - font) / 2.0;
    // Wrap bound must be looser than the measured width: queue() quantizes
    // max_width down by up to 0.125px, which wraps the last glyph onto a
    // clipped second line ("Connect" → "Connec").
    text.queue(
        label,
        font,
        lx,
        ly,
        accent.with_alpha(0.95 * alpha),
        rect.w.max(lw),
        surface_w,
        surface_h,
    );
}

/// Truncate a filename or error message to fit visual width.
pub(super) fn truncate_name(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

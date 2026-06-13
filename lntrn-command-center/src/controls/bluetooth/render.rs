//! Bluetooth click-expand view — the full-content view (header, toggles
//! row, paired/available device sections) shown when the user opens the
//! BT tile, plus its hit-testing.
//!
//! The inline controls-row glyph lives in `super::glyph`; the expanded
//! device-detail block in `super::detail`; inline request strips (pair /
//! file Accept-Reject) in `super::prompt`.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::detail;
use super::prompt::{self, prompt_button_rects, prompt_extra_height, row_prompt, RowPrompt};
use super::{Bluetooth, Device, SendStatus};

// ── Click-expand view ───────────────────────────────────────────────────────

const VIEW_TOP_PAD: f32 = 24.0;
const VIEW_HEADER_FONT: f32 = 22.0;
const VIEW_HEADER_BOTTOM_GAP: f32 = 16.0;

const TOGGLES_ROW_HEIGHT: f32 = 36.0;
const TOGGLES_ROW_FONT: f32 = 16.0;
const TOGGLES_ROW_BOTTOM_GAP: f32 = 16.0;
const TOGGLE_W: f32 = 44.0;
const TOGGLE_H: f32 = 24.0;
const TOGGLE_LABEL_GAP: f32 = 8.0;
const TOGGLES_INTER_GAP: f32 = 24.0;

const SECTION_HEADER_FONT: f32 = 14.0;
const SECTION_HEADER_BOTTOM_GAP: f32 = 6.0;
const SECTION_GAP: f32 = 12.0;

const ROW_HEIGHT: f32 = 56.0;
pub(super) const ROW_INNER_PAD: f32 = 16.0;
pub(super) const ROW_RIGHT_GAP: f32 = 12.0;
const MAX_PAIRED_ROWS: usize = 4;
// Show more of the (now junk-filtered, RSSI-ranked) available list so a
// real device like earbuds isn't pushed off by a few named smart-locks.
const MAX_UNPAIRED_ROWS: usize = 6;

fn header_row_y(panel_top_y: f32, scale: f32) -> f32 {
    panel_top_y + VIEW_TOP_PAD * scale
}

fn toggles_row_y(panel_top_y: f32, scale: f32) -> f32 {
    header_row_y(panel_top_y, scale) + VIEW_HEADER_FONT * scale + VIEW_HEADER_BOTTOM_GAP * scale
}

fn list_top_y(panel_top_y: f32, scale: f32) -> f32 {
    toggles_row_y(panel_top_y, scale)
        + TOGGLES_ROW_HEIGHT * scale
        + TOGGLES_ROW_BOTTOM_GAP * scale
}

/// Power-toggle pill rect at the top right of the view.
pub fn toggle_rect(panel: Rect, panel_top_y: f32, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_w = panel.w - pad * 2.0;
    let toggle_w = TOGGLE_W * scale;
    let toggle_h = TOGGLE_H * scale;
    let header_font = VIEW_HEADER_FONT * scale;
    let header_y = header_row_y(panel_top_y, scale);
    let toggle_x = panel.x + pad + inner_w - toggle_w;
    let toggle_y = header_y + (header_font - toggle_h) / 2.0;
    Rect::new(toggle_x, toggle_y, toggle_w, toggle_h)
}

/// Layout of the discoverable + scan toggles in the second header row.
struct TogglesRow {
    discoverable_label: Rect,
    discoverable_toggle: Rect,
    scan_label: Rect,
    scan_toggle: Rect,
}

fn toggles_row_layout(panel: Rect, panel_top_y: f32, scale: f32) -> TogglesRow {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let row_y = toggles_row_y(panel_top_y, scale);
    let row_h = TOGGLES_ROW_HEIGHT * scale;
    let label_font = TOGGLES_ROW_FONT * scale;
    let toggle_w = TOGGLE_W * scale;
    let toggle_h = TOGGLE_H * scale;
    let label_gap = TOGGLE_LABEL_GAP * scale;
    let inter = TOGGLES_INTER_GAP * scale;

    // Tight estimates of label widths so the layout is deterministic
    // without measuring text. Slightly generous.
    let disc_label_w = label_font * 7.5; // "Discoverable"
    let scan_label_w = label_font * 4.0; // "Scan"

    let disc_label_x = inner_x;
    let disc_toggle_x = disc_label_x + disc_label_w + label_gap;
    let scan_label_x = disc_toggle_x + toggle_w + inter;
    let scan_toggle_x = scan_label_x + scan_label_w + label_gap;

    let label_y = row_y + (row_h - label_font) / 2.0;
    let toggle_y = row_y + (row_h - toggle_h) / 2.0;

    TogglesRow {
        discoverable_label: Rect::new(disc_label_x, label_y, disc_label_w, label_font),
        discoverable_toggle: Rect::new(disc_toggle_x, toggle_y, toggle_w, toggle_h),
        scan_label: Rect::new(scan_label_x, label_y, scan_label_w, label_font),
        scan_toggle: Rect::new(scan_toggle_x, toggle_y, toggle_w, toggle_h),
    }
}

/// Hit-test result for the BT view.
pub enum BtClick {
    PowerToggle,
    DiscoverableToggle,
    ScanToggle,
    /// Click on the **header** row of a device — toggles the expanded
    /// detail panel.
    DeviceRow(String),
    /// Click on the Connect / Disconnect / Pair button inside the
    /// expanded panel.
    ConnectButton(String),
    /// Click on the Send-file button inside the expanded panel. Only
    /// fires when the device exposes an OBEX push profile.
    SendButton(String),
    /// Accept button on an inline request strip (incoming file, incoming
    /// pair, or outgoing pair-confirm) attached to this device's row.
    PromptAccept(String),
    /// Reject button on an inline request strip.
    PromptReject(String),
}

/// Body font for the expanded view + inline strips, scaled off the
/// user's Text Size setting so device rows honour what they picked.
pub(super) fn body_font(text_size: f32, scale: f32) -> f32 {
    (text_size.max(12.0)) * scale
}

/// Total extra height a device row contributes below its header: the
/// inline prompt strip (if any) plus the expanded-detail block (if
/// expanded). A row with a live request always shows its strip, even
/// when not toggled open.
fn row_extra_height(bt: &Bluetooth, dev: &Device, text_size: f32, scale: f32) -> f32 {
    let mut h = prompt_extra_height(bt, dev, text_size, scale);
    if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
        h += detail::expanded_extra_height(dev, text_size, scale);
    }
    h
}

fn header_row_rect(inner_x: f32, inner_w: f32, row_y: f32, scale: f32) -> Rect {
    Rect::new(inner_x, row_y, inner_w, ROW_HEIGHT * scale)
}

/// Per-row layout passed to `walk_devices` visitors. `strip_top` is the
/// y where the inline request strip starts (= just below the header);
/// `expanded_top` is where the expanded-detail block starts (below the
/// strip when one is present).
struct RowGeom {
    header: Rect,
    strip_top: f32,
    expanded_top: f32,
}

/// Walk the device list top-to-bottom, mirroring the renderer's layout,
/// and invoke `visit` for each device with its row geometry. `visit`
/// returns `true` to stop iteration early (used by hit-testing).
fn walk_devices(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    text_size: f32,
    scale: f32,
    mut visit: impl FnMut(&Device, RowGeom) -> bool,
) {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let row_h = ROW_HEIGHT * scale;
    let section_header_h = SECTION_HEADER_FONT * scale + SECTION_HEADER_BOTTOM_GAP * scale;
    let section_gap = SECTION_GAP * scale;

    let mut cy = list_top_y(panel_top_y, scale);

    let walk_section = |cy: &mut f32, devs: &[&Device], max: usize, visit: &mut dyn FnMut(&Device, RowGeom) -> bool| -> bool {
        for dev in devs.iter().take(max) {
            let header = header_row_rect(inner_x, inner_w, *cy, scale);
            let strip_top = *cy + row_h;
            let prompt_h = prompt_extra_height(bt, dev, text_size, scale);
            let expanded_top = strip_top + prompt_h;
            if visit(dev, RowGeom { header, strip_top, expanded_top }) {
                return true;
            }
            *cy += row_h + row_extra_height(bt, dev, text_size, scale);
        }
        false
    };

    let paired = bt.paired_devices();
    if !paired.is_empty() {
        cy += section_header_h;
        if walk_section(&mut cy, &paired, MAX_PAIRED_ROWS, &mut visit) {
            return;
        }
        cy += section_gap;
    }
    let unpaired = bt.unpaired_devices();
    if !unpaired.is_empty() {
        cy += section_header_h;
        if walk_section(&mut cy, &unpaired, MAX_UNPAIRED_ROWS, &mut visit) {
            return;
        }
    }
}

/// Hit-test all interactive regions in the BT view.
pub fn hit_test(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    text_size: f32,
    x: f32,
    y: f32,
) -> Option<BtClick> {
    let inside = |r: Rect| {
        x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
    };

    let power = toggle_rect(panel, panel_top_y, scale);
    if inside(power) {
        return Some(BtClick::PowerToggle);
    }
    if bt.is_powered() {
        let togs = toggles_row_layout(panel, panel_top_y, scale);
        if inside(togs.discoverable_toggle) {
            return Some(BtClick::DiscoverableToggle);
        }
        if inside(togs.scan_toggle) {
            return Some(BtClick::ScanToggle);
        }
    }

    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }

    let mut hit: Option<BtClick> = None;
    walk_devices(bt, panel, panel_top_y, text_size, scale, |dev, geom| {
        // Inline request strip buttons take priority over the header
        // toggle so a click on Accept/Reject doesn't also collapse/expand.
        if row_prompt(bt, dev).is_some() {
            let (accept, reject, _field) =
                prompt_button_rects(bt, dev, inner_x, inner_w, geom.strip_top, text_size, scale);
            if inside(accept) {
                hit = Some(BtClick::PromptAccept(dev.mac.clone()));
                return true;
            }
            if inside(reject) {
                hit = Some(BtClick::PromptReject(dev.mac.clone()));
                return true;
            }
        }
        if inside(geom.header) {
            hit = Some(BtClick::DeviceRow(dev.mac.clone()));
            return true;
        }
        if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
            let connect = detail::connect_button_rect(inner_x, inner_w, geom.expanded_top, dev, text_size, scale);
            if inside(connect) {
                hit = Some(BtClick::ConnectButton(dev.mac.clone()));
                return true;
            }
            if let Some(send) = detail::send_button_rect_expanded(inner_x, inner_w, geom.expanded_top, dev, text_size, scale) {
                if inside(send) {
                    hit = Some(BtClick::SendButton(dev.mac.clone()));
                    return true;
                }
            }
        }
        false
    });
    hit
}

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
    draw_toggles_row(painter, text, bt, panel, panel_top_y, scale, alpha, surface_w, surface_h);

    // ── Device sections ──
    let mut cy = list_top_y(panel_top_y, scale);
    let paired = bt.paired_devices();
    let unpaired = bt.unpaired_devices();

    if !paired.is_empty() {
        cy = draw_section(
            painter, text, bt, "Paired", &paired, MAX_PAIRED_ROWS,
            inner_x, inner_w, cy, scale, alpha, text_size, surface_w, surface_h,
        );
        cy += SECTION_GAP * scale;
    }

    if !unpaired.is_empty() {
        let header = if bt.is_scanning() { "Available" } else { "Recently seen" };
        cy = draw_section(
            painter, text, bt, header, &unpaired, MAX_UNPAIRED_ROWS,
            inner_x, inner_w, cy, scale, alpha, text_size, surface_w, surface_h,
        );
        // Note how many discovered-but-unnamed devices we hid, so a device
        // whose name BlueZ hasn't resolved yet isn't a silent mystery.
        let hidden = bt.hidden_unpaired_count();
        if hidden > 0 {
            let msg = format!("+{hidden} unnamed device{} hidden", if hidden == 1 { "" } else { "s" });
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
        cy += row_font;
    }

    // Pair prompts, incoming-file requests, and incoming-pair requests
    // are now drawn inline on the relevant device row (see
    // `draw_prompt_strip`), so there's no floating modal to compose here.

    cy
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
    super::toggle::draw_toggle(painter, layout.discoverable_toggle, bt.is_discoverable(), alpha, scale);

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
                let name = if s.filename.is_empty() { "file".to_string() }
                    else { truncate_name(&s.filename, 20) };
                (format!("Sending {} · {}%", name, pct), white.with_alpha(alpha))
            }
            SendStatus::Done => ("Sent ✓".into(), gold.with_alpha(alpha)),
            SendStatus::Failed(msg) => {
                (format!("Send failed: {}", truncate_name(msg, 30)), red.with_alpha(alpha))
            }
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
/// y-coordinate where the section ends.
#[allow(clippy::too_many_arguments)]
fn draw_section(
    painter: &mut Painter,
    text: &mut TextRenderer,
    bt: &Bluetooth,
    header: &str,
    devices: &[&Device],
    max_rows: usize,
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

    for (i, dev) in devices.iter().take(max_rows).enumerate() {
        let is_expanded = bt.expanded_mac.as_deref() == Some(dev.mac.as_str());
        let is_hovered = bt.hovered_mac.as_deref() == Some(dev.mac.as_str());
        let has_prompt = row_prompt(bt, dev).is_some();
        let extra = row_extra_height(bt, dev, text_size, scale);
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
        let name_color = if dev.connected { white.with_alpha(alpha) } else { white.with_alpha(0.88 * alpha) };
        let display_name = if dev.name.is_empty() { dev.mac.as_str() } else { dev.name.as_str() };
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
                painter, text, dev, bt, inner_x, inner_w, cy, scale, alpha,
                text_size, surface_w, surface_h,
            );
        }

        if is_expanded {
            cy += detail::draw_expanded(
                painter, text, dev, bt, inner_x, inner_w, cy, scale, alpha,
                text_size, surface_w, surface_h,
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
        label, font, lx, ly, accent.with_alpha(0.95 * alpha),
        rect.w.max(lw), surface_w, surface_h,
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


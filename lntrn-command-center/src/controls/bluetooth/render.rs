//! Bluetooth view rendering — inline tile in the controls row + the
//! full-content view (header, toggles row, paired/available device
//! sections) when the user opens the BT tile.
//!
//! Modal dialogs (pair prompt, incoming-file request) live in
//! `super::modals` since they have their own layout and click flow.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;
use super::{Bluetooth, Device, SendStatus};

// ── Inline tile ─────────────────────────────────────────────────────────────

const ICON_SIZE: f32 = 28.0;
const ICON_LEFT_PAD: f32 = 16.0;

pub const TILE_WIDTH: f32 = ICON_LEFT_PAD + ICON_SIZE;

pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    bt: &Bluetooth,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
) {
    if !bt.is_present() {
        return;
    }
    let icon_size = ICON_SIZE * scale;
    let icon_x = layout.x + ICON_LEFT_PAD * scale;
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;
    // Faded when the controller is off so the row reads "BT is off"
    // without needing a separate badge.
    let icon_alpha = if bt.is_powered() { alpha } else { 0.30 * alpha };
    draw_bt_glyph(painter, icon_x, icon_y, icon_size, icon_size, icon_alpha);
}

/// The classic "B" rune: two stacked diamonds sharing the right vertex.
/// Drawn as 4 triangles. Pure shapes, no SVG.
fn draw_bt_glyph(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    // We render the bluetooth rune as a stroked path: top-bottom spine
    // line + four diagonal lines forming the two bowed-out shapes.
    // Stroked shapes look better than triangle-fanning a non-convex
    // polygon (lesson from the lightning bolt).
    let stroke = w * 0.12;
    let top = pt(0.50, 0.05);
    let bot = pt(0.50, 0.95);
    let mid_left = pt(0.20, 0.30);
    let mid_left_b = pt(0.20, 0.70);
    let upper_right = pt(0.80, 0.30);
    let lower_right = pt(0.80, 0.70);
    let center = pt(0.50, 0.50);

    // Spine (vertical line top → bottom).
    painter.line_round(top.0, top.1, bot.0, bot.1, stroke, color);
    // Top diamond: top → upper-right → center.
    painter.line_round(top.0, top.1, upper_right.0, upper_right.1, stroke, color);
    painter.line_round(upper_right.0, upper_right.1, center.0, center.1, stroke, color);
    // Bottom diamond: center → lower-right → bottom.
    painter.line_round(center.0, center.1, lower_right.0, lower_right.1, stroke, color);
    painter.line_round(lower_right.0, lower_right.1, bot.0, bot.1, stroke, color);
    // The two left-side cross strokes that complete the bowtie.
    painter.line_round(top.0, top.1, mid_left.0, mid_left.1, stroke, color);
    painter.line_round(mid_left.0, mid_left.1, center.0, center.1, stroke, color);
    painter.line_round(center.0, center.1, mid_left_b.0, mid_left_b.1, stroke, color);
    painter.line_round(mid_left_b.0, mid_left_b.1, bot.0, bot.1, stroke, color);
}

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
const ROW_FONT: f32 = 22.0;
const ROW_INNER_PAD: f32 = 16.0;
const ROW_RIGHT_GAP: f32 = 12.0;
const MAX_PAIRED_ROWS: usize = 4;
const MAX_UNPAIRED_ROWS: usize = 4;

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
    /// Click on a paired-or-discovered device row. Caller decides
    /// connect-vs-pair based on its `paired` state.
    DeviceRow(String),
    /// Click on the small "Send" button on a paired device row.
    SendButton(String),
}

/// Hit-test all interactive regions in the BT view.
pub fn hit_test(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
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
    // Send button (paired rows only) — checked before the row action
    // zone so the click doesn't fall through to Connect/Disconnect.
    // Also gated on no in-flight transfer for the row, matching the
    // draw-side logic.
    for dev in bt.paired_devices().iter().take(MAX_PAIRED_ROWS) {
        if bt.send_state.contains_key(&dev.mac) {
            continue;
        }
        if let Some(rect) = paired_row_send_rect(bt, panel, panel_top_y, scale, &dev.mac) {
            if inside(rect) {
                return Some(BtClick::SendButton(dev.mac.clone()));
            }
        }
    }
    if let Some(mac) = hit_test_device(bt, panel, panel_top_y, scale, x, y) {
        return Some(BtClick::DeviceRow(mac));
    }
    None
}

/// Width (logical px) of the right-side action zone in each device
/// row. Click here = connect / disconnect / pair. Click elsewhere on
/// the row = nothing. Sized generously so the user doesn't have to be
/// pixel-precise on the small badge text.
pub const ROW_ACTION_ZONE_WIDTH: f32 = 200.0;

/// Send-button geometry on a paired device row.
const SEND_BTN_W: f32 = 76.0;
const SEND_BTN_H: f32 = 36.0;
/// Distance (logical px) from the row's right edge to the Send button's
/// right edge. Smaller = Send button further right.
const SEND_BTN_RIGHT_OFFSET: f32 = 120.0;

/// Compute the Send-button rect for a row whose top-left is `(inner_x, row_y)`
/// and whose width is `inner_w`. Anchored from the right edge so the
/// position stays fixed regardless of badge text width.
fn send_button_rect(inner_x: f32, inner_w: f32, row_y: f32, scale: f32) -> Rect {
    let row_h = ROW_HEIGHT * scale;
    let btn_w = SEND_BTN_W * scale;
    let btn_h = SEND_BTN_H * scale;
    let right_edge = inner_x + inner_w - SEND_BTN_RIGHT_OFFSET * scale;
    let btn_x = right_edge - btn_w;
    let btn_y = row_y + (row_h - btn_h) / 2.0;
    Rect::new(btn_x, btn_y, btn_w, btn_h)
}

/// Walk paired rows and return the rect of the Send button for the row
/// matching `mac`, if any.
fn paired_row_send_rect(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    mac: &str,
) -> Option<Rect> {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let row_h = ROW_HEIGHT * scale;
    let section_header_h = SECTION_HEADER_FONT * scale + SECTION_HEADER_BOTTOM_GAP * scale;

    let mut cy = list_top_y(panel_top_y, scale);
    let paired = bt.paired_devices();
    if paired.is_empty() {
        return None;
    }
    cy += section_header_h;
    for dev in paired.iter().take(MAX_PAIRED_ROWS) {
        if dev.mac == mac {
            return Some(send_button_rect(inner_x, inner_w, cy, scale));
        }
        cy += row_h;
    }
    None
}

/// Hit-test the device list, restricted to the **action zone** on the
/// right side of each row. Click on the device name area is a no-op so
/// the user can right-click for "Send file" without accidentally
/// flipping the connection.
pub fn hit_test_device(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }
    // Restrict click target to the right-side action zone.
    let action_w = ROW_ACTION_ZONE_WIDTH * scale;
    let action_left = inner_x + inner_w - action_w;
    if x < action_left {
        return None;
    }

    let row_h = ROW_HEIGHT * scale;
    let section_header_h = SECTION_HEADER_FONT * scale + SECTION_HEADER_BOTTOM_GAP * scale;
    let section_gap = SECTION_GAP * scale;

    let mut cy = list_top_y(panel_top_y, scale);

    // Paired section.
    let paired = bt.paired_devices();
    if !paired.is_empty() {
        cy += section_header_h;
        for dev in paired.iter().take(MAX_PAIRED_ROWS) {
            if y >= cy && y <= cy + row_h {
                return Some(dev.mac.clone());
            }
            cy += row_h;
        }
        cy += section_gap;
    }

    // Available section (unpaired, only meaningful while scanning).
    let unpaired = bt.unpaired_devices();
    if !unpaired.is_empty() {
        cy += section_header_h;
        for dev in unpaired.iter().take(MAX_UNPAIRED_ROWS) {
            if y >= cy && y <= cy + row_h {
                return Some(dev.mac.clone());
            }
            cy += row_h;
        }
    }
    None
}

/// Hit-test the **whole row** (left side too). Used for actions that
/// should land anywhere on the row. Kept for future gestures (e.g. a
/// long-press / context menu); the current Send-button UI doesn't need
/// it.
#[allow(dead_code)]
pub fn hit_test_device_row_anywhere(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    if x < inner_x || x > inner_x + inner_w {
        return None;
    }
    let row_h = ROW_HEIGHT * scale;
    let section_header_h = SECTION_HEADER_FONT * scale + SECTION_HEADER_BOTTOM_GAP * scale;
    let section_gap = SECTION_GAP * scale;

    let mut cy = list_top_y(panel_top_y, scale);
    let paired = bt.paired_devices();
    if !paired.is_empty() {
        cy += section_header_h;
        for dev in paired.iter().take(MAX_PAIRED_ROWS) {
            if y >= cy && y <= cy + row_h {
                return Some(dev.mac.clone());
            }
            cy += row_h;
        }
        cy += section_gap;
    }
    let unpaired = bt.unpaired_devices();
    if !unpaired.is_empty() {
        cy += section_header_h;
        for dev in unpaired.iter().take(MAX_UNPAIRED_ROWS) {
            if y >= cy && y <= cy + row_h {
                return Some(dev.mac.clone());
            }
            cy += row_h;
        }
    }
    None
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    bt: &Bluetooth,
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
    let row_font = ROW_FONT * scale;

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
    super::modals::draw_toggle(painter, t_rect, bt.is_powered(), alpha, scale);

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
            inner_x, inner_w, cy, scale, alpha, surface_w, surface_h,
        );
        cy += SECTION_GAP * scale;
    }

    if !unpaired.is_empty() {
        let header = if bt.is_scanning() { "Available" } else { "Recently seen" };
        cy = draw_section(
            painter, text, bt, header, &unpaired, MAX_UNPAIRED_ROWS,
            inner_x, inner_w, cy, scale, alpha, surface_w, surface_h,
        );
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

    // Modals float on layer 1 so their backdrop + box correctly cover
    // any text we already queued underneath. The layered render pass in
    // layershell.rs is what makes this actually work — see the
    // TEXT_OCCLUSION_FIX doc in lntrn-render/.
    if bt.pair_prompt.is_some() || bt.incoming_request.is_some() {
        painter.set_layer(1);
        text.set_layer(1);

        if let Some(prompt) = &bt.pair_prompt {
            super::modals::draw_pair_modal(
                painter, text, prompt, panel, panel_top_y, scale, alpha, surface_w, surface_h,
            );
        }
        // Incoming-file modal also floats. We draw last so it sits on
        // top of the pair modal (rare to coincide, but well-defined).
        if let Some(req) = &bt.incoming_request {
            super::modals::draw_incoming_modal(
                painter, text, req, panel, panel_top_y, scale, alpha, surface_w, surface_h,
            );
        }
    }

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
    super::modals::draw_toggle(painter, layout.discoverable_toggle, bt.is_discoverable(), alpha, scale);

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
    super::modals::draw_toggle(painter, layout.scan_toggle, bt.is_scanning(), alpha, scale);
}

/// Draw one device-list section ("Paired" or "Available"). Returns the
/// y-coordinate where the section ends.
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
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let header_font = SECTION_HEADER_FONT * scale;
    let header_gap = SECTION_HEADER_BOTTOM_GAP * scale;
    let row_h = ROW_HEIGHT * scale;
    let row_font = ROW_FONT * scale;
    let row_pad = ROW_INNER_PAD * scale;
    let right_gap = ROW_RIGHT_GAP * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let muted = white.with_alpha(0.55 * alpha);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);

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
        let row_rect = Rect::new(inner_x, cy, inner_w, row_h);

        // Subtle alternating-row stripe; connected rows use the same
        // muted bg so only the gold "Connected" badge marks state.
        if i % 2 == 0 {
            painter.rect_filled(row_rect, 8.0 * scale, white.with_alpha(0.04 * alpha));
        }

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

        // Status badge on the right. If an outgoing transfer is in
        // flight or recently finished/failed, show its state instead.
        let send_state = bt.send_state.get(&dev.mac);
        let in_flight = Some(dev.mac.as_str()) == bt.pending();
        let badge_text: String = if let Some(s) = send_state {
            match &s.status {
                SendStatus::Starting => "Picking…".to_string(),
                SendStatus::InProgress => {
                    let pct = if s.bytes_total > 0 {
                        ((s.bytes_done as f32 / s.bytes_total as f32) * 100.0).round() as i32
                    } else {
                        0
                    };
                    let name = if s.filename.is_empty() { "file".to_string() }
                        else { truncate_name(&s.filename, 20) };
                    format!("Sending {} · {}%", name, pct)
                }
                SendStatus::Done => "Sent ✓".to_string(),
                SendStatus::Failed(msg) => format!("Send failed: {}", truncate_name(msg, 30)),
            }
        } else if in_flight {
            if !dev.paired {
                "Pairing…".into()
            } else if dev.connected {
                "Disconnecting…".into()
            } else {
                "Connecting…".into()
            }
        } else if dev.connected {
            "Connected".into()
        } else if dev.paired {
            "Connect".into()
        } else {
            "Pair".into()
        };
        let badge_font = row_font * 0.85;
        let badge_w = text.measure_width(&badge_text, badge_font);
        let badge_x = inner_x + inner_w - badge_w - right_gap;
        let badge_y = cy + (row_h - badge_font) / 2.0;
        let badge_color = match send_state.map(|s| &s.status) {
            Some(SendStatus::Done) => gold.with_alpha(alpha),
            Some(SendStatus::Failed(_)) => Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha),
            Some(_) => white.with_alpha(alpha),
            None => {
                if dev.connected {
                    gold.with_alpha(alpha)
                } else if dev.paired {
                    white.with_alpha(0.65 * alpha)
                } else {
                    gold.with_alpha(alpha)
                }
            }
        };
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

        // Send button (paired rows only, hidden while any send-state
        // entry exists for this device — its filename / progress badge
        // takes the visual slot).
        if dev.paired && send_state.is_none() {
            let btn = send_button_rect(inner_x, inner_w, cy, scale);
            painter.rect_filled(btn, btn.h * 0.5, white.with_alpha(0.10 * alpha));
            let label = "Send";
            let label_font = row_font * 0.78;
            let label_w = text.measure_width(label, label_font);
            let lx = btn.x + (btn.w - label_w) / 2.0;
            let ly = btn.y + (btn.h - label_font) / 2.0;
            text.queue(
                label,
                label_font,
                lx,
                ly,
                gold.with_alpha(0.95 * alpha),
                label_w,
                surface_w,
                surface_h,
            );
        }

        cy += row_h;
    }

    cy
}

/// Truncate a filename or error message to fit visual width.
fn truncate_name(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}


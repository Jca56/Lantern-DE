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

#[allow(clippy::too_many_arguments)]
pub fn draw_inline(
    painter: &mut Painter,
    _text: &mut TextRenderer,
    bt: &Bluetooth,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    _surface_w: u32,
    _surface_h: u32,
    lit: bool,
) {
    if !bt.is_present() {
        return;
    }
    let icon_size = ICON_SIZE * scale;
    let icon_x = layout.x + ICON_LEFT_PAD * scale;
    let icon_y = layout.y + (layout.h - icon_size) / 2.0;
    let icon_alpha = if bt.is_powered() { alpha } else { 0.30 * alpha };
    let color = if lit {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(icon_alpha)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(icon_alpha)
    };
    draw_bt_glyph_colored(painter, icon_x, icon_y, icon_size, icon_size, color);
}

#[allow(dead_code)]
fn draw_bt_glyph(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
    let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha);
    draw_bt_glyph_colored(painter, x, y, w, h, color);
}

fn draw_bt_glyph_colored(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, color: Color) {
    let pt = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
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
    /// Click on the **header** row of a device — toggles the expanded
    /// detail panel.
    DeviceRow(String),
    /// Click on the Connect / Disconnect / Pair button inside the
    /// expanded panel.
    ConnectButton(String),
    /// Click on the Send-file button inside the expanded panel. Only
    /// fires when the device exposes an OBEX push profile.
    SendButton(String),
}

// ── Expanded-row constants ─────────────────────────────────────────────
/// Padding inside the expanded panel.
const EXPAND_PAD_TOP: f32 = 10.0;
const EXPAND_PAD_BOTTOM: f32 = 14.0;
const EXPAND_LINE_GAP: f32 = 6.0;
const EXPAND_LABEL_W_FRAC: f32 = 0.30;
const EXPAND_BUTTON_TOP_GAP: f32 = 14.0;
const EXPAND_BUTTON_H: f32 = 44.0;
const EXPAND_BUTTON_W: f32 = 180.0;
const EXPAND_BUTTON_GAP: f32 = 12.0;

/// Body font for the expanded view, scaled off the user's Text Size
/// setting. We use the setting directly so the device-detail rows
/// honour what the user picked in Settings.
fn body_font(text_size: f32, scale: f32) -> f32 {
    (text_size.max(12.0)) * scale
}

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
fn expanded_extra_height(dev: &Device, text_size: f32, scale: f32) -> f32 {
    let lines = detail_lines(dev);
    let body = body_font(text_size, scale);
    let gap = EXPAND_LINE_GAP * scale;
    let detail_h = lines.len() as f32 * (body + gap) - gap.max(0.0);
    let mut h = EXPAND_PAD_TOP * scale + detail_h.max(0.0);
    h += EXPAND_BUTTON_TOP_GAP * scale + EXPAND_BUTTON_H * scale;
    h += EXPAND_PAD_BOTTOM * scale;
    h
}

fn header_row_rect(inner_x: f32, inner_w: f32, row_y: f32, scale: f32) -> Rect {
    Rect::new(inner_x, row_y, inner_w, ROW_HEIGHT * scale)
}

/// Compute the Connect button rect inside `dev`'s expanded panel.
fn connect_button_rect(
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
fn send_button_rect_expanded(
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
    Some(Rect::new(connect.x + connect.w + gap, connect.y, btn_w, connect.h))
}

/// Walk the device list top-to-bottom, mirroring the renderer's layout,
/// and invoke `visit` for each device with its header rect, expanded-top
/// y, and a reference to the device. `visit` returns `true` to stop
/// iteration early (used by hit-testing).
fn walk_devices(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    text_size: f32,
    scale: f32,
    mut visit: impl FnMut(&Device, Rect, f32) -> bool,
) {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let row_h = ROW_HEIGHT * scale;
    let section_header_h = SECTION_HEADER_FONT * scale + SECTION_HEADER_BOTTOM_GAP * scale;
    let section_gap = SECTION_GAP * scale;

    let mut cy = list_top_y(panel_top_y, scale);

    let paired = bt.paired_devices();
    if !paired.is_empty() {
        cy += section_header_h;
        for dev in paired.iter().take(MAX_PAIRED_ROWS) {
            let header = header_row_rect(inner_x, inner_w, cy, scale);
            let expanded_top = cy + row_h;
            if visit(dev, header, expanded_top) {
                return;
            }
            cy += row_h;
            if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
                cy += expanded_extra_height(dev, text_size, scale);
            }
        }
        cy += section_gap;
    }
    let unpaired = bt.unpaired_devices();
    if !unpaired.is_empty() {
        cy += section_header_h;
        for dev in unpaired.iter().take(MAX_UNPAIRED_ROWS) {
            let header = header_row_rect(inner_x, inner_w, cy, scale);
            let expanded_top = cy + row_h;
            if visit(dev, header, expanded_top) {
                return;
            }
            cy += row_h;
            if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
                cy += expanded_extra_height(dev, text_size, scale);
            }
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
    walk_devices(bt, panel, panel_top_y, text_size, scale, |dev, header, expanded_top| {
        if inside(header) {
            hit = Some(BtClick::DeviceRow(dev.mac.clone()));
            return true;
        }
        if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
            let connect = connect_button_rect(inner_x, inner_w, expanded_top, dev, text_size, scale);
            if inside(connect) {
                hit = Some(BtClick::ConnectButton(dev.mac.clone()));
                return true;
            }
            if let Some(send) = send_button_rect_expanded(inner_x, inner_w, expanded_top, dev, text_size, scale) {
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
        let is_expanded = bt.expanded_mac.as_deref() == Some(dev.mac.as_str());
        let is_hovered = bt.hovered_mac.as_deref() == Some(dev.mac.as_str());
        let row_rect = Rect::new(inner_x, cy, inner_w, row_h);

        if is_expanded {
            // Container plate behind the header + expanded body —
            // darker grey to match the Wi-Fi pattern.
            let total_h = row_h + expanded_extra_height(dev, text_size, scale);
            painter.rect_filled(
                Rect::new(inner_x, cy, inner_w, total_h),
                10.0 * scale,
                Color::rgba(0.0, 0.0, 0.0, 0.35 * alpha),
            );
        } else if i % 2 == 0 {
            painter.rect_filled(row_rect, 8.0 * scale, white.with_alpha(0.04 * alpha));
        }
        if is_hovered && !is_expanded {
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
            "Paired".into()
        } else {
            "Available".into()
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

        cy += row_h;

        if is_expanded {
            cy += draw_expanded(
                painter, text, dev, bt, inner_x, inner_w, cy, scale, alpha,
                text_size, surface_w, surface_h,
            );
        }
    }

    cy
}

#[allow(clippy::too_many_arguments)]
fn draw_expanded(
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
            label, lbl_font, inner_x + pad_l, cy + (body - lbl_font) / 2.0,
            muted, label_w - pad_l, surface_w, surface_h,
        );
        let value_x = inner_x + pad_l + label_w;
        text.queue(
            &value, body, value_x, cy, white.with_alpha(0.92 * alpha),
            inner_w - (label_w + pad_l + ROW_RIGHT_GAP * scale),
            surface_w, surface_h,
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
    draw_pill_button(painter, text, connect, connect_label, body, gold, alpha, scale, surface_w, surface_h);

    if let Some(send) = send_button_rect_expanded(inner_x, inner_w, expanded_top, dev, text_size, scale) {
        let label = if bt.send_state.contains_key(&dev.mac) {
            "Sending…"
        } else {
            "Send file"
        };
        draw_pill_button(painter, text, send, label, body, gold, alpha, scale, surface_w, surface_h);
    }

    expanded_extra_height(dev, text_size, scale)
}

#[allow(clippy::too_many_arguments)]
fn draw_pill_button(
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
    text.queue(
        label, font, lx, ly, accent.with_alpha(0.95 * alpha),
        lw, surface_w, surface_h,
    );
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


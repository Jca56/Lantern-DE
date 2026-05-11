//! Password-prompt modal that overlays the WiFi network list when the
//! user picks a secured network they don't have credentials for.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::PasswordPrompt;

const MODAL_W: f32 = 480.0;
const MODAL_H: f32 = 240.0;
const MODAL_RADIUS: f32 = 16.0;
const MODAL_PAD: f32 = 24.0;
const MODAL_TITLE_FONT: f32 = 22.0;
const MODAL_FIELD_HEIGHT: f32 = 48.0;
const MODAL_FIELD_FONT: f32 = 22.0;
const MODAL_BUTTON_HEIGHT: f32 = 44.0;
const MODAL_BUTTON_GAP: f32 = 12.0;
const MODAL_BUTTON_FONT: f32 = 18.0;
const MODAL_BACKDROP_ALPHA: f32 = 0.55;

/// Region rects for the password modal — used both for drawing and
/// for routing clicks. All values in physical pixels.
pub struct ModalRegions {
    /// Backdrop — anything inside the panel but outside `box_rect`.
    /// Click here cancels the modal.
    pub backdrop: Rect,
    pub box_rect: Rect,
    pub field: Rect,
    pub connect_btn: Rect,
    pub cancel_btn: Rect,
}

/// Compute modal regions for the given panel + content top y.
pub fn modal_regions(panel: Rect, panel_top_y: f32, scale: f32) -> ModalRegions {
    let modal_w = MODAL_W * scale;
    let modal_h = MODAL_H * scale;
    let pad = MODAL_PAD * scale;
    let field_h = MODAL_FIELD_HEIGHT * scale;
    let title_font = MODAL_TITLE_FONT * scale;
    let btn_h = MODAL_BUTTON_HEIGHT * scale;
    let btn_gap = MODAL_BUTTON_GAP * scale;

    // Center the modal box horizontally inside the panel; vertically,
    // place it a bit below the controls row underline.
    let box_x = panel.x + (panel.w - modal_w) / 2.0;
    let box_y = panel_top_y + (panel.h - (panel_top_y - panel.y) - modal_h) * 0.30;
    let box_rect = Rect::new(box_x, box_y, modal_w, modal_h);

    // Field y = pad + title + breathing room.
    let field_y = box_y + pad + title_font + pad * 0.5;
    let field_rect = Rect::new(box_x + pad, field_y, modal_w - pad * 2.0, field_h);

    // Buttons at the bottom — Cancel left, Connect right (each ~half
    // width minus gap).
    let buttons_y = box_y + modal_h - pad - btn_h;
    let half_w = (modal_w - pad * 2.0 - btn_gap) / 2.0;
    let cancel_btn = Rect::new(box_x + pad, buttons_y, half_w, btn_h);
    let connect_btn = Rect::new(box_x + pad + half_w + btn_gap, buttons_y, half_w, btn_h);

    let backdrop = Rect::new(panel.x, panel_top_y, panel.w, panel.h - (panel_top_y - panel.y));

    ModalRegions { backdrop, box_rect, field: field_rect, connect_btn, cancel_btn }
}

/// Hit-test a click against the modal. Returns which region was hit.
pub fn hit_test_modal(panel: Rect, panel_top_y: f32, scale: f32, x: f32, y: f32) -> ModalHit {
    let r = modal_regions(panel, panel_top_y, scale);
    let inside = |rect: Rect| {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    };
    if inside(r.connect_btn) {
        ModalHit::Connect
    } else if inside(r.cancel_btn) {
        ModalHit::Cancel
    } else if inside(r.field) {
        ModalHit::Field
    } else if inside(r.box_rect) {
        ModalHit::Box
    } else {
        ModalHit::Backdrop
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalHit {
    Connect,
    Cancel,
    Field,
    /// Inside the modal box but not on a specific control (no-op).
    Box,
    /// Inside the panel but outside the modal — cancel.
    Backdrop,
}

pub(super) fn draw_modal(
    painter: &mut Painter,
    text: &mut TextRenderer,
    prompt: &PasswordPrompt,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    last_error: Option<&str>,
    surface_w: u32,
    surface_h: u32,
) {
    let r = modal_regions(panel, panel_top_y, scale);
    let pad = MODAL_PAD * scale;
    let title_font = MODAL_TITLE_FONT * scale;
    let field_font = MODAL_FIELD_FONT * scale;
    let btn_font = MODAL_BUTTON_FONT * scale;
    let radius = MODAL_RADIUS * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let muted = white.with_alpha(0.55 * alpha);

    // Backdrop dim — covers the network list behind the modal.
    painter.rect_filled(r.backdrop, 0.0, Color::BLACK.with_alpha(MODAL_BACKDROP_ALPHA * alpha));

    // Modal box.
    painter.rect_filled(
        r.box_rect,
        radius,
        Color::from_rgb8(0x24, 0x24, 0x24).with_alpha(alpha),
    );

    // Title: "Password for {SSID}".
    let title = format!("Password for {}", prompt.ssid);
    text.queue(
        &title,
        title_font,
        r.box_rect.x + pad,
        r.box_rect.y + pad,
        white.with_alpha(alpha),
        r.box_rect.w - pad * 2.0,
        surface_w,
        surface_h,
    );

    // Field background.
    painter.rect_filled(
        r.field,
        8.0 * scale,
        Color::from_rgb8(0x14, 0x14, 0x14).with_alpha(alpha),
    );
    painter.rect_stroke_sdf(
        r.field,
        8.0 * scale,
        1.5 * scale,
        white.with_alpha(0.18 * alpha),
    );

    // Masked password.
    let mask_count = prompt.input.query().chars().count();
    let mask = "•".repeat(mask_count);
    let display: &str = if mask_count == 0 { "Enter password" } else { &mask };
    let display_color = if mask_count == 0 { muted } else { white.with_alpha(alpha) };
    let field_text_x = r.field.x + 12.0 * scale;
    let field_text_y = r.field.y + (r.field.h - field_font) / 2.0;
    text.queue(
        display,
        field_font,
        field_text_x,
        field_text_y,
        display_color,
        r.field.w - 24.0 * scale,
        surface_w,
        surface_h,
    );

    // Cursor — only when the field has content (so it doesn't overlap
    // the placeholder).
    if mask_count > 0 && prompt.input.cursor_visible() {
        // Each `•` we emit advances by mask_w / mask_count, but since
        // glyph widths can vary slightly, measure the prefix directly.
        // Cursor lies on a char boundary in `prompt.input`'s real buffer;
        // count chars up to that point and use that many bullets.
        let cursor_char_idx = prompt
            .input
            .query()
            .char_indices()
            .take_while(|(b, _)| *b < prompt.input.cursor_byte())
            .count();
        let prefix: String = std::iter::repeat('•').take(cursor_char_idx).collect();
        let prefix_w = text.measure_width(&prefix, field_font);
        let cx = field_text_x + prefix_w;
        let cy = field_text_y - 2.0 * scale;
        let ch = field_font + 4.0 * scale;
        painter.rect_filled(
            Rect::new(cx, cy, 2.0 * scale, ch),
            1.0 * scale,
            gold.with_alpha(alpha),
        );
    }

    // Cancel button (subtle outline).
    painter.rect_filled(
        r.cancel_btn,
        8.0 * scale,
        white.with_alpha(0.06 * alpha),
    );
    let cancel_label = "Cancel";
    let cancel_w = text.measure_width(cancel_label, btn_font);
    text.queue(
        cancel_label,
        btn_font,
        r.cancel_btn.x + (r.cancel_btn.w - cancel_w) / 2.0,
        r.cancel_btn.y + (r.cancel_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        cancel_w,
        surface_w,
        surface_h,
    );

    // Connect button (gold). Dim when connecting or empty.
    let can_submit = !prompt.input.query().is_empty() && !prompt.connecting;
    let conn_alpha = if can_submit { alpha } else { 0.5 * alpha };
    painter.rect_filled(
        r.connect_btn,
        8.0 * scale,
        gold.with_alpha(conn_alpha),
    );
    let connect_label = if prompt.connecting { "Connecting…" } else { "Connect" };
    let connect_w = text.measure_width(connect_label, btn_font);
    text.queue(
        connect_label,
        btn_font,
        r.connect_btn.x + (r.connect_btn.w - connect_w) / 2.0,
        r.connect_btn.y + (r.connect_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        connect_w,
        surface_w,
        surface_h,
    );

    // Error below the buttons (last attempt failed).
    if let Some(err) = last_error {
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            btn_font * 0.85,
            r.box_rect.x + pad,
            r.cancel_btn.y - btn_font - 4.0 * scale,
            red,
            r.box_rect.w - pad * 2.0,
            surface_w,
            surface_h,
        );
    }
}

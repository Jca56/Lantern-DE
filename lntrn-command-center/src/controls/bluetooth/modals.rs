//! Bluetooth modal dialogs:
//! - **Pair prompt** — confirm-passkey / enter-passkey / authorize-service.
//! - **Incoming-file request** — Accept / Reject for an OBEX push.
//!
//! Each modal owns its own layout helpers (`*_regions`), hit-testers
//! (`hit_test_*`), and draw fns. Both are summoned by the BT view in
//! `super::render::draw_view` when the corresponding state is set on
//! `Bluetooth`.
//!
//! `draw_toggle` lives here because both modals and the view's power
//! toggle need it; keeping it in one place avoids drift in the visual.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{IncomingRequest, PairPrompt, PairPromptKind};

// ── Pair-prompt modal ───────────────────────────────────────────────────────

const MODAL_W: f32 = 480.0;
const MODAL_H: f32 = 280.0;
const MODAL_RADIUS: f32 = 16.0;
const MODAL_PAD: f32 = 24.0;
const MODAL_TITLE_FONT: f32 = 20.0;
const MODAL_PROMPT_FONT: f32 = 16.0;
const MODAL_PASSKEY_FONT: f32 = 32.0;
const MODAL_FIELD_HEIGHT: f32 = 48.0;
const MODAL_BUTTON_HEIGHT: f32 = 44.0;
const MODAL_BUTTON_GAP: f32 = 12.0;
const MODAL_BUTTON_FONT: f32 = 18.0;
const MODAL_BACKDROP_ALPHA: f32 = 0.55;

/// Modal hit zones.
pub struct PairModalRegions {
    pub backdrop: Rect,
    pub box_rect: Rect,
    pub field: Option<Rect>,
    pub primary_btn: Rect,
    pub secondary_btn: Rect,
}

/// Hit-test result for the pair-prompt modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairModalHit {
    Primary,
    Secondary,
    Field,
    Box,
    Backdrop,
}

pub fn pair_modal_regions(
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    has_field: bool,
) -> PairModalRegions {
    let modal_w = MODAL_W * scale;
    let modal_h = MODAL_H * scale;
    let pad = MODAL_PAD * scale;
    let field_h = MODAL_FIELD_HEIGHT * scale;
    let btn_h = MODAL_BUTTON_HEIGHT * scale;
    let btn_gap = MODAL_BUTTON_GAP * scale;

    let box_x = panel.x + (panel.w - modal_w) / 2.0;
    let box_y = panel_top_y + (panel.h - (panel_top_y - panel.y) - modal_h) * 0.30;
    let box_rect = Rect::new(box_x, box_y, modal_w, modal_h);

    let field = if has_field {
        // Roughly centered vertically inside the body.
        let field_y = box_y + modal_h * 0.50 - field_h * 0.5;
        Some(Rect::new(box_x + pad, field_y, modal_w - pad * 2.0, field_h))
    } else {
        None
    };

    let buttons_y = box_y + modal_h - pad - btn_h;
    let half_w = (modal_w - pad * 2.0 - btn_gap) / 2.0;
    let secondary = Rect::new(box_x + pad, buttons_y, half_w, btn_h);
    let primary = Rect::new(box_x + pad + half_w + btn_gap, buttons_y, half_w, btn_h);

    let backdrop = Rect::new(panel.x, panel_top_y, panel.w, panel.h - (panel_top_y - panel.y));

    PairModalRegions {
        backdrop,
        box_rect,
        field,
        primary_btn: primary,
        secondary_btn: secondary,
    }
}

pub fn hit_test_pair_modal(
    prompt: &PairPrompt,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> PairModalHit {
    let has_field = matches!(prompt.kind, PairPromptKind::Enter);
    let r = pair_modal_regions(panel, panel_top_y, scale, has_field);
    let inside = |rr: Rect| {
        x >= rr.x && x <= rr.x + rr.w && y >= rr.y && y <= rr.y + rr.h
    };
    if inside(r.primary_btn) {
        PairModalHit::Primary
    } else if inside(r.secondary_btn) {
        PairModalHit::Secondary
    } else if r.field.map_or(false, inside) {
        PairModalHit::Field
    } else if inside(r.box_rect) {
        PairModalHit::Box
    } else {
        PairModalHit::Backdrop
    }
}

pub fn draw_pair_modal(
    painter: &mut Painter,
    text: &mut TextRenderer,
    prompt: &PairPrompt,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let has_field = matches!(prompt.kind, PairPromptKind::Enter);
    let r = pair_modal_regions(panel, panel_top_y, scale, has_field);
    let pad = MODAL_PAD * scale;
    let title_font = MODAL_TITLE_FONT * scale;
    let prompt_font = MODAL_PROMPT_FONT * scale;
    let passkey_font = MODAL_PASSKEY_FONT * scale;
    let field_font = MODAL_PASSKEY_FONT * scale * 0.7;
    let btn_font = MODAL_BUTTON_FONT * scale;
    let radius = MODAL_RADIUS * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let muted = white.with_alpha(0.55 * alpha);

    // Backdrop dim.
    painter.rect_filled(r.backdrop, 0.0, Color::BLACK.with_alpha(MODAL_BACKDROP_ALPHA * alpha));

    // Box.
    painter.rect_filled(
        r.box_rect,
        radius,
        Color::from_rgb8(0x24, 0x24, 0x24).with_alpha(alpha),
    );

    // Title — "Pair {device}".
    let title = format!("Pair {}", prompt.device_name);
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

    // Body — depends on prompt kind.
    let body_y = r.box_rect.y + pad + title_font + pad * 0.6;
    let (subtitle, primary_label, secondary_label) = match &prompt.kind {
        PairPromptKind::Confirm(passkey) => {
            // Big passkey display + "Confirm this number matches the device's display".
            text.queue(
                "Confirm this number matches the device's display",
                prompt_font,
                r.box_rect.x + pad,
                body_y,
                muted,
                r.box_rect.w - pad * 2.0,
                surface_w,
                surface_h,
            );
            let pk_w = text.measure_width(passkey, passkey_font);
            text.queue(
                passkey,
                passkey_font,
                r.box_rect.x + (r.box_rect.w - pk_w) / 2.0,
                body_y + prompt_font + pad * 0.5,
                white.with_alpha(alpha),
                pk_w,
                surface_w,
                surface_h,
            );
            ("", "Confirm", "Cancel")
        }
        PairPromptKind::Enter => {
            text.queue(
                "Enter the passkey shown on the device",
                prompt_font,
                r.box_rect.x + pad,
                body_y,
                muted,
                r.box_rect.w - pad * 2.0,
                surface_w,
                surface_h,
            );
            // Field background.
            if let Some(field) = r.field {
                painter.rect_filled(
                    field,
                    8.0 * scale,
                    Color::from_rgb8(0x14, 0x14, 0x14).with_alpha(alpha),
                );
                painter.rect_stroke_sdf(
                    field,
                    8.0 * scale,
                    1.5 * scale,
                    white.with_alpha(0.18 * alpha),
                );

                let raw = prompt.passkey_input.query();
                let display = if raw.is_empty() { "passkey" } else { raw };
                let dc = if raw.is_empty() { muted } else { white.with_alpha(alpha) };
                let text_x = field.x + 12.0 * scale;
                let text_y = field.y + (field.h - field_font) / 2.0;
                text.queue(
                    display,
                    field_font,
                    text_x,
                    text_y,
                    dc,
                    field.w - 24.0 * scale,
                    surface_w,
                    surface_h,
                );
                if !raw.is_empty() && prompt.passkey_input.cursor_visible() {
                    let prefix_chars: usize = raw
                        .char_indices()
                        .take_while(|(b, _)| *b < prompt.passkey_input.cursor_byte())
                        .count();
                    let prefix: String = raw.chars().take(prefix_chars).collect();
                    let cx = text_x + text.measure_width(&prefix, field_font);
                    let cy = text_y - 2.0 * scale;
                    let ch = field_font + 4.0 * scale;
                    painter.rect_filled(
                        Rect::new(cx, cy, 2.0 * scale, ch),
                        1.0 * scale,
                        gold.with_alpha(alpha),
                    );
                }
            }
            ("", "Connect", "Cancel")
        }
        PairPromptKind::Authorize(svc) => {
            let msg = format!("Authorize service {}?", svc);
            text.queue(
                &msg,
                prompt_font,
                r.box_rect.x + pad,
                body_y,
                white.with_alpha(alpha),
                r.box_rect.w - pad * 2.0,
                surface_w,
                surface_h,
            );
            ("", "Authorize", "Reject")
        }
    };
    let _ = subtitle;

    // Buttons — secondary outlined, primary gold.
    painter.rect_filled(
        r.secondary_btn,
        8.0 * scale,
        white.with_alpha(0.06 * alpha),
    );
    let sw = text.measure_width(secondary_label, btn_font);
    text.queue(
        secondary_label,
        btn_font,
        r.secondary_btn.x + (r.secondary_btn.w - sw) / 2.0,
        r.secondary_btn.y + (r.secondary_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        sw,
        surface_w,
        surface_h,
    );

    let primary_active = match &prompt.kind {
        PairPromptKind::Enter => !prompt.passkey_input.query().is_empty(),
        _ => true,
    };
    let p_alpha = if primary_active { alpha } else { 0.5 * alpha };
    painter.rect_filled(
        r.primary_btn,
        8.0 * scale,
        gold.with_alpha(p_alpha),
    );
    let pw = text.measure_width(primary_label, btn_font);
    text.queue(
        primary_label,
        btn_font,
        r.primary_btn.x + (r.primary_btn.w - pw) / 2.0,
        r.primary_btn.y + (r.primary_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        pw,
        surface_w,
        surface_h,
    );

    // Inline error.
    if let Some(err) = &prompt.error {
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        text.queue(
            err,
            btn_font * 0.85,
            r.box_rect.x + pad,
            r.secondary_btn.y - btn_font - 4.0 * scale,
            red,
            r.box_rect.w - pad * 2.0,
            surface_w,
            surface_h,
        );
    }
}

// ── Incoming-file modal ─────────────────────────────────────────────────────

/// Hit-test result for the incoming-file modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingModalHit {
    Accept,
    Reject,
    Box,
    Backdrop,
}

pub fn incoming_modal_regions(panel: Rect, panel_top_y: f32, scale: f32) -> PairModalRegions {
    pair_modal_regions(panel, panel_top_y, scale, false)
}

pub fn hit_test_incoming_modal(
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> IncomingModalHit {
    let r = incoming_modal_regions(panel, panel_top_y, scale);
    let inside = |rr: Rect| {
        x >= rr.x && x <= rr.x + rr.w && y >= rr.y && y <= rr.y + rr.h
    };
    if inside(r.primary_btn) {
        IncomingModalHit::Accept
    } else if inside(r.secondary_btn) {
        IncomingModalHit::Reject
    } else if inside(r.box_rect) {
        IncomingModalHit::Box
    } else {
        IncomingModalHit::Backdrop
    }
}

pub(super) fn draw_incoming_modal(
    painter: &mut Painter,
    text: &mut TextRenderer,
    req: &IncomingRequest,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let r = incoming_modal_regions(panel, panel_top_y, scale);
    let pad = MODAL_PAD * scale;
    let title_font = MODAL_TITLE_FONT * scale;
    let body_font = MODAL_PROMPT_FONT * scale;
    let btn_font = MODAL_BUTTON_FONT * scale;
    let radius = MODAL_RADIUS * scale;

    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let muted = white.with_alpha(0.55 * alpha);

    painter.rect_filled(
        r.backdrop,
        0.0,
        Color::BLACK.with_alpha(MODAL_BACKDROP_ALPHA * alpha),
    );
    painter.rect_filled(
        r.box_rect,
        radius,
        Color::from_rgb8(0x24, 0x24, 0x24).with_alpha(alpha),
    );

    let title = format!("Incoming file from {}", req.from_name);
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

    let detail = if req.size > 0 {
        format!("{}  ·  {}", req.filename, format_bytes(req.size))
    } else {
        req.filename.clone()
    };
    text.queue(
        &detail,
        body_font,
        r.box_rect.x + pad,
        r.box_rect.y + pad + title_font + pad * 0.6,
        muted,
        r.box_rect.w - pad * 2.0,
        surface_w,
        surface_h,
    );

    // Reject (secondary), Accept (primary).
    painter.rect_filled(
        r.secondary_btn,
        8.0 * scale,
        white.with_alpha(0.06 * alpha),
    );
    let reject_w = text.measure_width("Reject", btn_font);
    text.queue(
        "Reject",
        btn_font,
        r.secondary_btn.x + (r.secondary_btn.w - reject_w) / 2.0,
        r.secondary_btn.y + (r.secondary_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        reject_w,
        surface_w,
        surface_h,
    );
    painter.rect_filled(r.primary_btn, 8.0 * scale, gold.with_alpha(alpha));
    let accept_w = text.measure_width("Accept", btn_font);
    text.queue(
        "Accept",
        btn_font,
        r.primary_btn.x + (r.primary_btn.w - accept_w) / 2.0,
        r.primary_btn.y + (r.primary_btn.h - btn_font) / 2.0,
        white.with_alpha(alpha),
        accept_w,
        surface_w,
        surface_h,
    );
}

fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut idx = 0;
    while v >= 1024.0 && idx < UNITS.len() - 1 {
        v /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} B", n)
    } else {
        format!("{:.1} {}", v, UNITS[idx])
    }
}

/// iOS-style on/off toggle. Same shape as the battery's charge-limit toggle.
pub(super) fn draw_toggle(painter: &mut Painter, rect: Rect, on: bool, alpha: f32, scale: f32) {
    let radius = rect.h * 0.5;
    let track_color = if on {
        Color::from_rgb8(0xc8, 0x86, 0x0a).with_alpha(alpha)
    } else {
        Color::from_rgb8(0x44, 0x44, 0x44).with_alpha(alpha)
    };
    painter.rect_filled(rect, radius, track_color);
    let inset = 3.0 * scale;
    let knob_r = (rect.h - inset * 2.0) * 0.5;
    let knob_cy = rect.y + rect.h * 0.5;
    let knob_cx = if on {
        rect.x + rect.w - inset - knob_r
    } else {
        rect.x + inset + knob_r
    };
    painter.circle_filled(
        knob_cx,
        knob_cy,
        knob_r,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
    );
}

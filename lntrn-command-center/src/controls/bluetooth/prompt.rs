//! Inline request strips on a Bluetooth device row.
//!
//! Pairing prompts (both directions) and incoming-file requests used to
//! float over the whole page as modal dialogs. They now render in place
//! on the row of the device they concern, with Accept/Reject buttons (and
//! an inline PIN field for the rare "enter passkey" flow). This module
//! owns the strip's layout (`row_prompt`, `prompt_extra_height`,
//! `prompt_button_rects`) and drawing (`draw_prompt_strip`); the parent
//! `render` module composes them into the device list.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::render::{body_font, draw_pill_button, truncate_name, ROW_INNER_PAD};
use super::{Bluetooth, Device};

// ── Strip geometry constants ───────────────────────────────────────────
const PROMPT_PAD_TOP: f32 = 8.0;
const PROMPT_PAD_BOTTOM: f32 = 12.0;
const PROMPT_TEXT_GAP: f32 = 8.0;
const PROMPT_BTN_H: f32 = 40.0;
const PROMPT_BTN_W: f32 = 120.0;
const PROMPT_BTN_GAP: f32 = 10.0;
const PROMPT_FIELD_W: f32 = 160.0;

/// What inline request strip — if any — a device row should render below
/// its header. Mirrors the modal flows that used to float over the page,
/// now drawn in place on the device's row.
pub(super) enum RowPrompt {
    /// Another device wants to pair with us. `Some(passkey)` shows the
    /// number to compare; `None` is a bare yes/no authorization.
    IncomingPair(Option<u32>),
    /// A device is trying to push a file to us.
    IncomingFile { filename: String, size: u64 },
    /// We initiated pairing and BlueZ asked us to confirm a passkey, or
    /// authorize a service. Just Accept/Reject.
    OutgoingConfirm(String),
    /// We initiated pairing and BlueZ wants a PIN typed in. Accept reads
    /// the inline field; the field text comes from `pair_prompt`.
    OutgoingEnter,
}

/// Resolve the inline prompt (if any) for `dev`. Priority: outgoing pair
/// flow (we started it) → incoming pair → incoming file. Only one strip
/// shows per row at a time.
pub(super) fn row_prompt(bt: &Bluetooth, dev: &Device) -> Option<RowPrompt> {
    use super::PairPromptKind;
    if let Some(p) = &bt.pair_prompt {
        if p.mac == dev.mac {
            return Some(match &p.kind {
                PairPromptKind::Confirm(pk) => RowPrompt::OutgoingConfirm(pk.clone()),
                PairPromptKind::Authorize(svc) => RowPrompt::OutgoingConfirm(svc.clone()),
                PairPromptKind::Enter => RowPrompt::OutgoingEnter,
            });
        }
    }
    if let Some(pr) = &bt.pair_request {
        if pr.mac == dev.mac {
            return Some(RowPrompt::IncomingPair(pr.passkey));
        }
    }
    if let Some(req) = &bt.incoming_request {
        // Incoming-file requests are keyed by sender name, not MAC, so
        // match on the friendly name we display for the row.
        let dev_name = if dev.name.is_empty() {
            dev.mac.as_str()
        } else {
            dev.name.as_str()
        };
        if req.from_name == dev_name || req.from_name == dev.alias {
            return Some(RowPrompt::IncomingFile {
                filename: req.filename.clone(),
                size: req.size,
            });
        }
    }
    None
}

/// Height of the inline prompt strip for `dev`, or 0.0 if it has none.
/// One text line (the prompt message) + a button row.
pub(super) fn prompt_extra_height(bt: &Bluetooth, dev: &Device, text_size: f32, scale: f32) -> f32 {
    if row_prompt(bt, dev).is_none() {
        return 0.0;
    }
    let body = body_font(text_size, scale);
    PROMPT_PAD_TOP * scale
        + body
        + PROMPT_TEXT_GAP * scale
        + PROMPT_BTN_H * scale
        + PROMPT_PAD_BOTTOM * scale
}

/// Accept/Reject (and optional PIN field) rects for the inline prompt
/// strip, given the strip's top y. Returns `(accept, reject, field)`.
pub(super) fn prompt_button_rects(
    bt: &Bluetooth,
    dev: &Device,
    inner_x: f32,
    _inner_w: f32,
    strip_top: f32,
    text_size: f32,
    scale: f32,
) -> (Rect, Rect, Option<Rect>) {
    let body = body_font(text_size, scale);
    let pad_l = ROW_INNER_PAD * scale;
    let btn_h = PROMPT_BTN_H * scale;
    let btn_w = PROMPT_BTN_W * scale;
    let gap = PROMPT_BTN_GAP * scale;
    let btn_y = strip_top + PROMPT_PAD_TOP * scale + body + PROMPT_TEXT_GAP * scale;

    let mut x = inner_x + pad_l;
    let field = if matches!(row_prompt(bt, dev), Some(RowPrompt::OutgoingEnter)) {
        let f = Rect::new(x, btn_y, PROMPT_FIELD_W * scale, btn_h);
        x += PROMPT_FIELD_W * scale + gap;
        Some(f)
    } else {
        None
    };
    let accept = Rect::new(x, btn_y, btn_w, btn_h);
    let reject = Rect::new(x + btn_w + gap, btn_y, btn_w, btn_h);
    (accept, reject, field)
}

/// Draw the inline request strip (incoming pair, incoming file, or
/// outgoing pair-confirm) for `dev`. Returns the strip's height so the
/// caller can advance its cursor. `top_y` is the strip's top.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_prompt_strip(
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
    let Some(prompt) = row_prompt(bt, dev) else {
        return 0.0;
    };
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let body = body_font(text_size, scale);
    let pad_l = ROW_INNER_PAD * scale;

    // Prompt message line.
    let msg: String = match &prompt {
        RowPrompt::IncomingPair(Some(pk)) => format!("Wants to pair · confirm {:06}", pk),
        RowPrompt::IncomingPair(None) => "Wants to pair with this device".into(),
        RowPrompt::IncomingFile { filename, size } => {
            if *size > 0 {
                format!(
                    "Sending {} · {}",
                    truncate_name(filename, 24),
                    format_bytes(*size)
                )
            } else {
                format!("Sending {}", truncate_name(filename, 28))
            }
        }
        RowPrompt::OutgoingConfirm(s) => format!("Confirm code {} on the device", s),
        RowPrompt::OutgoingEnter => "Enter the passkey shown on the device".into(),
    };
    let msg_y = top_y + PROMPT_PAD_TOP * scale;
    text.queue(
        &msg,
        body,
        inner_x + pad_l,
        msg_y,
        white.with_alpha(0.92 * alpha),
        inner_w - pad_l * 2.0,
        surface_w,
        surface_h,
    );

    // Accept / Reject buttons (+ optional PIN field).
    let (accept, reject, field) =
        prompt_button_rects(bt, dev, inner_x, inner_w, top_y, text_size, scale);

    if let Some(field) = field {
        draw_pin_field(
            painter, text, field, bt, body, alpha, scale, surface_w, surface_h,
        );
    }

    // Accept is disabled until a non-empty PIN is typed (Enter flow only).
    let accept_ready = match &prompt {
        RowPrompt::OutgoingEnter => bt
            .pair_prompt
            .as_ref()
            .map(|p| !p.passkey_input.query().is_empty())
            .unwrap_or(false),
        _ => true,
    };
    let accept_label = if matches!(prompt, RowPrompt::OutgoingEnter) {
        "Connect"
    } else {
        "Accept"
    };
    let a_alpha = if accept_ready { alpha } else { 0.5 * alpha };
    draw_solid_button(
        painter,
        text,
        accept,
        accept_label,
        body,
        gold,
        a_alpha,
        surface_w,
        surface_h,
    );
    draw_pill_button(
        painter, text, reject, "Reject", body, white, alpha, scale, surface_w, surface_h,
    );

    // Per-prompt error (e.g. wrong PIN) for the outgoing pair flow,
    // tucked at the right of the message line so it doesn't grow the row.
    if let Some(err) = bt
        .pair_prompt
        .as_ref()
        .filter(|p| p.mac == dev.mac)
        .and_then(|p| p.error.as_ref())
    {
        let red = Color::from_rgb8(0xe0, 0x40, 0x40).with_alpha(alpha);
        let efont = body * 0.85;
        let ew = text.measure_width(err, efont).min(inner_w * 0.5);
        text.queue(
            err,
            efont,
            inner_x + inner_w - ew - pad_l,
            msg_y + (body - efont) / 2.0,
            red,
            ew,
            surface_w,
            surface_h,
        );
    }

    prompt_extra_height(bt, dev, text_size, scale)
}

/// Inline PIN field for the outgoing-Enter pair flow.
#[allow(clippy::too_many_arguments)]
fn draw_pin_field(
    painter: &mut Painter,
    text: &mut TextRenderer,
    field: Rect,
    bt: &Bluetooth,
    font: f32,
    alpha: f32,
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let white = Color::from_rgb8(0xff, 0xff, 0xff);
    let gold = Color::from_rgb8(0xc8, 0x86, 0x0a);
    let muted = white.with_alpha(0.55 * alpha);
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

    let Some(input) = bt.pair_prompt.as_ref().map(|p| &p.passkey_input) else {
        return;
    };
    let raw = input.query();
    let display = if raw.is_empty() { "passkey" } else { raw };
    let dc = if raw.is_empty() {
        muted
    } else {
        white.with_alpha(alpha)
    };
    let tx = field.x + 12.0 * scale;
    let ty = field.y + (field.h - font) / 2.0;
    text.queue(
        display,
        font,
        tx,
        ty,
        dc,
        field.w - 24.0 * scale,
        surface_w,
        surface_h,
    );
    if !raw.is_empty() && input.cursor_visible() {
        let prefix_chars = raw
            .char_indices()
            .take_while(|(b, _)| *b < input.cursor_byte())
            .count();
        let prefix: String = raw.chars().take(prefix_chars).collect();
        let cx = tx + text.measure_width(&prefix, font);
        painter.rect_filled(
            Rect::new(cx, ty - 2.0 * scale, 2.0 * scale, font + 4.0 * scale),
            1.0 * scale,
            gold.with_alpha(alpha),
        );
    }
}

/// Filled accent button (the primary Accept/Connect action on a strip).
/// Contrasts with the outlined `draw_pill_button`.
#[allow(clippy::too_many_arguments)]
fn draw_solid_button(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    label: &str,
    font: f32,
    accent: Color,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let radius = rect.h * 0.5;
    painter.rect_filled(rect, radius, accent.with_alpha(alpha));
    let lw = text.measure_width(label, font);
    let lx = rect.x + (rect.w - lw) / 2.0;
    let ly = rect.y + (rect.h - font) / 2.0;
    text.queue(
        label,
        font,
        lx,
        ly,
        Color::from_rgb8(0x16, 0x12, 0x06).with_alpha(0.95 * alpha),
        lw,
        surface_w,
        surface_h,
    );
}

/// Human-readable byte count for incoming-file prompts.
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

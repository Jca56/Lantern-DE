//! Drawing of the dialogs in `op_dialogs.rs`: one panel, a title, wrapped
//! text, and up to two large buttons.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use super::OpDialog;
use crate::dialogs::{draw_button, draw_overlay_with_scrim, wrap_lines, ButtonStyle};
use crate::{ZONE_OP_DIALOG_ACT, ZONE_OP_DIALOG_PANEL, ZONE_OP_DIALOG_SAFE, ZONE_OP_DIALOG_SCRIM};

/// Shorten `text` from the middle until it fits `max_w` (paths and file
/// names have no spaces to wrap at).
pub(crate) fn fit_middle(text: &mut TextRenderer, line: &str, font: f32, max_w: f32) -> String {
    if text.measure_width(line, font) <= max_w {
        return line.to_string();
    }
    let chars: Vec<char> = line.chars().collect();
    let mut keep = chars.len().saturating_sub(1);
    while keep > 4 {
        let head = keep / 2;
        let tail = keep - head;
        let candidate: String = chars[..head]
            .iter()
            .chain(std::iter::once(&'\u{2026}'))
            .chain(chars[chars.len() - tail..].iter())
            .collect();
        if text.measure_width(&candidate, font) <= max_w {
            return candidate;
        }
        keep -= (keep / 10).max(1);
    }
    "\u{2026}".to_string()
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    dialog: &OpDialog,
    busy_label: Option<String>,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    // (title, body, safe button, acting button)
    let close_lines;
    let (title, lines, safe, act): (&str, &[String], Option<&str>, Option<&str>) = match dialog {
        OpDialog::Notice { title, lines } => (title, lines, Some("OK"), None),
        OpDialog::ConfirmDelete { title, lines, .. } => {
            (title, lines, Some("Cancel"), Some("Delete Permanently"))
        }
        OpDialog::Leftovers { title, lines, .. } => {
            (title, lines, Some("Keep"), Some("Move to Trash"))
        }
        OpDialog::Pick(dialog) => (
            &dialog.title,
            &dialog.lines,
            Some("Cancel"),
            dialog.act_label(),
        ),
        OpDialog::Cloud(dialog) => {
            return crate::cloud_ui::draw_dialog(dialog, painter, text, pal, input, screen, s);
        }
        OpDialog::CloseBusy { stopping: false } => {
            close_lines = vec![
                match busy_label {
                    Some(what) => format!("{what} is still running."),
                    None => "An operation is still running.".to_string(),
                },
                String::new(),
                "Keep Open lets it finish. Stop and Close cancels it: items already done stay, the one in progress is undone, and nothing is left half-written.".to_string(),
            ];
            (
                "Close while files are still being worked on?",
                &close_lines[..],
                Some("Keep Open"),
                Some("Stop and Close"),
            )
        }
        OpDialog::CloseBusy { stopping: true } => {
            close_lines = vec![
                "Undoing the item that was in progress. The window closes by itself in a moment."
                    .to_string(),
            ];
            ("Stopping\u{2026}", &close_lines[..], None, None)
        }
    };

    let pad = 28.0 * s;
    let cr = 12.0 * s;
    let title_font = 24.0 * s;
    let body_font = 18.0 * s;
    let line_gap = 8.0 * s;
    let btn_h = 52.0 * s;
    let btn_gap = 14.0 * s;
    let dialog_w = (660.0 * s).min(screen_w - 32.0 * s).max(200.0 * s);
    let text_w = dialog_w - pad * 2.0;

    let title_lines = wrap_lines(text, title, title_font, text_w);
    let title_h = (title_font + line_gap) * title_lines.len() as f32;
    let has_buttons = safe.is_some() || act.is_some();
    let buttons_h = if has_buttons { pad + btn_h } else { 0.0 };
    let line_h = body_font + line_gap;

    // As many body lines as the window has room for.
    let room = screen_h - 32.0 * s - pad * 2.0 - title_h - pad * 0.4 - buttons_h;
    let max_lines = ((room / line_h).floor() as usize).max(1);
    let mut body: Vec<String> = Vec::new();
    for line in lines {
        body.extend(wrap_lines(text, line, body_font, text_w));
    }
    if body.len() > max_lines {
        body.truncate(max_lines.saturating_sub(1));
        body.push("\u{2026}".to_string());
    }

    let dialog_h = pad * 2.0 + title_h + pad * 0.4 + line_h * body.len() as f32 + buttons_h;
    let dx = (screen_w - dialog_w) * 0.5;
    let dy = ((screen_h - dialog_h) * 0.5).max(0.0);

    draw_overlay_with_scrim(
        painter,
        input,
        screen_w,
        screen_h,
        dx,
        dy,
        dialog_w,
        dialog_h,
        cr,
        pal,
        s,
        ZONE_OP_DIALOG_SCRIM,
    );
    // The panel is its own zone so a click on its text is not a click
    // "outside" that dismisses it.
    input.add_zone(ZONE_OP_DIALOG_PANEL, Rect::new(dx, dy, dialog_w, dialog_h));

    let mut cy = dy + pad;
    for line in &title_lines {
        let shown = fit_middle(text, line, title_font, text_w);
        TextLabel::new(&shown, dx + pad, cy)
            .size(FontSize::Custom(title_font))
            .color(pal.text)
            // Slack so the measured last glyph isn't clipped by the bound.
            .max_width(text_w + 4.0 * s)
            .draw(text, sw, sh);
        cy += title_font + line_gap;
    }
    cy += pad * 0.4;
    for line in &body {
        let shown = fit_middle(text, line, body_font, text_w);
        TextLabel::new(&shown, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.text_secondary)
            .max_width(text_w + 4.0 * s)
            .draw(text, sw, sh);
        cy += line_h;
    }
    if !has_buttons {
        return;
    }
    cy += pad - line_gap;

    // Generous buttons, sized to their labels. The acting one sits on the
    // right; the safe one is the larger target on its left.
    let btn_font = 18.0 * s;
    let width_for = |text: &mut TextRenderer, label: &str| {
        (text.measure_width(label, btn_font) + 56.0 * s).max(170.0 * s)
    };
    let mut right = dx + dialog_w - pad;
    if let Some(label) = act {
        let w = width_for(text, label);
        let rect = Rect::new(right - w, cy, w, btn_h);
        let hovered = input.add_zone(ZONE_OP_DIALOG_ACT, rect).is_hovered();
        draw_button(painter, text, rect, label, hovered, pal, ButtonStyle::Danger, sw, sh, s);
        right -= w + btn_gap;
    }
    if let Some(label) = safe {
        let w = width_for(text, label);
        let rect = Rect::new(right - w, cy, w, btn_h);
        let hovered = input.add_zone(ZONE_OP_DIALOG_SAFE, rect).is_hovered();
        draw_button(painter, text, rect, label, hovered, pal, ButtonStyle::Primary, sw, sh, s);
    }
}

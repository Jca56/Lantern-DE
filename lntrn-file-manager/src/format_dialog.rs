//! The "Format this drive?" dialog, and what it turns into while the
//! format runs (app/device_ops.rs runs it on a worker thread).

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::dialogs::{draw_button, draw_overlay, wrap_lines, ButtonStyle};
use crate::fs::Drive;
use crate::{ZONE_DRIVE_DIALOG_CANCEL, ZONE_DRIVE_DIALOG_CONFIRM};

#[allow(clippy::too_many_arguments)]
pub fn draw(
    drive: &Drive,
    error: Option<&str>,
    working: bool,
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

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 28.0 * s;
    let body_font = 18.0 * s;
    let row_gap = 8.0 * s;
    let btn_h = 40.0 * s;
    let btn_w = 120.0 * s;
    let btn_gap = 12.0 * s;
    let dialog_w = 480.0 * s;

    // The error is wrapped: a UDisks message is rarely one short line.
    let error_lines = match error {
        Some(err) if !working => wrap_lines(
            text,
            &format!("Error: {err}"),
            body_font,
            dialog_w - pad * 2.0,
        ),
        _ => Vec::new(),
    };
    let body_lines: f32 = 3.0 + error_lines.len() as f32;
    let dialog_h = pad * 2.0
        + title_font
        + pad * 0.6
        + body_font * body_lines
        + row_gap * (body_lines - 1.0)
        + pad
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay(
        painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s,
    );

    let mut cy = dy + pad;
    let title = if working {
        format!("Formatting {}{}", drive.name, crate::bg::dots())
    } else {
        format!("Format {}?", drive.name)
    };
    TextLabel::new(&title, dx + pad, cy)
        .size(FontSize::Custom(title_font))
        .color(pal.text)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += title_font + pad * 0.6;

    let body_color = pal.text_secondary;
    TextLabel::new(&format!("Device: {}", drive.device), dx + pad, cy)
        .size(FontSize::Custom(body_font))
        .color(body_color)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + row_gap;
    TextLabel::new(&format!("Size: {}", drive.total_display()), dx + pad, cy)
        .size(FontSize::Custom(body_font))
        .color(body_color)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + row_gap;
    let warning = if working {
        "Keep the drive plugged in until this is done."
    } else {
        "All data on this drive will be erased."
    };
    TextLabel::new(warning, dx + pad, cy)
        .size(FontSize::Custom(body_font))
        .color(pal.danger)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + row_gap;

    for line in &error_lines {
        TextLabel::new(line, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.danger)
            // Slack so the measured last glyph isn't clipped by the wrap bound.
            .max_width(dialog_w - pad * 2.0 + 4.0 * s)
            .draw(text, sw, sh);
        cy += body_font + row_gap;
    }

    cy += pad - row_gap;

    let total_btn_w = btn_w * 2.0 + btn_gap;
    let btn_x = dx + dialog_w - pad - total_btn_w;

    // While the format runs there is nothing to cancel and nothing to
    // confirm: the one button puts the dialog away. The format carries on,
    // the sidebar row says so, and the result is reported when it is in.
    let cancel_rect = Rect::new(btn_x, cy, btn_w, btn_h);
    let cancel_state = input.add_zone(ZONE_DRIVE_DIALOG_CANCEL, cancel_rect);
    draw_button(
        painter,
        text,
        cancel_rect,
        if working { "Hide" } else { "Cancel" },
        cancel_state.is_hovered(),
        pal,
        ButtonStyle::Secondary,
        sw,
        sh,
        s,
    );

    let confirm_rect = Rect::new(btn_x + btn_w + btn_gap, cy, btn_w, btn_h);
    if working {
        // Shown greyed out, with no zone: it cannot be pressed.
        draw_button(
            painter,
            text,
            confirm_rect,
            "Format",
            false,
            pal,
            ButtonStyle::Disabled,
            sw,
            sh,
            s,
        );
        return;
    }
    let confirm_state = input.add_zone(ZONE_DRIVE_DIALOG_CONFIRM, confirm_rect);
    draw_button(
        painter,
        text,
        confirm_rect,
        "Format",
        confirm_state.is_hovered(),
        pal,
        ButtonStyle::Danger,
        sw,
        sh,
        s,
    );
}

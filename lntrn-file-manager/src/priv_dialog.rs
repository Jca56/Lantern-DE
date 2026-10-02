//! Drawing of the privileged-operation modal (`priv_ops::SudoPrompt`): the
//! question before a permanent delete, the password field, and the
//! "working" state while sudo runs.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextInput, TextLabel};

use crate::dialogs::{draw_button, draw_overlay_with_scrim, wrap_lines, ButtonStyle};
use crate::op_dialogs::fit_middle;
use crate::priv_ops::{doomed_lines, Phase, SudoPrompt};
use crate::{
    ZONE_SUDO_CANCEL, ZONE_SUDO_PANEL, ZONE_SUDO_PASSWORD, ZONE_SUDO_SCRIM, ZONE_SUDO_SUBMIT,
};

/// One line of the dialog body.
struct Line {
    text: String,
    /// A path or a name: shortened from the middle, never wrapped.
    is_path: bool,
    danger: bool,
}

fn plain(text: impl Into<String>) -> Line {
    Line {
        text: text.into(),
        is_path: false,
        danger: false,
    }
}

pub fn draw_sudo_prompt(
    dialog: &SudoPrompt,
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

    let pad = 28.0 * s;
    let cr = 12.0 * s;
    let title_font = 24.0 * s;
    let body_font = 18.0 * s;
    let line_gap = 8.0 * s;
    let line_h = body_font + line_gap;
    let field_h = 52.0 * s;
    let btn_h = 52.0 * s;
    let btn_gap = 14.0 * s;
    let dialog_w = (660.0 * s).min(screen_w - 32.0 * s).max(200.0 * s);
    let text_w = dialog_w - pad * 2.0;

    let doomed = dialog.op.doomed();
    // (title, body, acting button)
    let (title, mut lines, act): (String, Vec<Line>, Option<(&str, ButtonStyle)>) =
        match dialog.phase {
            // Not a delete: an operation in a folder root mode is not on for.
            Phase::Confirm if doomed.is_empty() => (
                "This needs administrator rights".to_string(),
                vec![
                    plain(format!("{}.", dialog.op.description())),
                    plain(""),
                    plain(
                        "Your own account is not allowed to do this. It can be done as the \
                         administrator (root). Root mode is not on for every folder this \
                         changes, so nothing is done as administrator without your say.",
                    ),
                ],
                Some(("Continue as Administrator", ButtonStyle::Secondary)),
            ),
            Phase::Confirm => {
                let title = match doomed {
                    [one] => format!(
                        "Delete \u{201C}{}\u{201D} permanently as administrator?",
                        crate::sudo::name_of(one)
                    ),
                    _ => format!(
                        "Delete {} items permanently as administrator?",
                        doomed.len()
                    ),
                };
                let mut lines: Vec<Line> = doomed_lines(doomed)
                    .into_iter()
                    .map(|text| Line {
                        text,
                        is_path: true,
                        danger: false,
                    })
                    .collect();
                lines.push(plain(""));
                lines.push(plain(
                    "Your own account is not allowed to delete this. The administrator \
                     account (root) is. Nothing goes to the Trash and this cannot be undone.",
                ));
                (
                    title,
                    lines,
                    Some(("Delete Permanently", ButtonStyle::Danger)),
                )
            }
            Phase::Password => {
                let mut lines = vec![plain(format!("{}.", dialog.op.description()))];
                // What is deleted stays named while the password is typed.
                if doomed.len() > 1 {
                    lines.extend(doomed_lines(doomed).into_iter().map(|text| Line {
                        text,
                        is_path: true,
                        danger: false,
                    }));
                }
                let style = if dialog.can_submit() {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Disabled
                };
                (
                    "Administrator password required".to_string(),
                    lines,
                    Some(("Authenticate", style)),
                )
            }
            Phase::Working => (
                format!("Working as administrator{}", dialog.working_dots()),
                vec![
                    plain(format!("{}.", dialog.op.description())),
                    plain(""),
                    plain("It cannot be interrupted. This closes by itself when it is done."),
                ],
                None,
            ),
        };
    if let Some(err) = dialog.error.as_ref().filter(|_| dialog.phase == Phase::Password) {
        lines.push(Line {
            text: err.clone(),
            is_path: false,
            danger: true,
        });
    }

    // Wrap prose; a path keeps its one line.
    let mut body: Vec<Line> = Vec::new();
    for line in lines {
        if line.is_path {
            body.push(line);
        } else {
            for part in wrap_lines(text, &line.text, body_font, text_w) {
                body.push(Line {
                    text: part,
                    is_path: false,
                    danger: line.danger,
                });
            }
        }
    }
    let title_lines = wrap_lines(text, &title, title_font, text_w);
    let title_h = (title_font + line_gap) * title_lines.len() as f32;
    let has_field = dialog.phase == Phase::Password;
    let field_block = if has_field {
        body_font + 6.0 * s + field_h + line_gap
    } else {
        0.0
    };
    let buttons_h = if dialog.phase == Phase::Working {
        0.0
    } else {
        pad + btn_h
    };

    // As many body lines as the window has room for.
    let room = screen_h - 32.0 * s - pad * 2.0 - title_h - pad * 0.4 - field_block - buttons_h;
    let max_lines = ((room / line_h).floor() as usize).max(1);
    if body.len() > max_lines {
        body.truncate(max_lines.saturating_sub(1));
        body.push(plain("\u{2026}"));
    }

    let dialog_h =
        pad * 2.0 + title_h + pad * 0.4 + line_h * body.len() as f32 + field_block + buttons_h;
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
        ZONE_SUDO_SCRIM,
    );
    // The panel is its own zone so a click on its text is not a click
    // "outside" that cancels.
    input.add_zone(ZONE_SUDO_PANEL, Rect::new(dx, dy, dialog_w, dialog_h));

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
        let shown = fit_middle(text, &line.text, body_font, text_w);
        let color = if line.danger {
            pal.danger
        } else if line.is_path {
            pal.text
        } else {
            pal.text_secondary
        };
        TextLabel::new(&shown, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(color)
            .max_width(text_w + 4.0 * s)
            .draw(text, sw, sh);
        cy += line_h;
    }

    if has_field {
        TextLabel::new("Password", dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.text_secondary)
            .max_width(text_w)
            .draw(text, sw, sh);
        cy += body_font + 6.0 * s;
        let pw_rect = Rect::new(dx + pad, cy, text_w, field_h);
        input.add_zone(ZONE_SUDO_PASSWORD, pw_rect);
        let masked: String = "\u{2022}".repeat(dialog.password.chars().count());
        TextInput::new(pw_rect)
            .text(&masked)
            .placeholder("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}")
            .focused(true)
            .cursor_pos(dialog.cursor)
            .scale(s)
            .draw(painter, text, pal, sw, sh);
        cy += field_h + line_gap;
    }

    let Some((act_label, act_style)) = act else {
        return;
    };
    cy += pad - line_gap;

    // Generous buttons, sized to their labels. The acting one sits on the
    // right; Cancel on its left.
    let btn_font = 18.0 * s;
    let width_for = |text: &mut TextRenderer, label: &str| {
        (text.measure_width(label, btn_font) + 56.0 * s).max(170.0 * s)
    };
    let mut right = dx + dialog_w - pad;
    let w = width_for(text, act_label);
    let act_rect = Rect::new(right - w, cy, w, btn_h);
    if matches!(act_style, ButtonStyle::Disabled) {
        // Greyed out, with no zone: it cannot be pressed.
        draw_button(painter, text, act_rect, act_label, false, pal, act_style, sw, sh, s);
    } else {
        let hovered = input.add_zone(ZONE_SUDO_SUBMIT, act_rect).is_hovered();
        draw_button(painter, text, act_rect, act_label, hovered, pal, act_style, sw, sh, s);
    }
    right -= w + btn_gap;

    let w = width_for(text, "Cancel");
    let cancel_rect = Rect::new(right - w, cy, w, btn_h);
    let hovered = input.add_zone(ZONE_SUDO_CANCEL, cancel_rect).is_hovered();
    // On the delete question Cancel is the button to reach for.
    let cancel_style = if dialog.phase == Phase::Confirm {
        ButtonStyle::Primary
    } else {
        ButtonStyle::Secondary
    };
    draw_button(painter, text, cancel_rect, "Cancel", hovered, pal, cancel_style, sw, sh, s);
}

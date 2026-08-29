//! Canvas-mode modal dialogs: save-name prompt, unsaved-changes confirms,
//! and error boxes. Drawn on the overlay layer above everything else.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, FontSize, FoxPalette, InteractionContext, TextInput, TextLabel,
};

use crate::canvas::editor::{CanvasEditor, DialogKind};
use crate::{ZONE_DIALOG_BACKDROP, ZONE_DIALOG_BTN0, ZONE_DIALOG_BTN1, ZONE_DIALOG_BTN2};

#[allow(clippy::too_many_arguments)]
pub fn draw_dialog(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    editor: &CanvasEditor,
    palette: &FoxPalette,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let Some(dialog) = &editor.dialog else { return };

    // Backdrop eats every click that isn't a dialog button.
    input.add_zone(ZONE_DIALOG_BACKDROP, Rect::new(0.0, 0.0, wf, hf));
    painter.rect_filled(
        Rect::new(0.0, 0.0, wf, hf),
        0.0,
        Color::BLACK.with_alpha(0.55),
    );

    let (title, body, buttons): (&str, String, Vec<(u32, &str, bool)>) = match dialog {
        DialogKind::SaveName { .. } => (
            "Save canvas",
            "Name this canvas:".into(),
            vec![
                (ZONE_DIALOG_BTN0, "Save", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
        ),
        DialogKind::ConfirmQuit => (
            "Unsaved changes",
            "Save your canvas before quitting?".into(),
            vec![
                (ZONE_DIALOG_BTN0, "Save", true),
                (ZONE_DIALOG_BTN1, "Discard", false),
                (ZONE_DIALOG_BTN2, "Cancel", false),
            ],
        ),
        DialogKind::ConfirmNew => (
            "Unsaved changes",
            "Start a new canvas and discard changes?".into(),
            vec![
                (ZONE_DIALOG_BTN0, "Discard & New", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
        ),
        DialogKind::Error(msg) => ("Error", msg.clone(), vec![(ZONE_DIALOG_BTN0, "OK", true)]),
    };
    let has_input = matches!(dialog, DialogKind::SaveName { .. });

    let pad = 24.0 * s;
    let title_px = FontSize::Body.px() * s;
    let body_px = FontSize::Small.px() * s;
    let input_h = if has_input { 52.0 * s + 16.0 * s } else { 0.0 };
    let btn_h = 48.0 * s;
    let pw = 560.0_f32.min(wf / s - 40.0) * s;
    let ph = pad * 2.0 + title_px + 12.0 * s + body_px * 2.0 + input_h + 20.0 * s + btn_h;
    let panel = Rect::new((wf - pw) * 0.5, (hf - ph) * 0.5, pw, ph);

    painter.rect_filled(
        panel.expand(8.0 * s),
        16.0 * s,
        Color::BLACK.with_alpha(0.3),
    );
    painter.rect_filled(panel, 12.0 * s, palette.surface);
    painter.rect_stroke(panel, 12.0 * s, 1.0, palette.muted.with_alpha(0.25));

    let mut y = panel.y + pad;
    TextLabel::new(title, panel.x + pad, y)
        .size(FontSize::Custom(title_px))
        .bold()
        .color(palette.text)
        .draw(text, sw, sh);
    y += title_px + 12.0 * s;
    TextLabel::new(&body, panel.x + pad, y)
        .size(FontSize::Custom(body_px))
        .color(palette.text_secondary)
        .max_width(panel.w - pad * 2.0)
        .draw(text, sw, sh);
    y += body_px * 2.0;

    if has_input {
        let field = Rect::new(panel.x + pad, y, panel.w - pad * 2.0, 52.0 * s);
        TextInput::new(field)
            .text(&editor.name_buf)
            .placeholder("My collage")
            .focused(true)
            .cursor_pos(editor.name_cursor)
            .scale(s)
            .draw(painter, text, palette, sw, sh);
    }

    let btn_w = 170.0 * s;
    let gap = 12.0 * s;
    let by = panel.y + panel.h - pad - btn_h;
    let mut bx = panel.x + panel.w - pad - btn_w;
    for (zone, label, primary) in buttons.iter().rev() {
        let rect = Rect::new(bx, by, btn_w, btn_h);
        let st = input.add_zone(*zone, rect);
        Button::new(rect, label)
            .variant(if *primary {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Default
            })
            .hovered(st.is_hovered())
            .pressed(st.is_active())
            .scale(s)
            .draw(painter, text, palette, sw, sh);
        bx -= btn_w + gap;
    }
}

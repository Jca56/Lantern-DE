//! Modal dialogs, drawn on the overlay layer above everything else. One
//! generic box (`draw_dialog_box`) serves both the canvas editor's flows
//! (save-name prompt, unsaved-changes confirms, errors) and the viewer's
//! trash confirmation.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, FontSize, FoxPalette, InteractionContext, TextInput, TextLabel,
};

use crate::app::ViewerDialog;
use crate::canvas::editor::{CanvasEditor, DialogKind};
use crate::{ZONE_DIALOG_BACKDROP, ZONE_DIALOG_BTN0, ZONE_DIALOG_BTN1, ZONE_DIALOG_BTN2};

pub struct DialogSpec<'a> {
    pub title: &'a str,
    pub body: String,
    /// (zone, label, primary). Laid out right-to-left: first entry rightmost.
    pub buttons: Vec<(u32, &'a str, bool)>,
    /// Text field contents + cursor index, for prompts.
    pub name_input: Option<(&'a str, usize)>,
}

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
    let spec = match dialog {
        DialogKind::SaveName { .. } => DialogSpec {
            title: "Save canvas",
            body: "Name this canvas:".into(),
            buttons: vec![
                (ZONE_DIALOG_BTN0, "Save", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
            name_input: Some((&editor.name_buf, editor.name_cursor)),
        },
        DialogKind::ConfirmQuit => DialogSpec {
            title: "Unsaved changes",
            body: "Save your canvas before quitting?".into(),
            buttons: vec![
                (ZONE_DIALOG_BTN0, "Save", true),
                (ZONE_DIALOG_BTN1, "Discard", false),
                (ZONE_DIALOG_BTN2, "Cancel", false),
            ],
            name_input: None,
        },
        DialogKind::ConfirmNew => DialogSpec {
            title: "Unsaved changes",
            body: "Start a new canvas and discard changes?".into(),
            buttons: vec![
                (ZONE_DIALOG_BTN0, "Discard & New", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
            name_input: None,
        },
        DialogKind::Error(msg) => DialogSpec {
            title: "Error",
            body: msg.clone(),
            buttons: vec![(ZONE_DIALOG_BTN0, "OK", true)],
            name_input: None,
        },
    };
    draw_dialog_box(painter, text, input, &spec, palette, wf, hf, s, sw, sh);
}

#[allow(clippy::too_many_arguments)]
pub fn draw_viewer_dialog(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    dialog: &ViewerDialog,
    palette: &FoxPalette,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let spec = match dialog {
        ViewerDialog::ConfirmTrash(path) => DialogSpec {
            title: "Move to trash?",
            body: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            buttons: vec![
                (ZONE_DIALOG_BTN0, "Trash", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
            name_input: None,
        },
        ViewerDialog::Error(msg) => DialogSpec {
            title: "Couldn't do that",
            body: msg.clone(),
            buttons: vec![(ZONE_DIALOG_BTN0, "OK", true)],
            name_input: None,
        },
    };
    draw_dialog_box(painter, text, input, &spec, palette, wf, hf, s, sw, sh);
}

#[allow(clippy::too_many_arguments)]
pub fn draw_dialog_box(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    spec: &DialogSpec,
    palette: &FoxPalette,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    // Backdrop eats every click that isn't a dialog button.
    input.add_zone(ZONE_DIALOG_BACKDROP, Rect::new(0.0, 0.0, wf, hf));
    painter.rect_filled(
        Rect::new(0.0, 0.0, wf, hf),
        0.0,
        Color::BLACK.with_alpha(0.55),
    );

    let has_input = spec.name_input.is_some();
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
    TextLabel::new(spec.title, panel.x + pad, y)
        .size(FontSize::Custom(title_px))
        .bold()
        .color(palette.text)
        .draw(text, sw, sh);
    y += title_px + 12.0 * s;
    TextLabel::new(&spec.body, panel.x + pad, y)
        .size(FontSize::Custom(body_px))
        .color(palette.text_secondary)
        .max_width(panel.w - pad * 2.0)
        .draw(text, sw, sh);
    y += body_px * 2.0;

    if let Some((buf, cursor)) = spec.name_input {
        let field = Rect::new(panel.x + pad, y, panel.w - pad * 2.0, 52.0 * s);
        TextInput::new(field)
            .text(buf)
            .placeholder("My collage")
            .focused(true)
            .cursor_pos(cursor)
            .scale(s)
            .draw(painter, text, palette, sw, sh);
    }

    let btn_w = 170.0 * s;
    let gap = 12.0 * s;
    let by = panel.y + panel.h - pad - btn_h;
    let mut bx = panel.x + panel.w - pad - btn_w;
    for (zone, label, primary) in spec.buttons.iter().rev() {
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

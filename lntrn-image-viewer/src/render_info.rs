//! The info overlay (I key): a translucent card in the picture's top-right
//! corner listing file facts and camera EXIF. Drawn on the overlay layer so
//! it floats above the image.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::app::App;

#[allow(clippy::too_many_arguments)]
pub fn draw_info_overlay(
    painter: &mut Painter,
    text: &mut TextRenderer,
    app: &App,
    palette: &FoxPalette,
    canvas: Rect,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let Some(info) = &app.info else { return };

    let pad = 18.0 * s;
    let margin = 16.0 * s;
    let label_px = FontSize::Small.px() * s;
    let value_px = FontSize::Body.px() * s;
    let title_px = FontSize::Body.px() * s;
    let row_h = label_px + value_px + 12.0 * s;

    let mut rows: Vec<(&str, String)> = vec![("File", app.file_name.clone())];
    if let Some(dir) = app.path.as_ref().and_then(|p| p.parent()) {
        rows.push(("Folder", dir.to_string_lossy().into_owned()));
    }
    rows.push(("Dimensions", app.dimensions_text.clone()));
    if app.dir_files.len() > 1 {
        rows.push((
            "Position",
            format!("{} of {}", app.dir_index + 1, app.dir_files.len()),
        ));
    }
    rows.extend(info.rows());

    let panel_w = (440.0 * s).min(canvas.w - margin * 2.0).max(120.0 * s);
    let wanted_h = pad * 2.0 + title_px + 12.0 * s + rows.len() as f32 * row_h;
    let panel_h = wanted_h.min(canvas.h - margin * 2.0).max(60.0 * s);
    let panel = Rect::new(
        canvas.x + canvas.w - margin - panel_w,
        canvas.y + margin,
        panel_w,
        panel_h,
    );

    painter.rect_filled(
        panel.expand(6.0 * s),
        16.0 * s,
        Color::BLACK.with_alpha(0.25),
    );
    painter.rect_filled(panel, 12.0 * s, palette.surface.with_alpha(0.94));
    painter.rect_stroke(panel, 12.0 * s, 1.0, palette.muted.with_alpha(0.3));

    let x = panel.x + pad;
    let max_w = panel.w - pad * 2.0;
    let mut y = panel.y + pad;

    TextLabel::new("Info", x, y)
        .size(FontSize::Custom(title_px))
        .bold()
        .color(palette.text)
        .draw(text, sw, sh);
    let hint = "I to close";
    let hint_w = text.measure_width(hint, label_px);
    TextLabel::new(
        hint,
        panel.x + panel.w - pad - hint_w,
        y + (title_px - label_px) * 0.5,
    )
    .size(FontSize::Custom(label_px))
    .color(palette.text_secondary)
    .max_width(hint_w + 12.0 * s)
    .draw(text, sw, sh);
    y += title_px + 12.0 * s;

    painter.push_clip(panel);
    text.push_clip([panel.x, panel.y, panel.w, panel.h]);
    for (label, value) in &rows {
        if y + row_h > panel.y + panel.h - pad * 0.5 {
            break;
        }
        TextLabel::new(label, x, y)
            .size(FontSize::Custom(label_px))
            .color(palette.text_secondary)
            .max_width(max_w)
            .draw(text, sw, sh);
        TextLabel::new(value, x, y + label_px + 2.0 * s)
            .size(FontSize::Custom(value_px))
            .color(palette.text)
            .max_width(max_w)
            .draw(text, sw, sh);
        y += row_h;
    }
    text.pop_clip();
    painter.pop_clip();
}

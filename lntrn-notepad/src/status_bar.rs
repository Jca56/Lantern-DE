//! Bottom status bar — filename, cursor position, line/word/char counts.
//! Pulled out of `render.rs` to keep that file under the size limit.

use lntrn_render::{Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::editor::Editor;
use crate::render::STATUS_BAR_H;
use crate::theme::Theme;
use lntrn_render::Painter;

pub fn draw_status_bar(
    editor: &Editor,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    theme: Theme,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let status_h = STATUS_BAR_H * s;
    let status_y = hf - status_h;
    // Footer plate — bookends the window with the top ribbon.
    painter.rect_filled(Rect::new(0.0, status_y, wf, status_h), 0.0, palette.sidebar);
    // Top hairline seam.
    painter.rect_filled(
        Rect::new(0.0, status_y, wf, 1.0 * s),
        0.0,
        theme.hairline_faint(),
    );

    let font_px = 16.0 * s;
    let font = FontSize::Custom(font_px);
    let label_y = status_y + (status_h - font_px) * 0.5;

    // ── Left: filename + modified marker ──────────────────────────
    let filename_label = if editor.modified {
        format!("{} •", editor.filename)
    } else {
        editor.filename.clone()
    };
    TextLabel::new(&filename_label, 14.0 * s, label_y)
        .size(font)
        .color(palette.text)
        .draw(text, sw, sh);

    // ── Right: word / character counts (selection-aware) ───────────
    let pos_text = if let Some(selected) = editor.selected_text() {
        let words = selected.split_whitespace().count();
        let chars = selected.chars().count();
        format!(
            "{} {} selected · {} {}",
            words,
            if words == 1 { "word" } else { "words" },
            chars,
            if chars == 1 { "char" } else { "chars" },
        )
    } else {
        let words: usize = editor
            .lines
            .iter()
            .map(|l| l.split_whitespace().count())
            .sum();
        let chars: usize = editor.lines.iter().map(|l| l.chars().count()).sum::<usize>()
            + editor.lines.len().saturating_sub(1);
        let chars_label = if chars >= 1000 {
            format!("{:.1}k chars", chars as f32 / 1000.0)
        } else {
            format!("{} chars", chars)
        };
        format!("{} words · {}", words, chars_label)
    };
    let pos_w = text.measure_width(&pos_text, font_px);
    // Word-style count chip: a faint rounded pill behind the counts.
    let pill_pad_x = 10.0 * s;
    let pill_h = font_px + 8.0 * s;
    let pill_x = wf - pos_w - pill_pad_x * 2.0 - 12.0 * s;
    let pill_y = status_y + (status_h - pill_h) * 0.5;
    painter.rect_filled(
        Rect::new(pill_x, pill_y, pos_w + pill_pad_x * 2.0, pill_h),
        pill_h * 0.5,
        palette.surface_2.with_alpha(0.6),
    );
    TextLabel::new(&pos_text, pill_x + pill_pad_x, label_y)
        .size(font)
        .color(palette.text_secondary)
        .draw(text, sw, sh);
}

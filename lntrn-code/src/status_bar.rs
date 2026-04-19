//! Bottom status bar — filename, cursor position, line/word/char counts.
//! Pulled out of `render.rs` to keep that file under the size limit.

use lntrn_render::{Color, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::editor::Editor;
use crate::render::STATUS_BAR_H;
use lntrn_render::Painter;

/// `diagnostics`: `(errors, warnings, infos, hints)` from the LSP for the
/// active file. `lsp_status`: last window/logMessage string (e.g. indexing
/// progress). Both pass through unchanged when LSP isn't live.
pub fn draw_status_bar(
    editor: &Editor,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
    diagnostics: (usize, usize, usize, usize),
    lsp_status: &str,
) {
    let status_h = STATUS_BAR_H * s;
    let status_y = hf - status_h;
    // Top hairline border.
    painter.rect_filled(
        Rect::new(0.0, status_y, wf, 1.0 * s),
        0.0,
        Color::from_rgba8(60, 50, 35, 22),
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

    // ── Left-middle: diagnostic counts + LSP status string ────────
    let (errs, warns, _infos, _hints) = diagnostics;
    let mut tail_x = 14.0 * s + text.measure_width(&filename_label, font_px) + 20.0 * s;
    if errs > 0 {
        let t = format!("\u{2716} {errs}");
        TextLabel::new(&t, tail_x, label_y)
            .size(font)
            .color(palette.danger)
            .draw(text, sw, sh);
        tail_x += text.measure_width(&t, font_px) + 14.0 * s;
    }
    if warns > 0 {
        let t = format!("\u{26A0} {warns}");
        TextLabel::new(&t, tail_x, label_y)
            .size(font)
            .color(palette.warning)
            .draw(text, sw, sh);
        tail_x += text.measure_width(&t, font_px) + 14.0 * s;
    }
    if !lsp_status.is_empty() {
        // Truncate long status lines so they don't push past the counts.
        let max_w = wf * 0.45;
        let mut status = lsp_status.to_string();
        while text.measure_width(&status, font_px) > max_w && status.len() > 4 {
            status.pop();
        }
        TextLabel::new(&status, tail_x, label_y)
            .size(font)
            .color(palette.text_secondary)
            .draw(text, sw, sh);
    }

    // ── Right: cursor position + counts (selection-aware) ─────────
    let pos_text = if let Some(selected) = editor.selected_text() {
        let words = selected.split_whitespace().count();
        let chars = selected.chars().count();
        format!(
            "Ln {} Col {}  ·  {} {} selected · {} {}",
            editor.cursor_line + 1,
            editor.cursor_col + 1,
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
        format!(
            "Ln {} Col {}  ·  {} lines · {} words · {}",
            editor.cursor_line + 1,
            editor.cursor_col + 1,
            editor.lines.len(),
            words,
            chars_label,
        )
    };
    let pos_w = text.measure_width(&pos_text, font_px);
    TextLabel::new(&pos_text, wf - pos_w - 14.0 * s, label_y)
        .size(font)
        .color(palette.text_secondary)
        .draw(text, sw, sh);
}

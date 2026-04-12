//! Word-wrap and per-span text measurement helpers. Pulled out of `render.rs`
//! to keep that file under the size limit.

use lntrn_render::{FontStyle, FontWeight, TextRenderer};

use crate::editor::Editor;
use crate::format::FormatSpan;

/// Convert a `FormatSpan`'s attrs into `(font_size, FontWeight, FontStyle)`.
pub fn span_rendering(
    span: &FormatSpan,
    default_font_size: f32,
) -> (f32, FontWeight, FontStyle) {
    let fs = span.attrs.font_size.unwrap_or(default_font_size);
    let weight = if span.attrs.bold {
        FontWeight::Bold
    } else {
        FontWeight::Normal
    };
    let style = if span.attrs.italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    (fs, weight, style)
}

/// Measure the x-offset from the line start to a byte offset within a line,
/// accounting for per-span font size and weight/style.
pub fn measure_to_offset(
    text: &mut TextRenderer,
    editor: &Editor,
    line: usize,
    byte_offset: usize,
    default_font_size: f32,
) -> f32 {
    if byte_offset == 0 {
        return 0.0;
    }
    let line_str = &editor.lines[line];
    let spans = editor.formats.get(line).iter_spans(line_str.len());
    let mut x = 0.0;
    for span in &spans {
        if span.start >= byte_offset {
            break;
        }
        let end = span.end.min(byte_offset);
        let span_text = &line_str[span.start..end];
        if !span_text.is_empty() {
            let (fs, weight, style) = span_rendering(span, default_font_size);
            x += text.measure_width_styled(span_text, fs, weight, style);
        }
        if span.end >= byte_offset {
            break;
        }
    }
    x
}

/// Measure the pixel width of a byte range within a line.
pub fn measure_range(
    text: &mut TextRenderer,
    editor: &Editor,
    line: usize,
    from: usize,
    to: usize,
    default_font_size: f32,
) -> f32 {
    if from >= to {
        return 0.0;
    }
    measure_to_offset(text, editor, line, to, default_font_size)
        - measure_to_offset(text, editor, line, from, default_font_size)
}

/// Compute word-wrap break points for a single document line. Returns byte
/// offsets where each visual row starts (the first is always 0).
pub fn compute_line_wraps(
    text: &mut TextRenderer,
    editor: &Editor,
    line_idx: usize,
    max_width: f32,
    default_font_size: f32,
) -> Vec<usize> {
    let line_str = &editor.lines[line_idx];
    if line_str.is_empty() || max_width <= 0.0 {
        return vec![0];
    }

    let spans = editor.formats.get(line_idx).iter_spans(line_str.len());
    let mut row_starts: Vec<usize> = vec![0];
    let mut row_x: f32 = 0.0;
    let mut last_space: Option<(usize, f32)> = None;

    for span in &spans {
        let (fs, weight, style) = span_rendering(span, default_font_size);
        for (rel_i, ch) in line_str[span.start..span.end].char_indices() {
            let byte_pos = span.start + rel_i;
            let ch_w = text.measure_width_styled(
                &line_str[byte_pos..byte_pos + ch.len_utf8()],
                fs,
                weight,
                style,
            );

            if row_x + ch_w > max_width && byte_pos > *row_starts.last().unwrap() {
                if let Some((sp_byte, sp_x)) = last_space {
                    if sp_byte > *row_starts.last().unwrap() {
                        row_starts.push(sp_byte);
                        row_x -= sp_x;
                    } else {
                        row_starts.push(byte_pos);
                        row_x = 0.0;
                    }
                } else {
                    row_starts.push(byte_pos);
                    row_x = 0.0;
                }
                last_space = None;
            }

            row_x += ch_w;

            if ch == ' ' {
                last_space = Some((byte_pos + 1, row_x));
            }
        }
    }

    row_starts
}

/// Recompute all word-wrap info and store on the editor.
pub fn compute_wraps(
    text: &mut TextRenderer,
    editor: &mut Editor,
    max_width: f32,
    default_font_size: f32,
) {
    if !editor.wrap_enabled {
        editor.wrap_rows = vec![vec![0]; editor.lines.len()];
        return;
    }
    let mut wraps = Vec::with_capacity(editor.lines.len());
    for i in 0..editor.lines.len() {
        wraps.push(compute_line_wraps(text, editor, i, max_width, default_font_size));
    }
    editor.wrap_rows = wraps;
}

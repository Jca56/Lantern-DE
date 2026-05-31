//! Word-wrap computation. Resolves each document line into the byte offsets
//! where its visual rows begin, honouring per-span font metrics and the
//! paragraph's first-line indent. Kept apart from `render.rs` so the frame
//! renderer stays about painting, not measuring.

use lntrn_render::TextRenderer;

use crate::editor::Editor;
use crate::render::span_rendering;

/// Recompute all word-wrap info and store it on the editor.
pub fn compute(
    text: &mut TextRenderer,
    editor: &mut Editor,
    max_width: f32,
    scale: f32,
    default_font_size: f32,
) {
    let mut wraps = Vec::with_capacity(editor.lines.len());
    for i in 0..editor.lines.len() {
        let indent_px = editor.formats.get(i).para.first_indent * scale;
        wraps.push(line_wraps(text, editor, i, max_width, indent_px, default_font_size));
    }
    editor.wrap_rows = wraps;
}

/// Compute word-wrap break points for a single document line.
/// Returns byte offsets where each visual row starts (first is always 0).
/// `first_indent_px` reduces the first row's available width.
fn line_wraps(
    text: &mut TextRenderer,
    editor: &Editor,
    line_idx: usize,
    max_width: f32,
    first_indent_px: f32,
    default_font_size: f32,
) -> Vec<usize> {
    let line_str = &editor.lines[line_idx];
    if line_str.is_empty() || max_width <= 0.0 {
        return vec![0];
    }

    let spans = editor.formats.get(line_idx).iter_spans(line_str.len());
    let mut row_starts: Vec<usize> = vec![0];
    let mut row_x: f32 = 0.0;
    // First row has reduced width for first-line indent
    let mut effective_w = (max_width - first_indent_px).max(10.0);
    let mut last_break: Option<(usize, f32)> = None; // (byte_after_break, row_x_at_that_point)

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

            if row_x + ch_w > effective_w && byte_pos > *row_starts.last().unwrap() {
                if let Some((br_byte, br_x)) = last_break {
                    if br_byte > *row_starts.last().unwrap() {
                        row_starts.push(br_byte);
                        row_x -= br_x;
                    } else {
                        row_starts.push(byte_pos);
                        row_x = 0.0;
                    }
                } else {
                    row_starts.push(byte_pos);
                    row_x = 0.0;
                }
                last_break = None;
                // Subsequent rows get full width (no indent)
                effective_w = max_width;
            }

            row_x += ch_w;

            // Track word-boundary break points: spaces and hyphens
            if ch == ' ' || ch == '-' {
                last_break = Some((byte_pos + ch.len_utf8(), row_x));
            }
        }
    }

    row_starts
}

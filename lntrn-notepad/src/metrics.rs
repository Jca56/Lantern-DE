//! Editor layout metrics — row heights, total content height, and the
//! y/x → document-position mapping used by clicks and scrolling. Pulled out
//! of `editor.rs` so that file stays focused on state + editing ops.

use crate::editor::{Editor, FONT_SIZE, LINE_HEIGHT, PAD};

impl Editor {
    /// The largest font size (logical px, unscaled) used by any span within the
    /// byte range `[start, end)` of `line`. Falls back to the default size when
    /// the range is empty or has no size overrides. This is what drives a wrap
    /// row's height so larger text gets a taller row (no overlap).
    pub fn row_font_size(&self, line: usize, start: usize, end: usize) -> f32 {
        let line_len = self.lines[line].len();
        let mut max_fs = FONT_SIZE;
        // An empty row (blank line) still uses any pending/size at col 0.
        for span in self.formats.get(line).iter_spans(line_len) {
            if span.end <= start || span.start >= end {
                continue;
            }
            if let Some(fs) = span.attrs.font_size {
                if fs > max_fs {
                    max_fs = fs;
                }
            }
        }
        max_fs
    }

    /// Physical height of a single wrap row: its max span font size ×
    /// line spacing × scale.
    pub fn row_height(&self, line: usize, start: usize, end: usize, scale: f32) -> f32 {
        let para = self.formats.get(line).para;
        self.row_font_size(line, start, end) * para.line_spacing * scale
    }

    /// Total content height in physical pixels (accounts for word-wrap rows,
    /// per-row font size, and spacing before+after each paragraph).
    pub fn content_height(&self, scale: f32) -> f32 {
        let mut h = PAD * scale * 2.0;
        if self.wrap_rows.len() == self.lines.len() {
            for (i, wraps) in self.wrap_rows.iter().enumerate() {
                let para = self.formats.get(i).para;
                let line_len = self.lines[i].len();
                for (row_idx, &row_start) in wraps.iter().enumerate() {
                    let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_len);
                    h += self.row_height(i, row_start, row_end, scale);
                }
                h += (para.space_before + para.space_after) * scale;
            }
        } else {
            let row_h = FONT_SIZE * LINE_HEIGHT * scale;
            h += self.lines.len() as f32 * row_h;
        }
        h
    }

    /// Resolve which doc line and wrap-row byte range a click y falls on.
    /// Returns `(doc_line, row_start_byte, row_end_byte)`.
    pub fn wrap_row_at_y(
        &self,
        cy: f32,
        editor_rect: lntrn_render::Rect,
        scale: f32,
    ) -> (usize, usize, usize) {
        let text_y_start = editor_rect.y + PAD * scale * 1.5 - self.scroll_offset;
        let mut y = text_y_start;

        for (i, wraps) in self.wrap_rows.iter().enumerate() {
            let para = self.formats.get(i).para;
            let line_len = self.lines[i].len();
            y += para.space_before * scale;
            for (row_idx, &row_start) in wraps.iter().enumerate() {
                let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_len);
                let row_h = self.row_height(i, row_start, row_end, scale);
                if cy < y + row_h {
                    return (i, row_start, row_end);
                }
                y += row_h;
            }
            y += para.space_after * scale;
        }

        let last = self.lines.len() - 1;
        let last_start = *self.wrap_rows.get(last).and_then(|w| w.last()).unwrap_or(&0);
        (last, last_start, self.lines[last].len())
    }

    /// Find the byte column closest to click x within a wrap-row byte range.
    /// `content_x` is the pixel x where text starts (accounts for page
    /// centering, padding, alignment offset, and first-line indent).
    pub fn col_at_x(
        &self,
        cx: f32,
        line_idx: usize,
        row_start: usize,
        row_end: usize,
        content_x: f32,
        mut measure_fn: impl FnMut(usize) -> f32,
    ) -> usize {
        let rel_x = (cx - content_x).max(0.0);

        let line = &self.lines[line_idx];
        let char_offsets: Vec<usize> = line[row_start..row_end]
            .char_indices()
            .map(|(i, _)| row_start + i)
            .chain(std::iter::once(row_end))
            .collect();

        let mut best_col = row_start;
        let mut best_dist = f32::MAX;
        for &byte_off in &char_offsets {
            let dist = (measure_fn(byte_off) - rel_x).abs();
            if dist < best_dist {
                best_dist = dist;
                best_col = byte_off;
            }
        }
        best_col
    }
}

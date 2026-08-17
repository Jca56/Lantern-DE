//! Editor layout queries — content height, and the y/x → document-position
//! mapping used by clicks and scrolling. Everything here reads the per-line
//! cache in `layout.rs`; nothing measures text. Pulled out of `editor.rs` so
//! that file stays focused on state + editing ops.

use crate::editor::{Editor, FONT_SIZE, PAD};
use crate::layout::LineLayout;

/// Clamp `i` into `line` and walk back to the nearest char boundary.
fn snap_floor(line: &str, i: usize) -> usize {
    let mut i = i.min(line.len());
    while i > 0 && !line.is_char_boundary(i) {
        i -= 1;
    }
    i
}

impl Editor {
    /// The largest font size (logical px, unscaled) used by any span within the
    /// byte range `[start, end)` of `line`. Falls back to the default size when
    /// the range is empty or has no size overrides. Drives caret height and
    /// bullet size; row heights come pre-computed from the layout cache.
    pub fn row_font_size(&self, line: usize, start: usize, end: usize) -> f32 {
        self.formats
            .get(line)
            .max_font_size_in(start, end)
            .map_or(FONT_SIZE, |fs| fs.max(FONT_SIZE))
    }

    /// The layout of `line`, or `None` if the cache has not caught up with an
    /// edit yet (input events can land between an edit and the next frame).
    pub fn line_layout(&self, line: usize) -> Option<&LineLayout> {
        if line >= self.lines.len() {
            return None;
        }
        self.layout.get(line)
    }

    /// How many lines have valid layout. Edits grow `lines` before the next
    /// frame rebuilds `layout`, so every walk stops here.
    pub fn laid_out_lines(&self) -> usize {
        self.lines.len().min(self.layout.len())
    }

    /// Total content height in physical pixels. O(1) — the stacking pass keeps
    /// the running total, which used to be a full-document walk per query (and
    /// the scrollbar asks for it every single frame).
    pub fn content_height(&self, scale: f32) -> f32 {
        PAD * scale * 2.0 + self.total_h
    }

    /// Screen y where the document's first row starts, at the current scroll.
    pub fn text_origin_y(&self, editor_rect: lntrn_render::Rect, scale: f32) -> f32 {
        editor_rect.y + PAD * scale * 1.5 - self.scroll_offset
    }

    /// Index of the first line whose bottom edge reaches `y_doc` (doc-space y,
    /// measured from the text origin). Binary search over the stacked tops.
    pub fn line_at_doc_y(&self, y_doc: f32) -> usize {
        let n = self.laid_out_lines();
        if n == 0 {
            return 0;
        }
        self.layout[..n]
            .partition_point(|l| l.top + l.height < y_doc)
            .min(n - 1)
    }

    /// Index of the first line that starts below `y_doc` — the exclusive end of
    /// a visible range.
    pub fn line_after_doc_y(&self, y_doc: f32) -> usize {
        let n = self.laid_out_lines();
        self.layout[..n].partition_point(|l| l.top <= y_doc)
    }

    /// Resolve which doc line and wrap-row byte range a click y falls on.
    /// Returns `(doc_line, row_start_byte, row_end_byte)`.
    pub fn wrap_row_at_y(
        &self,
        cy: f32,
        editor_rect: lntrn_render::Rect,
        scale: f32,
    ) -> (usize, usize, usize) {
        let n = self.laid_out_lines();
        if n == 0 {
            let last = self.lines.len().saturating_sub(1);
            return (last, 0, self.lines[last].len());
        }
        let y_doc = cy - self.text_origin_y(editor_rect, scale);
        let i = self.line_at_doc_y(y_doc);
        let line = &self.lines[i];
        let l = &self.layout[i];

        let mut ry = l.top;
        let last_row = l.row_count().saturating_sub(1);
        for idx in 0..l.row_count() {
            let h = l.row_h[idx];
            if y_doc < ry + h || idx == last_row {
                let (s, e) = l.row_range(idx, line.len());
                let s = snap_floor(line, s);
                return (i, s, snap_floor(line, e).max(s));
            }
            ry += h;
        }
        (i, 0, line.len())
    }

    /// Byte column nearest to `rel_x` pixels from the row's text start.
    pub fn col_at_x(&self, rel_x: f32, line_idx: usize, row_start: usize, row_end: usize) -> usize {
        let Some(l) = self.line_layout(line_idx) else {
            return row_start;
        };
        let line = &self.lines[line_idx];
        let start = snap_floor(line, row_start);
        let end = snap_floor(line, row_end).max(start);
        snap_floor(line, l.col_at_x(rel_x, start, end))
    }

    /// The wrap row the caret sits on, as `(row_index, row_start, row_end)`.
    pub fn caret_row(&self) -> (usize, usize, usize) {
        let line = self.cursor_line.min(self.lines.len().saturating_sub(1));
        let line_len = self.lines[line].len();
        match self.line_layout(line) {
            Some(l) => {
                let idx = l.row_at(self.cursor_col);
                let (s, e) = l.row_range(idx, line_len);
                (idx, s, e)
            }
            None => (0, 0, line_len),
        }
    }
}

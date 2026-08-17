//! Editor layout: the per-line cache that every pixel measurement reads from.
//!
//! One shaping pass per line produces *everything* geometric about it — the
//! cumulative advance at each cluster boundary, where its wrap rows start, how
//! wide and tall each row is, and where the line sits in the document. Nothing
//! downstream measures text again: carets, selection rectangles, alignment,
//! hit-testing and scrolling are all array lookups or binary searches.
//!
//! That is the whole point. The previous design measured at draw time, and
//! `measure_to_offset` shaped a *prefix substring* per query — six shaped
//! prefixes per visible row per frame, each a distinct key in the text
//! engine's 512-entry layout cache. A large document with a live selection
//! evicted that cache every frame; a pointer drag re-shaped a prefix per
//! character in the row. Wrapping itself measured one character at a time,
//! which meant a full `line::build` (bidi resolve, break opportunities, row
//! emission) *per character* of the document.
//!
//! Lines are cached by content signature, so only what actually changed is
//! rebuilt. The signature covers every input to layout — text, span bounds,
//! size, weight, style, family, indent, bullet, line spacing — so no edit path
//! has to remember to invalidate anything.

use lntrn_render::TextRenderer;

use crate::editor::{Editor, BULLET_INDENT, FONT_SIZE};
use crate::render::{span_family, span_rendering};

/// Everything geometric about one document line, cached until its content or
/// formatting changes.
pub struct LineLayout {
    /// Byte offset where each visual row starts. Never empty; `rows[0]` is 0.
    pub rows: Vec<usize>,
    /// Width of each row in physical px, parallel to `rows`.
    pub row_w: Vec<f32>,
    /// Height of each row in physical px, parallel to `rows`.
    pub row_h: Vec<f32>,
    /// Cumulative advance at each cluster boundary: `(byte offset, x px)`,
    /// ascending, from `(0, 0.0)` to `(line.len(), line width)`. Offsets
    /// interior to a ligature have no entry — `x_at` interpolates them.
    pub adv: Vec<(u32, f32)>,
    /// Doc-space y of the line's first row, measured from the text origin
    /// (i.e. `space_before` already added). Filled by `restack`.
    pub top: f32,
    /// Sum of the row heights — the line's ink box, excluding para spacing.
    pub height: f32,
    /// Signature of the inputs this was built from; `None` = needs rebuild.
    sig: Option<u64>,
}

impl LineLayout {
    /// A placeholder for a line that has not been laid out yet. Shaped like a
    /// valid single empty row so an input event arriving before the next frame
    /// reads sane geometry instead of panicking.
    fn stale() -> Self {
        Self {
            rows: vec![0],
            row_w: vec![0.0],
            row_h: vec![0.0],
            adv: vec![(0, 0.0)],
            top: 0.0,
            height: 0.0,
            sig: None,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Index of the wrap row containing byte offset `col`.
    pub fn row_at(&self, col: usize) -> usize {
        self.rows.partition_point(|&s| s <= col).saturating_sub(1)
    }

    /// Byte range `[start, end)` of row `idx`. `line_len` ends the last row.
    pub fn row_range(&self, idx: usize, line_len: usize) -> (usize, usize) {
        let start = self.rows.get(idx).copied().unwrap_or(0).min(line_len);
        let end = self
            .rows
            .get(idx + 1)
            .copied()
            .unwrap_or(line_len)
            .min(line_len)
            .max(start);
        (start, end)
    }

    /// Doc-space y of row `idx` relative to the line's own top.
    pub fn row_offset_y(&self, idx: usize) -> f32 {
        self.row_h[..idx.min(self.row_h.len())].iter().sum()
    }

    /// Pixel x of byte offset `col`, measured from the line's start.
    pub fn x_at(&self, col: usize) -> f32 {
        let col = col as u32;
        match self.adv.binary_search_by_key(&col, |&(o, _)| o) {
            Ok(i) => self.adv[i].1,
            Err(0) => 0.0,
            Err(i) if i >= self.adv.len() => self.adv.last().map_or(0.0, |&(_, x)| x),
            Err(i) => {
                // Inside a ligature or mark cluster, which has no distinct pen
                // position — split it by byte fraction so the caret still moves.
                let (o0, x0) = self.adv[i - 1];
                let (o1, x1) = self.adv[i];
                let t = (col - o0) as f32 / (o1 - o0).max(1) as f32;
                x0 + (x1 - x0) * t
            }
        }
    }

    /// Width of the byte range `[from, to)`.
    pub fn width_of(&self, from: usize, to: usize) -> f32 {
        if from >= to {
            return 0.0;
        }
        self.x_at(to) - self.x_at(from)
    }

    /// Byte offset within row `[row_start, row_end)` nearest to `rel_x` pixels
    /// from the row's left edge. Binary search over the advance table — the old
    /// path re-measured the line prefix once per character, per pointer move.
    pub fn col_at_x(&self, rel_x: f32, row_start: usize, row_end: usize) -> usize {
        let target = self.x_at(row_start) + rel_x.max(0.0);
        let lo = self.adv.partition_point(|&(o, _)| (o as usize) < row_start);
        let hi = self.adv.partition_point(|&(o, _)| (o as usize) <= row_end);
        if lo >= hi {
            return row_start;
        }
        let window = &self.adv[lo..hi];
        let k = window.partition_point(|&(_, x)| x < target);
        // Nearest of the two entries straddling the target x.
        let before = window[k.saturating_sub(1)];
        let after = window[k.min(window.len() - 1)];
        let pick = if (target - before.1).abs() <= (after.1 - target).abs() {
            before.0
        } else {
            after.0
        };
        (pick as usize).clamp(row_start, row_end)
    }
}

/// Rebuild whatever is stale and restack the document. Called once per frame
/// before anything reads geometry.
pub fn compute(
    text: &mut TextRenderer,
    editor: &mut Editor,
    max_width: f32,
    scale: f32,
    default_font_size: f32,
) {
    let key = (max_width.to_bits(), scale.to_bits(), default_font_size.to_bits());
    let n = editor.lines.len();
    if editor.layout_key != Some(key) {
        // Width, scale or base size moved: every line's geometry is invalid.
        editor.layout.clear();
        editor.layout_key = Some(key);
    }
    // Resize rather than clear on a line-count change: lines *before* an
    // insertion keep matching signatures and skip the rebuild entirely.
    if editor.layout.len() != n {
        editor.layout.resize_with(n, LineLayout::stale);
    }

    let mut dirty = false;
    let mut buf: Vec<(u32, f32)> = Vec::new();
    for i in 0..n {
        let sig = line_signature(editor, i);
        if editor.layout[i].sig == Some(sig) {
            continue;
        }
        let mut built = build_line(text, editor, i, max_width, scale, default_font_size, &mut buf);
        built.sig = Some(sig);
        editor.layout[i] = built;
        dirty = true;
    }
    if dirty {
        restack(editor, scale);
    }
}

/// Recompute the document-space y of every line plus the total height. Pure
/// arithmetic over cached row heights — no measurement, no allocation.
fn restack(editor: &mut Editor, scale: f32) {
    let mut y = 0.0;
    for i in 0..editor.layout.len() {
        let para = editor.formats.get(i).para;
        y += para.space_before * scale;
        let h: f32 = editor.layout[i].row_h.iter().sum();
        let l = &mut editor.layout[i];
        l.top = y;
        l.height = h;
        y += h + para.space_after * scale;
    }
    editor.total_h = y;
}

/// Lay out one line: shape each format span once, concatenate the advances,
/// then derive wrap rows, row widths and row heights from that table.
fn build_line(
    text: &mut TextRenderer,
    editor: &Editor,
    line_idx: usize,
    max_width: f32,
    scale: f32,
    default_font_size: f32,
    buf: &mut Vec<(u32, f32)>,
) -> LineLayout {
    let line_str = &editor.lines[line_idx];
    let lf = editor.formats.get(line_idx);
    let para = lf.para;

    // ── Advance table: one shaping pass per span ──────────────────────
    let mut adv: Vec<(u32, f32)> = Vec::with_capacity(line_str.len() + 1);
    adv.push((0, 0.0));
    let mut base_x = 0.0f32;
    for span in &lf.iter_spans(line_str.len()) {
        if span.start >= span.end {
            continue;
        }
        let (fs, weight, style) = span_rendering(span, default_font_size);
        text.measure_advances(
            &line_str[span.start..span.end],
            fs,
            weight,
            style,
            span_family(span),
            buf,
        );
        // Entry 0 of each span duplicates the previous span's end.
        for &(off, x) in buf.iter().skip(1) {
            adv.push((span.start as u32 + off, base_x + x));
        }
        base_x = adv.last().map_or(base_x, |&(_, x)| x);
    }

    // ── Wrap rows ─────────────────────────────────────────────────────
    // Bullet paragraphs lose width to the hanging indent on every row; the
    // first-line indent only narrows row 0.
    let line_w = if para.bullet {
        (max_width - BULLET_INDENT * scale).max(10.0)
    } else {
        max_width
    };
    let mut eff_w = (line_w - para.first_indent * scale).max(10.0);
    let mut rows: Vec<usize> = vec![0];
    let mut row_x0 = 0.0f32;
    // Last usable break point as (advance index, byte offset).
    let mut last_break: Option<(usize, usize)> = None;

    for k in 1..adv.len() {
        let byte_pos = adv[k - 1].0 as usize;
        let end_x = adv[k].1;
        let row_start = *rows.last().unwrap();
        if end_x - row_x0 > eff_w && byte_pos > row_start {
            let (brk_idx, brk_byte) = match last_break {
                Some((bi, bb)) if bb > row_start => (bi, bb),
                // No word break available in this row — split mid-word.
                _ => (k - 1, byte_pos),
            };
            rows.push(brk_byte);
            row_x0 = adv[brk_idx].1;
            eff_w = line_w;
            last_break = None;
        }
        // Break opportunities: after a space or a hyphen.
        if matches!(line_str[byte_pos..].chars().next(), Some(' ') | Some('-')) {
            last_break = Some((k, adv[k].0 as usize));
        }
    }

    // ── Per-row width + height ────────────────────────────────────────
    let line_len = line_str.len();
    let mut row_w = Vec::with_capacity(rows.len());
    let mut row_h = Vec::with_capacity(rows.len());
    for (idx, &start) in rows.iter().enumerate() {
        let end = rows.get(idx + 1).copied().unwrap_or(line_len);
        let x0 = adv
            .binary_search_by_key(&(start as u32), |&(o, _)| o)
            .map_or_else(|_| 0.0, |i| adv[i].1);
        let x1 = adv
            .binary_search_by_key(&(end as u32), |&(o, _)| o)
            .map_or_else(|_| adv.last().map_or(0.0, |&(_, x)| x), |i| adv[i].1);
        row_w.push(x1 - x0);
        // A row is as tall as its largest span, so big text never overlaps.
        let fs = lf.max_font_size_in(start, end).map_or(FONT_SIZE, |f| f.max(FONT_SIZE));
        row_h.push(fs * para.line_spacing * scale);
    }

    LineLayout {
        rows,
        row_w,
        row_h,
        adv,
        top: 0.0,
        height: 0.0,
        sig: None,
    }
}

// ── Line signatures (FNV-1a, u64-chunked) ───────────────────────────────────

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv_u64(h: &mut u64, v: u64) {
    *h ^= v;
    *h = h.wrapping_mul(FNV_PRIME);
}

fn fnv_bytes(h: &mut u64, bytes: &[u8]) {
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        fnv_u64(h, u64::from_le_bytes(c.try_into().unwrap()));
    }
    let rem = chunks.remainder();
    let mut last = [0u8; 8];
    last[..rem.len()].copy_from_slice(rem);
    // Mix the remainder length so "ab" and "ab\0" can't collide.
    fnv_u64(h, u64::from_le_bytes(last) ^ ((rem.len() as u64 + 1) << 56));
}

/// Hash everything that affects a line's layout: its text plus the geometric
/// parts of its formatting. Colors, decorations and alignment are excluded —
/// they change painting, not measurement or stacking.
fn line_signature(editor: &Editor, line: usize) -> u64 {
    let mut h = FNV_OFFSET;
    fnv_bytes(&mut h, editor.lines[line].as_bytes());
    let lf = editor.formats.get(line);
    for span in lf.spans() {
        fnv_u64(&mut h, span.start as u64);
        fnv_u64(&mut h, span.end as u64);
        fnv_u64(&mut h, span.attrs.font_size.map_or(0, |f| f.to_bits() as u64 + 1));
        fnv_u64(
            &mut h,
            (span.attrs.bold as u64)
                | (span.attrs.italic as u64) << 1
                | (span.attrs.font.map_or(0, |f| f as u64 + 1)) << 2,
        );
    }
    fnv_u64(&mut h, lf.para.first_indent.to_bits() as u64);
    fnv_u64(&mut h, lf.para.line_spacing.to_bits() as u64);
    fnv_u64(&mut h, lf.para.space_before.to_bits() as u64);
    fnv_u64(&mut h, lf.para.space_after.to_bits() as u64);
    fnv_u64(&mut h, lf.para.bullet as u64);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two rows: "hello " (6 bytes) then "world" — 10px per char, and a
    /// deliberate gap at bytes 2..4 standing in for a two-byte ligature.
    fn sample() -> LineLayout {
        let adv = vec![
            (0, 0.0),
            (1, 10.0),
            (2, 20.0),
            // bytes 2..4 shape as one cluster 20px wide
            (4, 40.0),
            (5, 50.0),
            (6, 60.0),
            (7, 70.0),
            (8, 80.0),
            (9, 90.0),
            (10, 100.0),
            (11, 110.0),
        ];
        LineLayout {
            rows: vec![0, 6],
            row_w: vec![60.0, 50.0],
            row_h: vec![24.0, 24.0],
            adv,
            top: 100.0,
            height: 48.0,
            sig: None,
        }
    }

    #[test]
    fn x_at_hits_exact_boundaries() {
        let l = sample();
        assert_eq!(l.x_at(0), 0.0);
        assert_eq!(l.x_at(6), 60.0);
        assert_eq!(l.x_at(11), 110.0);
    }

    #[test]
    fn x_at_interpolates_inside_a_cluster() {
        // Byte 3 is interior to the 2..4 cluster: half of a 20px span.
        assert_eq!(sample().x_at(3), 30.0);
    }

    #[test]
    fn x_at_past_the_end_clamps_to_the_width() {
        assert_eq!(sample().x_at(999), 110.0);
    }

    #[test]
    fn col_at_x_picks_the_nearest_boundary() {
        let l = sample();
        // Row 1 spans bytes 6..11 starting at x=60.
        assert_eq!(l.col_at_x(0.0, 6, 11), 6);
        assert_eq!(l.col_at_x(14.0, 6, 11), 7); // 74 -> nearer 70 than 80
        assert_eq!(l.col_at_x(16.0, 6, 11), 8); // 76 -> nearer 80 than 70
    }

    #[test]
    fn col_at_x_clamps_to_the_row() {
        let l = sample();
        // Far past the row's right edge still lands on its last offset, not
        // the line's — dragging off the end must not jump rows.
        assert_eq!(l.col_at_x(9000.0, 0, 6), 6);
        assert_eq!(l.col_at_x(-50.0, 6, 11), 6);
    }

    #[test]
    fn rows_resolve_by_column() {
        let l = sample();
        assert_eq!(l.row_at(0), 0);
        assert_eq!(l.row_at(5), 0);
        assert_eq!(l.row_at(6), 1);
        assert_eq!(l.row_at(11), 1);
        assert_eq!(l.row_range(0, 11), (0, 6));
        assert_eq!(l.row_range(1, 11), (6, 11));
    }

    #[test]
    fn row_range_clamps_to_a_shrunken_line() {
        // Layout lagging a delete: rows still describe the longer text.
        let l = sample();
        assert_eq!(l.row_range(1, 4), (4, 4));
    }

    #[test]
    fn width_of_is_a_difference_not_a_measurement() {
        let l = sample();
        assert_eq!(l.width_of(6, 11), 50.0);
        assert_eq!(l.width_of(5, 5), 0.0);
        assert_eq!(l.width_of(9, 2), 0.0);
    }

    #[test]
    fn row_offset_y_stacks_row_heights() {
        let l = sample();
        assert_eq!(l.row_offset_y(0), 0.0);
        assert_eq!(l.row_offset_y(1), 24.0);
    }
}

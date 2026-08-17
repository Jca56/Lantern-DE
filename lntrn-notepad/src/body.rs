//! Editor body painting: selection, find highlights, text runs and the caret.
//!
//! Every position here is read from the per-line cache in `layout.rs` — no
//! text is measured during a frame, and only the rows actually on screen are
//! visited. Split out of `render.rs`, which keeps the frame orchestration and
//! the window chrome.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::FoxPalette;

use crate::editor::{self, Editor};
use crate::find_bar::{match_color, FindBar};
use crate::format::{Alignment, ParagraphAttrs};
use crate::layout::LineLayout;
use crate::render::{alignment_offset, span_family, span_rendering};
use crate::theme::Theme;

/// Where the document sits on screen this frame, plus the visible line range.
pub struct BodyGeom {
    /// The editor viewport.
    pub er: Rect,
    /// Screen x where row text starts before per-row offsets.
    pub content_x: f32,
    /// Usable text width inside the page margins.
    pub content_max_w: f32,
    /// Screen y of the document's first row at the current scroll.
    pub text_y: f32,
    /// Default font size, already scaled.
    pub font_size: f32,
    pub scale: f32,
    /// Visible line range, `[first, last)`.
    pub first: usize,
    pub last: usize,
}

impl BodyGeom {
    /// Screen y of a line's first row.
    fn line_y(&self, l: &LineLayout) -> f32 {
        self.text_y + l.top
    }

    fn row_visible(&self, y: f32, row_h: f32) -> bool {
        y + row_h >= self.er.y && y <= self.er.y + self.er.h
    }
}

/// Per-row x-offset: bullet hanging indent + alignment + first-line indent.
/// Left-aligned text — the common case — costs nothing but two adds. Clicks
/// resolve through this same function so they land on the glyph they hit.
pub(crate) fn row_x_offset(l: &LineLayout, para: &ParagraphAttrs, row_idx: usize, max_w: f32, s: f32) -> f32 {
    let bullet = if para.bullet { editor::BULLET_INDENT * s } else { 0.0 };
    let indent = if row_idx == 0 { para.first_indent * s } else { 0.0 };
    let align = match para.alignment {
        Alignment::Left | Alignment::Justify => 0.0,
        a => {
            let avail = (max_w - bullet).max(10.0);
            alignment_offset(a, avail, l.row_w.get(row_idx).copied().unwrap_or(0.0))
        }
    };
    bullet + align + indent
}

/// Highlight the selected byte range across every visible row it touches.
pub fn draw_selection(painter: &mut Painter, editor: &Editor, g: &BodyGeom, color: Color) {
    let Some((sel_start, sel_end)) = editor.selection_range() else {
        return;
    };
    for i in g.first..g.last {
        if i < sel_start.line || i > sel_end.line {
            continue;
        }
        let Some(l) = editor.line_layout(i) else { continue };
        let line_len = editor.lines[i].len();
        let para = editor.formats.get(i).para;

        // Clip the selection to this line.
        let (sel_begin, sel_finish) = match (i == sel_start.line, i == sel_end.line) {
            (true, true) => (sel_start.col, sel_end.col),
            (true, false) => (sel_start.col, line_len),
            (false, true) => (0, sel_end.col),
            (false, false) => (0, line_len),
        };

        let mut y = g.line_y(l);
        for row_idx in 0..l.row_count() {
            let row_h = l.row_h[row_idx];
            if !g.row_visible(y, row_h) {
                y += row_h;
                continue;
            }
            let (row_start, row_end) = l.row_range(row_idx, line_len);
            let x_off = row_x_offset(l, &para, row_idx, g.content_max_w, g.scale);

            // A selection spanning a line break draws a stub past the last row
            // so the swallowed newline reads as selected.
            let last_row = row_idx + 1 == l.row_count();
            let stub = if i != sel_end.line && last_row && sel_finish >= row_end {
                g.font_size * 0.4
            } else {
                0.0
            };

            let hl_start = sel_begin.max(row_start);
            let hl_end = sel_finish.min(row_end);
            let (x1, w) = if hl_start < hl_end {
                let x1 = g.content_x + x_off + l.width_of(row_start, hl_start);
                (x1, l.width_of(hl_start, hl_end) + stub)
            } else if stub > 0.0 {
                (g.content_x + x_off + l.width_of(row_start, row_end), stub)
            } else {
                y += row_h;
                continue;
            };
            if w > 0.0 {
                painter.rect_filled(Rect::new(x1, y, w, row_h), 0.0, color);
            }
            y += row_h;
        }
    }
}

/// Paint the find bar's match highlights, the current one accented.
pub fn draw_matches(
    painter: &mut Painter,
    editor: &Editor,
    find_bar: &FindBar,
    g: &BodyGeom,
    theme: Theme,
) {
    for (m_idx, m) in find_bar.matches.iter().enumerate() {
        if m.line < g.first || m.line >= g.last {
            continue;
        }
        let Some(l) = editor.line_layout(m.line) else { continue };
        let line_len = editor.lines[m.line].len();
        let para = editor.formats.get(m.line).para;

        let mut y = g.line_y(l);
        for row_idx in 0..l.row_count() {
            let row_h = l.row_h[row_idx];
            let (row_start, row_end) = l.row_range(row_idx, line_len);
            if m.end <= row_start || m.start >= row_end || !g.row_visible(y, row_h) {
                y += row_h;
                continue;
            }
            let x_off = row_x_offset(l, &para, row_idx, g.content_max_w, g.scale);
            let hl_start = m.start.max(row_start);
            let hl_end = m.end.min(row_end);
            let x1 = g.content_x + x_off + l.width_of(row_start, hl_start);
            let w = l.width_of(hl_start, hl_end);
            if w > 0.0 {
                painter.rect_filled(
                    Rect::new(x1, y, w, row_h),
                    2.0 * g.scale,
                    match_color(m_idx == find_bar.current, theme),
                );
            }
            y += row_h;
        }
    }
}

/// Draw the visible text runs, bullets and text decorations.
pub fn draw_text(
    painter: &mut Painter,
    text: &mut TextRenderer,
    editor: &Editor,
    g: &BodyGeom,
    pal: &FoxPalette,
    screen_w: u32,
    screen_h: u32,
) {
    let s = g.scale;
    for i in g.first..g.last {
        let Some(l) = editor.line_layout(i) else { continue };
        let line_str = &editor.lines[i];
        let para = editor.formats.get(i).para;
        let spans = editor.formats.get(i).iter_spans(line_str.len());

        let mut y = g.line_y(l);
        for row_idx in 0..l.row_count() {
            let row_h = l.row_h[row_idx];
            if !g.row_visible(y, row_h) {
                y += row_h;
                continue;
            }
            let (row_start, row_end) = l.row_range(row_idx, line_str.len());

            // Bullet glyph on the first row of a bullet paragraph (even when
            // the line has no text yet). The dot centers on the text's visual
            // midline (~0.58 of the font size below the row top, where glyph
            // ink actually sits given the 1.2 line-height), not the row middle.
            if para.bullet && row_idx == 0 {
                let row_fs = editor.row_font_size(i, row_start, row_end) * s;
                let dot_r = (row_fs * 0.16).max(4.0 * s);
                let dot_x = g.content_x + editor::BULLET_INDENT * 0.5 * s;
                painter.circle_filled(dot_x, y + row_fs * 0.58, dot_r, pal.text);
            }

            if row_start >= row_end {
                y += row_h;
                continue;
            }

            let mut x = g.content_x + row_x_offset(l, &para, row_idx, g.content_max_w, s);
            for span in &spans {
                if span.end <= row_start || span.start >= row_end {
                    continue;
                }
                let clip_start = span.start.max(row_start);
                let clip_end = span.end.min(row_end);
                let span_text = &line_str[clip_start..clip_end];
                if span_text.is_empty() {
                    continue;
                }

                let (fs, weight, style) = span_rendering(span, g.font_size);
                let family = span_family(span);
                let span_color = match span.attrs.color {
                    Some(rgb) => Color::from_rgb8(
                        ((rgb >> 16) & 0xFF) as u8,
                        ((rgb >> 8) & 0xFF) as u8,
                        (rgb & 0xFF) as u8,
                    ),
                    None => pal.text,
                };

                // Slack on the layout bound: quantization can round an
                // exact-width bound down and clip the row's last glyph.
                text.queue_full(
                    span_text, fs, x, y, span_color, g.content_max_w + 4.0 * s, weight, style,
                    family, screen_w, screen_h,
                );

                // Advance from the cached table — the run was measured once,
                // when the line was laid out.
                let span_w = l.width_of(clip_start, clip_end);
                if span.attrs.underline {
                    let ul_y = y + fs + 2.0 * s;
                    painter.line(x, ul_y, x + span_w, ul_y, 1.5 * s, pal.text);
                }
                if span.attrs.strikethrough {
                    let st_y = y + fs * 0.55;
                    painter.line(x, st_y, x + span_w, st_y, 1.5 * s, pal.text);
                }
                x += span_w;
            }
            y += row_h;
        }
    }
}

/// The caret rectangle in screen space, or `None` when it is scrolled away.
pub fn caret_rect(editor: &Editor, g: &BodyGeom) -> Option<Rect> {
    let line = editor.cursor_line.min(editor.lines.len().saturating_sub(1));
    let l = editor.line_layout(line)?;
    let (row_idx, row_start, row_end) = editor.caret_row();
    let row_h = *l.row_h.get(row_idx)?;
    let y = g.line_y(l) + l.row_offset_y(row_idx);
    if y + row_h <= g.er.y || y >= g.er.y + g.er.h {
        return None;
    }
    let para = editor.formats.get(line).para;
    let x = g.content_x
        + row_x_offset(l, &para, row_idx, g.content_max_w, g.scale)
        + l.width_of(row_start, editor.cursor_col.clamp(row_start, row_end));
    // Caret height tracks the row's font size so it's tall on big text.
    let caret_fs = editor.row_font_size(line, row_start, row_end) * g.scale;
    Some(Rect::new(x, y, 2.5 * g.scale, caret_fs + 2.0 * g.scale))
}

//! Drawing a document: a monospace grid inside a two-way scroll area
//! with a fixed gutter, the current line, the selection, search hits, the
//! matching bracket, problem marks, the caret, and every visible line
//! placed once and recolored by its tokens. Keys are handled first so the
//! view can follow the caret in the same frame.

use lntrn_math::{Rect, Vec2};
use lntrn_text::GlyphQuad;
use lntrn_ui::{CursorIcon, Sense, Ui};

use crate::buffer::{Pos, Range};
use crate::doc::Doc;
use crate::editor::lsp_ui::{self, Geom, LspOut, LspUi};
use crate::editor::{cell_metrics, code_style, input, ops};
use crate::git::gutter::{LineMark, MarkKind};
use crate::problems::severity_color;
use crate::settings::Settings;
use crate::syntax::TokenKind;
use crate::term::diag::Severity;
use crate::text_util::{byte_at_cell, cell_of_byte, expand_tabs};

/// A problem to mark: a 0-based line and byte column (and where it
/// ends on that line, when known), and what to say about it when the
/// pointer rests there.
pub struct DiagMark {
    pub line: usize,
    pub col: usize,
    pub end: Option<usize>,
    pub severity: Severity,
    pub message: String,
}

pub struct ViewOpts<'a> {
    pub area_active: bool,
    /// Search hits to mark, and which one is current.
    pub matches: &'a [Range],
    pub current_match: Option<usize>,
    pub diags: &'a [DiagMark],
    /// What git says changed, for bars in the gutter.
    pub git: &'a [LineMark],
    /// The language server's popups.
    pub lsp: &'a mut LspUi,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocOut {
    /// The text changed.
    pub changed: bool,
    pub focused: bool,
    pub clicked: bool,
    /// What to ask the language server.
    pub lsp: LspOut,
}

/// Blink period and the part of it the caret shows.
const BLINK_PERIOD: f64 = 1.1;
const BLINK_ON: f64 = 0.6;

/// The position under a pointer.
fn hit(doc: &Doc, p: Vec2, origin: Vec2, gutter_w: f64, cell_w: f64, lh: f64) -> Pos {
    let n = doc.buffer.line_count();
    let line = (((p.y - origin.y) / lh).floor().max(0.0) as usize).min(n - 1);
    let cell = ((p.x - origin.x - gutter_w) / cell_w).round().max(0.0) as usize;
    Pos::new(line, byte_at_cell(doc.line(line), doc.tab(), cell))
}

pub fn draw_doc(ui: &mut Ui, doc: &mut Doc, settings: &Settings, opts: ViewOpts) -> DocOut {
    let id = ui.id("code");
    let m = ui.m;
    let style = code_style(ui, settings);
    let (cell_w, lh) = cell_metrics(ui, &style);
    let tab = settings.tab();
    doc.set_tab(tab);
    let bar = m.scrollbar_w;
    let inner_w = (ui.avail_width() - bar - m.gap).max(1.0);
    let inner_h = (ui.remaining_height() - bar - m.gap).max(lh);
    let digits = doc.buffer.line_count().to_string().len().max(2);
    let gutter_w = ((digits + 2) as f64 * cell_w).round();
    let text_w = (inner_w - gutter_w).max(cell_w);
    let focused = ui.state.focus == Some(id);
    let page = ((inner_h / lh).floor() as usize).saturating_sub(1).max(1);
    let mut out = DocOut { focused, ..DocOut::default() };
    let mut lsp_out = LspOut::default();
    if focused {
        let (picked, asked) = opts.lsp.keys(ui, doc, ui.state.now);
        out.changed |= picked;
        lsp_out = asked;
        let typed = input::handle(ui, doc, settings, page);
        out.changed |= typed;
        if let Some(t) = opts.lsp.after_edit(doc, typed) {
            lsp_out.complete = Some(t);
        }
    }
    // Follow the caret when it moved (or the text changed under it).
    let now = ui.state.now;
    let slot = *ui.state.floats(id, [-1.0; 4]);
    let moved = slot[0] < 0.0 || slot[0] as usize != doc.cursor.line || slot[1] as usize != doc.cursor.col;
    if moved || out.changed {
        let cx = cell_of_byte(doc.line(doc.cursor.line), tab, doc.cursor.col) as f64 * cell_w;
        let cy = doc.cursor.line as f64 * lh;
        let off = &mut ui.state.scroll(id).offset;
        if cy < off.y {
            off.y = cy;
        } else if cy + lh > off.y + inner_h {
            off.y = cy + lh - inner_h;
        }
        let margin = cell_w * 4.0;
        if cx < off.x + margin {
            off.x = (cx - margin).max(0.0);
        } else if cx + cell_w > off.x + text_w - margin {
            off.x = cx + cell_w - text_w + margin;
        }
    }
    let phase = if moved { now } else { slot[2] };
    *ui.state.floats(id, [-1.0; 4]) = [doc.cursor.line as f64, doc.cursor.col as f64, phase, 0.0];

    let content_w = gutter_w + (doc.max_cells() as f64 + 4.0) * cell_w;
    let n = doc.buffer.line_count();
    let colors = &settings.colors;
    let highlight_line = settings.highlight_line;
    ui.scroll_area_2d("code", None, content_w, |ui, view| {
        let vp = view.viewport;
        let theme = ui.theme;
        ui.draw.rect(vp, theme.field);
        let r = ui.interact(id, vp, Sense::FOCUS);
        let focused = ui.focusable(id, vp);
        out.focused = focused;
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Text;
        }
        let origin = view.origin;
        let text_x0 = origin.x + gutter_w;
        // ---- pointer ----
        if r.pressed {
            let p = hit(doc, ui.state.pointer, origin, gutter_w, cell_w, lh);
            if r.double_clicked {
                doc.select(input::word_range(doc, p));
            } else {
                doc.set_cursor(p, ui.state.mods.shift());
            }
            out.clicked = true;
            ui.state.request_rebuild = true;
        } else if r.dragging {
            let p = hit(doc, vp.clamp_point(ui.state.pointer), origin, gutter_w, cell_w, lh);
            if p != doc.cursor {
                doc.set_cursor(p, true);
                ui.state.request_rebuild = true;
            }
        }
        // Ctrl+click goes to the definition; a resting pointer asks what is under it.
        if r.clicked && ui.state.mods.ctrl() {
            lsp_out.definition = Some(hit(doc, ui.state.pointer, origin, gutter_w, cell_w, lh));
        }
        if r.hovered && !r.dragging && ui.state.pointer.x >= vp.min.x + gutter_w {
            let p = ui.state.pointer;
            let line = (((p.y - origin.y) / lh).floor().max(0.0) as usize).min(n.saturating_sub(1));
            let cell = ((p.x - text_x0) / cell_w).floor().max(0.0) as usize;
            let under = Pos::new(line, byte_at_cell(doc.line(line), tab, cell));
            if let Some(q) = opts.lsp.pointer(ui, doc.id, under, now) {
                lsp_out.hover = Some(q);
            }
        }
        // ---- what shows ----
        let first = ((view.offset.y / lh).floor().max(0.0) as usize).min(n - 1);
        let last = (((view.offset.y + vp.height()) / lh).ceil() as usize + 1).min(n);
        doc.highlight.ensure(&doc.buffer, last.saturating_sub(1));
        let sel = doc.selection();
        let cur = doc.cursor;
        let row_y = |line: usize| origin.y + line as f64 * lh;
        let cell_x = |line: usize, col: usize| text_x0 + cell_of_byte(doc.line(line), tab, col) as f64 * cell_w;
        let text_clip = Rect::new(Vec2::new(vp.min.x + gutter_w, vp.min.y), vp.max);
        ui.draw.push_clip(text_clip);
        if highlight_line && sel.is_empty() && (first..last).contains(&cur.line) {
            ui.draw.rect(Rect::new(Vec2::new(text_clip.min.x, row_y(cur.line)), Vec2::new(vp.max.x, row_y(cur.line) + lh)), theme.panel);
        }
        if !sel.is_empty() {
            let color = theme.selection.fade(0.45);
            for line in sel.start.line.max(first)..(sel.end.line + 1).min(last) {
                let x0 = if line == sel.start.line { cell_x(line, sel.start.col) } else { text_x0 };
                let x1 = if line == sel.end.line { cell_x(line, sel.end.col) } else { text_x0 + (doc.line_cells(line) as f64 + 0.5) * cell_w };
                ui.draw.rect(Rect::new(Vec2::new(x0, row_y(line)), Vec2::new(x1.max(x0 + m.px(2.0)), row_y(line) + lh)), color);
            }
        }
        for (i, mr) in opts.matches.iter().enumerate() {
            if mr.start.line < first || mr.start.line >= last {
                continue;
            }
            let rect = Rect::new(Vec2::new(cell_x(mr.start.line, mr.start.col), row_y(mr.start.line)), Vec2::new(cell_x(mr.end.line, mr.end.col), row_y(mr.start.line) + lh));
            let current = opts.current_match == Some(i);
            ui.draw.rounded_rect(rect, m.radius * 0.5, theme.accent.fade(if current { 0.5 } else { 0.22 }));
            if current {
                ui.draw.stroke_rect(rect, m.border, m.radius * 0.5, theme.accent);
            }
        }
        if let Some((a, b)) = ops::matching_bracket(doc, cur) {
            for p in [a, b] {
                if (first..last).contains(&p.line) {
                    let rect = Rect::from_min_size(Vec2::new(cell_x(p.line, p.col), row_y(p.line)), Vec2::new(cell_w, lh));
                    ui.draw.stroke_rect(rect, m.border, m.radius * 0.4, theme.focus.fade(0.7));
                }
            }
        }
        // ---- problem marks: an underline from the column to the line's end ----
        let mut hover_mark: Option<usize> = None;
        let pointer = ui.state.pointer;
        for (i, d) in opts.diags.iter().enumerate() {
            if d.line < first || d.line >= last || d.line >= n {
                continue;
            }
            let text = doc.line(d.line);
            let x0 = cell_x(d.line, d.col.min(text.len()));
            let x1 = match d.end {
                Some(e) if e > d.col => cell_x(d.line, e.min(text.len())).max(x0 + cell_w * 0.5),
                _ => (text_x0 + doc.line_cells(d.line) as f64 * cell_w).max(x0 + cell_w),
            };
            let y1 = row_y(d.line) + lh;
            let color = severity_color(ui, d.severity);
            ui.draw.hline(x0, x1, y1 - m.px(2.0), m.px(2.0), color);
            let band = Rect::new(Vec2::new(vp.min.x, row_y(d.line)), Vec2::new(vp.max.x, y1));
            let on_text = pointer.x >= x0 && pointer.x <= x1;
            let on_gutter = pointer.x < vp.min.x + gutter_w;
            if hover_mark.is_none() && r.hovered && band.contains(pointer) && (on_text || on_gutter) {
                hover_mark = Some(i);
            }
        }
        // ---- the lines ----
        let mut expanded = String::new();
        let mut cells: Vec<u32> = Vec::new();
        let mut quads: Vec<GlyphQuad> = Vec::new();
        let mut spans: Vec<(u32, u32, TokenKind)> = Vec::new();
        let plain = theme.text.to_gpu();
        for line in first..last {
            let text = doc.line(line);
            if text.is_empty() {
                continue;
            }
            expand_tabs(text, tab, &mut expanded, &mut cells);
            quads.clear();
            ui.text.place(&expanded, &style, text_x0 as f32, row_y(line) as f32, 1.0e6, plain, &mut quads);
            let tokens = doc.highlight.tokens(line);
            if !tokens.is_empty() {
                spans.clear();
                spans.extend(tokens.iter().map(|t| (cells[t.start as usize], cells[(t.end as usize).min(cells.len() - 1)], t.kind)));
                let mut ti = 0;
                let mut prev_cell = 0u32;
                for q in &mut quads {
                    let c = ((q.x + q.w * 0.5 - text_x0 as f32) / cell_w as f32).floor().max(0.0) as u32;
                    if c < prev_cell {
                        ti = 0;
                    }
                    prev_cell = c;
                    while ti < spans.len() && spans[ti].1 <= c {
                        ti += 1;
                    }
                    if ti < spans.len() && spans[ti].0 <= c {
                        q.color = colors.of(spans[ti].2, theme.text).to_gpu();
                    }
                }
            }
            ui.draw.glyphs(&quads);
        }
        // ---- caret ----
        if focused {
            let t = (now - phase) % BLINK_PERIOD;
            let on = ui.state.reduce_motion || !opts.area_active || t < BLINK_ON;
            let caret = Rect::from_min_size(Vec2::new(cell_x(cur.line, cur.col).round(), row_y(cur.line)), Vec2::new(m.px(2.0), lh));
            if on {
                ui.draw.rect(caret, theme.accent);
            }
            ui.state.ime_rect = Some(caret);
            if opts.area_active && !ui.state.reduce_motion {
                ui.state.request_redraw_after(if t < BLINK_ON { BLINK_ON - t } else { BLINK_PERIOD - t } + 0.01);
            }
        }
        ui.draw.pop_clip();
        // ---- the gutter, fixed while the text scrolls sideways ----
        let gutter = Rect::new(vp.min, Vec2::new(vp.min.x + gutter_w, vp.max.y));
        ui.draw.rect(gutter, theme.field);
        ui.draw.vline(gutter.max.x - m.border, vp.min.y, vp.max.y, m.border, theme.border_light.fade(0.35));
        let right = gutter.max.x - cell_w;
        for line in first..last {
            let num = (line + 1).to_string();
            let w = ui.measure(&num, &style);
            let color = if line == cur.line { theme.text } else { theme.text_dim.fade(0.7) };
            quads.clear();
            ui.text.place(&num, &style, (right - w) as f32, row_y(line) as f32, 1.0e6, color.to_gpu(), &mut quads);
            ui.draw.glyphs(&quads);
        }
        // ---- git: a bar beside the lines that changed, a tick where lines went ----
        let bar_x = gutter.max.x - m.px(6.0);
        for mk in opts.git {
            let color = match mk.kind {
                MarkKind::Added => settings.git.added,
                MarkKind::Modified => settings.git.modified,
                MarkKind::Deleted => settings.git.deleted,
            };
            match mk.kind {
                MarkKind::Deleted => {
                    if mk.line >= first && mk.line <= last {
                        let y = row_y(mk.line);
                        ui.draw.rect(Rect::new(Vec2::new(bar_x - m.px(4.0), y - m.px(2.0)), Vec2::new(bar_x + m.px(3.0), y + m.px(2.0))), color);
                    }
                }
                _ => {
                    let (a, b) = (mk.line.max(first), (mk.line + mk.len).min(last));
                    if a < b {
                        ui.draw.rect(Rect::new(Vec2::new(bar_x, row_y(a)), Vec2::new(bar_x + m.px(3.0), row_y(b))), color);
                    }
                }
            }
        }
        for d in opts.diags {
            if d.line >= first || d.line < last {
                let color = severity_color(ui, d.severity);
                let rad = (lh * 0.16).round().max(m.px(3.0));
                ui.draw.circle(Vec2::new(gutter.min.x + cell_w * 0.6, row_y(d.line) + lh * 0.5), rad, color);
            }
        }
        // What the problem under the pointer says, on a panel under its line.
        if let Some(i) = hover_mark {
            let d = &opts.diags[i];
            let more = opts.diags.iter().filter(|o| o.line == d.line).count() - 1;
            let text = if more > 0 { format!("{}: {}  (+{more} more)", d.severity.label(), d.message) } else { format!("{}: {}", d.severity.label(), d.message) };
            let ts = ui.text_style();
            let w = (ui.measure(&text, &ts) + m.pad * 2.0).min(vp.width() - m.gap * 2.0);
            let below = row_y(d.line) + lh + m.gap;
            let y = if below + m.widget_h > vp.max.y { row_y(d.line) - m.gap - m.widget_h } else { below };
            let x = (vp.min.x + gutter_w).min(vp.max.x - w - m.gap).max(vp.min.x + m.gap);
            let panel = Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, m.widget_h));
            ui.floating_panel(panel, theme.header);
            ui.draw.rect(Rect::new(panel.min, Vec2::new(panel.min.x + m.px(4.0), panel.max.y)), severity_color(ui, d.severity));
            ui.text_in_rect(&text, &ts, Rect::new(Vec2::new(panel.min.x + m.pad, panel.min.y), panel.max), theme.text);
        }
        // ---- the language server's popups, over everything ----
        if let Some(item) = opts.lsp.draw(ui, doc, Geom { vp, text_x0, origin_y: origin.y, cell_w, lh }, &style) {
            lsp_ui::apply(doc, &item, opts.lsp.utf16, now);
            out.changed = true;
            ui.state.request_rebuild = true;
        }
        // One line of slack below the last, so the end can sit above the bottom.
        ui.space(n as f64 * lh + lh + m.gap);
    });
    out.lsp = lsp_out;
    out
}

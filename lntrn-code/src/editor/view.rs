//! Drawing a document: a monospace grid inside a two-way scroll area
//! with a fixed gutter, folded blocks collapsed to their first line, the
//! current line, the decorations ([`decor`]), the caret, and every
//! visible line placed once and recolored by its tokens. Keys are handled
//! first so the view can follow the caret in the same frame.

use lntrn_math::{Rect, Vec2};
use lntrn_text::GlyphQuad;
use lntrn_ui::{CursorIcon, Sense, Ui};

use crate::buffer::{Pos, Range};
use crate::doc::Doc;
use crate::editor::decor::{self, DiagMark, Grid};
use crate::editor::fold::{self, Layout};
use crate::editor::lsp_ui::{self, Geom, LspOut, LspUi};
use crate::editor::{cell_metrics, code_style, input, ops};
use crate::git::gutter::LineMark;
use crate::settings::Settings;
use crate::syntax::TokenKind;
use crate::text_util::{bracket_pair, byte_at_cell, cell_of_byte, expand_tabs};

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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DocOut {
    /// The text changed.
    pub changed: bool,
    pub focused: bool,
    pub clicked: bool,
    /// What to ask the language server.
    pub lsp: LspOut,
    /// A right click on the text: where the menu goes.
    pub context: Option<Vec2>,
    /// Ctrl+wheel: steps of code font size.
    pub zoom: i32,
}

/// Blink period and the part of it the caret shows.
const BLINK_PERIOD: f64 = 1.1;
const BLINK_ON: f64 = 0.6;
/// A third click this soon after a double click selects the line.
const TRIPLE_CLICK: f64 = 0.5;

/// The position under a pointer.
fn hit(doc: &Doc, layout: &Layout, p: Vec2, origin: Vec2, gutter_w: f64, cell_w: f64, lh: f64) -> Pos {
    let row = ((p.y - origin.y) / lh).floor().max(0.0) as usize;
    let line = layout.line_at(row);
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
    let mut out = DocOut { focused: ui.state.focus == Some(id), ..DocOut::default() };
    // Ctrl+wheel over the editor zooms it; the scroll area must not see it.
    let area = Rect::from_min_size(ui.cursor(), Vec2::new(ui.avail_width(), ui.remaining_height()));
    if ui.state.mods.ctrl() && ui.state.wheel.y != 0.0 && ui.state.pointer_in_window && area.contains(ui.state.pointer) {
        out.zoom = if ui.state.wheel.y > 0.0 { 1 } else { -1 };
        ui.state.wheel = Vec2::ZERO;
    }
    let bar = m.scrollbar_w;
    let inner_w = (ui.avail_width() - bar - m.gap).max(1.0);
    let inner_h = (ui.remaining_height() - bar - m.gap).max(lh);
    let page = ((inner_h / lh).floor() as usize).saturating_sub(1).max(1);
    let mut lsp_out = LspOut::default();
    if out.focused {
        let (picked, asked) = opts.lsp.keys(ui, doc, ui.state.now);
        out.changed |= picked;
        lsp_out = asked;
        let typed = input::handle(ui, doc, settings, page);
        out.changed |= typed;
        let more = opts.lsp.after_edit(doc, typed);
        if more.complete.is_some() {
            lsp_out.complete = more.complete;
        }
        lsp_out.signature = more.signature;
    }
    // ---- folds: the blocks there are, the caret never inside one ----
    let n = doc.buffer.line_count();
    doc.highlight.ensure(&doc.buffer, n - 1);
    let scan = fold::scan(doc);
    doc.fold_ranges = scan.regions.iter().map(|r| (r.start, r.end)).collect();
    doc.folded.retain(|s| scan.regions.iter().any(|r| r.start == *s));
    if doc.is_hidden(doc.cursor.line) {
        doc.unfold_at(doc.cursor.line);
    }
    let layout = Layout::build(n, &scan.regions, &doc.folded);
    let foldable = !scan.regions.is_empty();
    let digits = n.to_string().len().max(2);
    let gutter_w = ((digits + if foldable { 3 } else { 2 }) as f64 * cell_w).round();
    let text_w = (inner_w - gutter_w).max(cell_w);
    // Follow the caret when it moved (or the text changed under it).
    let now = ui.state.now;
    let slot = *ui.state.floats(id, [-1.0; 4]);
    let moved = slot[0] < 0.0 || slot[0] as usize != doc.cursor.line || slot[1] as usize != doc.cursor.col;
    if moved || out.changed {
        let cx = cell_of_byte(doc.line(doc.cursor.line), tab, doc.cursor.col) as f64 * cell_w;
        let cy = layout.row_of(doc.cursor.line) as f64 * lh;
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
    let clicks = id.with("clicks");
    let last_double = ui.state.floats(clicks, [-10.0; 4])[0];

    let content_w = gutter_w + (doc.max_cells() as f64 + 4.0) * cell_w;
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
        let gutter = Rect::new(vp.min, Vec2::new(vp.min.x + gutter_w, vp.max.y));
        let rows = layout.rows();
        let first_row = ((view.offset.y / lh).floor().max(0.0) as usize).min(rows - 1);
        let last_row = (((view.offset.y + vp.height()) / lh).ceil() as usize + 1).min(rows);
        let g = Grid { layout: &layout, vp, origin, gutter_w, cell_w, lh, tab, first_row, last_row };
        let text_x0 = g.text_x0();
        // ---- pointer ----
        let on_gutter = gutter.contains(ui.state.pointer);
        let fold_hit = |p: Vec2| -> Option<usize> {
            if !gutter.contains(p) || p.x < gutter.max.x - cell_w * 1.4 {
                return None;
            }
            let row = ((p.y - origin.y) / lh).floor().max(0.0) as usize;
            let line = layout.line_at(row);
            doc.region_at(line).map(|_| line)
        };
        if r.pressed {
            if let Some(line) = fold_hit(ui.state.press_pos) {
                doc.toggle_fold(line);
                ui.state.request_rebuild = true;
            } else {
                let p = hit(doc, &layout, ui.state.pointer, origin, gutter_w, cell_w, lh);
                if r.double_clicked {
                    doc.select(input::word_range(doc, p));
                    ui.state.floats(clicks, [-10.0; 4])[0] = now;
                } else if now - last_double < TRIPLE_CLICK && p.line == doc.cursor.line {
                    ops::select_line(doc);
                    ui.state.floats(clicks, [-10.0; 4])[0] = -10.0;
                } else {
                    doc.set_cursor(p, ui.state.mods.shift());
                }
                out.clicked = true;
            }
            ui.state.request_rebuild = true;
        } else if r.dragging && now - last_double >= TRIPLE_CLICK {
            let p = hit(doc, &layout, vp.clamp_point(ui.state.pointer), origin, gutter_w, cell_w, lh);
            if p != doc.cursor {
                doc.set_cursor(p, true);
                ui.state.request_rebuild = true;
            }
        }
        // A right click opens the editor's menu, at the caret it moves there
        // unless it lands inside the selection.
        if ui.state.right_pressed && vp.contains(ui.state.pointer) {
            let p = hit(doc, &layout, ui.state.pointer, origin, gutter_w, cell_w, lh);
            let sel = doc.selection();
            let inside = !sel.is_empty() && (p.line, p.col) >= (sel.start.line, sel.start.col) && (p.line, p.col) <= (sel.end.line, sel.end.col);
            if !inside {
                doc.set_cursor(p, false);
            }
            out.context = Some(ui.state.pointer);
            ui.state.request_rebuild = true;
        }
        // Ctrl+click goes to the definition; a resting pointer asks what is under it.
        if r.clicked && ui.state.mods.ctrl() {
            lsp_out.definition = Some(hit(doc, &layout, ui.state.pointer, origin, gutter_w, cell_w, lh));
        }
        if r.hovered && !r.dragging && !on_gutter {
            let p = ui.state.pointer;
            let row = ((p.y - origin.y) / lh).floor().max(0.0) as usize;
            let line = layout.line_at(row);
            let cell = ((p.x - text_x0) / cell_w).floor().max(0.0) as usize;
            let under = Pos::new(line, byte_at_cell(doc.line(line), tab, cell));
            if let Some(q) = opts.lsp.pointer(ui, doc.id, under, now) {
                lsp_out.hover = Some(q);
            }
        }
        // ---- what shows ----
        let sel = doc.selection();
        let cur = doc.cursor;
        let text_clip = Rect::new(Vec2::new(vp.min.x + gutter_w, vp.min.y), vp.max);
        ui.draw.push_clip(text_clip);
        if highlight_line && sel.is_empty() && g.shown(cur.line) {
            ui.draw.rect(Rect::new(Vec2::new(text_clip.min.x, g.row_y(cur.line)), Vec2::new(vp.max.x, g.row_y(cur.line) + lh)), theme.panel.mid());
        }
        decor::indent_guides(ui, doc, &g, colors);
        decor::selection(ui, doc, &g);
        decor::occurrences(ui, doc, &g);
        decor::matches(ui, doc, &g, opts.matches, opts.current_match);
        if let Some((a, b)) = ops::matching_bracket(doc, cur) {
            for p in [a, b] {
                if g.shown(p.line) {
                    let rect = Rect::from_min_size(Vec2::new(g.cell_x(doc, p.line, p.col), g.row_y(p.line)), Vec2::new(cell_w, lh));
                    ui.draw.stroke_rect(rect, m.border, m.radius * 0.4, theme.focus.fade(0.7));
                }
            }
        }
        let hover_mark = decor::diag_marks(ui, doc, &g, opts.diags, r.hovered);
        // ---- the lines: placed once, recolored by token, brackets by depth ----
        let mut expanded = String::new();
        let mut cells: Vec<u32> = Vec::new();
        let mut quads: Vec<GlyphQuad> = Vec::new();
        let mut spans: Vec<(u32, u32, TokenKind)> = Vec::new();
        let mut brackets: Vec<(u32, u16)> = Vec::new();
        let plain = colors.text.to_gpu();
        let ts = ui.text_style();
        for row in first_row..last_row {
            let line = layout.line_at(row);
            let y = g.row_top(row);
            if let Some(end) = layout.folded_end(line) {
                // A folded header: a chip after its text says how much is hidden.
                let label = format!("⋯ {} lines", end - line);
                let x = text_x0 + (doc.line_cells(line) as f64 + 1.0) * cell_w;
                let w = ui.measure(&label, &ts) + m.pad * 2.0;
                let chip = Rect::new(Vec2::new(x, y + m.px(2.0)), Vec2::new(x + w, y + lh - m.px(2.0)));
                ui.raised(chip, theme.widget, false);
                ui.text_centered(&label, &ts, chip, theme.text_dim);
                if ui.state.pressed && chip.contains(ui.state.press_pos) {
                    doc.toggle_fold(line);
                    ui.state.request_rebuild = true;
                }
            }
            let text = doc.line(line);
            if text.is_empty() {
                continue;
            }
            expand_tabs(text, tab, &mut expanded, &mut cells);
            quads.clear();
            ui.text.place(&expanded, &style, text_x0 as f32, y as f32, 1.0e6, plain, &mut quads);
            let tokens = doc.highlight.tokens(line);
            spans.clear();
            brackets.clear();
            let mut depth = scan.depth.get(line).copied().unwrap_or(0);
            for t in tokens {
                let (a, b) = (t.start as usize, (t.end as usize).min(text.len()));
                spans.push((cells[a], cells[b.min(cells.len() - 1)], t.kind));
                if matches!(t.kind, TokenKind::Punct | TokenKind::Operator) {
                    for (i, c) in text[a..b].char_indices() {
                        match bracket_pair(c) {
                            Some((_, _, true)) => {
                                brackets.push((cells[a + i], depth));
                                depth = depth.saturating_add(1);
                            }
                            Some((_, _, false)) => {
                                depth = depth.saturating_sub(1);
                                brackets.push((cells[a + i], depth));
                            }
                            None => {}
                        }
                    }
                }
            }
            if !spans.is_empty() {
                let mut ti = 0;
                let mut bi = 0;
                let mut prev_cell = 0u32;
                for q in &mut quads {
                    let c = ((q.x + q.w * 0.5 - text_x0 as f32) / cell_w as f32).floor().max(0.0) as u32;
                    if c < prev_cell {
                        ti = 0;
                        bi = 0;
                    }
                    prev_cell = c;
                    while ti < spans.len() && spans[ti].1 <= c {
                        ti += 1;
                    }
                    if ti < spans.len() && spans[ti].0 <= c {
                        q.color = colors.of(spans[ti].2).to_gpu();
                    }
                    while bi < brackets.len() && brackets[bi].0 < c {
                        bi += 1;
                    }
                    if bi < brackets.len() && brackets[bi].0 == c {
                        q.color = decor::bracket_color(brackets[bi].1, colors).to_gpu();
                    }
                }
            }
            ui.draw.glyphs(&quads);
        }
        // ---- caret ----
        if focused {
            let t = (now - phase) % BLINK_PERIOD;
            let on = ui.state.reduce_motion || !opts.area_active || t < BLINK_ON;
            let caret = Rect::from_min_size(Vec2::new(g.cell_x(doc, cur.line, cur.col).round(), g.row_y(cur.line)), Vec2::new(m.px(2.0), lh));
            if on {
                ui.draw.rect(caret, theme.accent);
            }
            ui.state.ime_rect = Some(caret);
            opts.lsp.caret_screen = Some(Vec2::new(caret.min.x, caret.max.y));
            if opts.area_active && !ui.state.reduce_motion {
                ui.state.request_redraw_after(if t < BLINK_ON { BLINK_ON - t } else { BLINK_PERIOD - t } + 0.01);
            }
        }
        ui.draw.pop_clip();
        // ---- the gutter, fixed while the text scrolls sideways ----
        ui.draw.rect(gutter, theme.field);
        ui.draw.vline(gutter.max.x - m.border, vp.min.y, vp.max.y, m.border, theme.border_light.fade(0.35));
        let right = gutter.max.x - cell_w * if foldable { 2.2 } else { 1.0 };
        for row in first_row..last_row {
            let line = layout.line_at(row);
            let num = (line + 1).to_string();
            let w = ui.measure(&num, &style);
            let color = if line == cur.line { theme.text } else { theme.text_dim.fade(0.7) };
            quads.clear();
            ui.text.place(&num, &style, (right - w) as f32, g.row_top(row) as f32, 1.0e6, color.to_gpu(), &mut quads);
            ui.draw.glyphs(&quads);
            // Fold markers: folded blocks always, foldable ones while the pointer is on the gutter.
            if let Some(region) = doc.region_at(line).map(|(_, e)| e) {
                let folded = layout.folded_end(line).is_some();
                if folded || (on_gutter && region > line) {
                    let c = Vec2::new(gutter.max.x - cell_w * 0.85, g.row_top(row) + lh * 0.5);
                    let s = m.px(5.0);
                    let wl = m.px(2.0);
                    let ink = if folded { theme.accent } else { theme.text_dim };
                    if folded {
                        ui.draw.line(Vec2::new(c.x - s * 0.5, c.y - s), Vec2::new(c.x + s * 0.5, c.y), wl, ink);
                        ui.draw.line(Vec2::new(c.x + s * 0.5, c.y), Vec2::new(c.x - s * 0.5, c.y + s), wl, ink);
                    } else {
                        ui.draw.line(Vec2::new(c.x - s, c.y - s * 0.5), Vec2::new(c.x, c.y + s * 0.5), wl, ink);
                        ui.draw.line(Vec2::new(c.x, c.y + s * 0.5), Vec2::new(c.x + s, c.y - s * 0.5), wl, ink);
                    }
                }
            }
        }
        if on_gutter && r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        decor::git_marks(ui, &g, gutter, opts.git, &settings.git);
        decor::diag_dots(ui, &g, gutter, opts.diags);
        if let Some(i) = hover_mark {
            decor::diag_panel(ui, &g, opts.diags, i);
        }
        // ---- the language server's popups, over everything ----
        let popup_origin_y = origin.y + (layout.row_of(cur.line) as f64 - cur.line as f64) * lh;
        if let Some(item) = opts.lsp.draw(ui, doc, Geom { vp, text_x0, origin_y: popup_origin_y, cell_w, lh }, &style) {
            lsp_ui::apply(doc, &item, opts.lsp.utf16, now);
            out.changed = true;
            ui.state.request_rebuild = true;
        }
        // One line of slack below the last, so the end can sit above the bottom.
        ui.space(rows as f64 * lh + lh + m.gap);
    });
    out.lsp = lsp_out;
    out
}

//! Language-server help in the code view: the hover panel where the
//! pointer rests, the completion list with its keys and its pick, the
//! signature of the call being typed, and the requests the view asks the
//! app to send.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Key, Sense, Ui};

use crate::buffer::Pos;
use crate::doc::{Doc, DocId, EditKind};
use crate::lsp::edits::range_of;
use crate::lsp::{CompletionItem, SignatureHelp, TextEdit};
use crate::text_util::cell_of_byte;

/// The pointer rests this long before a hover is asked for.
const HOVER_REST: f64 = 0.45;
/// Rows of the completion list on screen at once.
const LIST_ROWS: usize = 10;

pub struct Hover {
    pub doc: DocId,
    pub pos: Pos,
    pub lines: Vec<String>,
    /// Where the pointer was; the panel opens under that row.
    pub anchor: Vec2,
}

pub struct Completion {
    pub doc: DocId,
    /// Where the word being completed starts.
    pub anchor: Pos,
    pub items: Vec<CompletionItem>,
    pub selected: usize,
}

/// The signature panel: for which document, above which line.
pub struct SignaturePopup {
    pub doc: DocId,
    pub line: usize,
    pub help: SignatureHelp,
}

#[derive(Default)]
pub struct LspUi {
    pub hover: Option<Hover>,
    pub completion: Option<Completion>,
    pub signature: Option<SignaturePopup>,
    /// Where the caret was when the signature was asked for; moving it
    /// without typing closes the panel.
    pub sig_cursor: Option<Pos>,
    /// Where the pointer has been resting, and since when.
    rest: Option<(Vec2, f64)>,
    /// The hover asked for and not answered yet: document, position,
    /// pointer.
    pub asked: Option<(DocId, Pos, Vec2)>,
    /// The server counts columns in UTF-16 units.
    pub utf16: bool,
    /// Under the caret on screen, where a menu for it opens.
    pub caret_screen: Option<Vec2>,
}

/// Where the text sits on screen, for placing the popups.
#[derive(Clone, Copy, Debug)]
pub struct Geom {
    pub vp: Rect,
    pub text_x0: f64,
    pub origin_y: f64,
    pub cell_w: f64,
    pub lh: f64,
}

/// What the view wants sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LspOut {
    pub hover: Option<Pos>,
    pub definition: Option<Pos>,
    pub complete: Option<(Pos, Option<char>)>,
    /// Signature help at a position: the trigger character typed, and
    /// whether a panel is already up.
    pub signature: Option<(Pos, Option<char>, bool)>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where the word the caret is in (or after) starts.
pub fn word_start(line: &str, col: usize) -> usize {
    let mut a = col.min(line.len());
    while a > 0 {
        let prev = line[..a].chars().next_back().unwrap_or(' ');
        if !is_word(prev) {
            break;
        }
        a -= prev.len_utf8();
    }
    a
}

impl LspUi {
    pub fn close_all(&mut self) {
        self.hover = None;
        self.completion = None;
        self.signature = None;
        self.asked = None;
    }

    /// The items matching what was typed since the anchor, as indexes.
    pub fn filtered(&self, doc: &Doc) -> Vec<usize> {
        let Some(c) = &self.completion else {
            return Vec::new();
        };
        let line = doc.line(c.anchor.line);
        let prefix = if doc.cursor.line == c.anchor.line && doc.cursor.col >= c.anchor.col { line[c.anchor.col..doc.cursor.col].to_lowercase() } else { String::new() };
        if prefix.is_empty() {
            return (0..c.items.len()).collect();
        }
        let mut starts: Vec<usize> = Vec::new();
        let mut contains: Vec<usize> = Vec::new();
        for (i, it) in c.items.iter().enumerate() {
            let f = it.filter.to_lowercase();
            if f.starts_with(&prefix) {
                starts.push(i);
            } else if f.contains(&prefix) {
                contains.push(i);
            }
        }
        starts.extend(contains);
        starts
    }

    /// Keys the popups take before the editor sees them. Returns whether
    /// the text changed, and what to ask for.
    pub fn keys(&mut self, ui: &mut Ui, doc: &mut Doc, now: f64) -> (bool, LspOut) {
        let mut out = LspOut::default();
        let mut changed = false;
        if ui.state.take_key(|k| k.key == Key::Space && k.mods.ctrl()).is_some() {
            out.complete = Some((doc.cursor, None));
        }
        if ui.state.take_key(|k| k.key == Key::F(12) && k.mods.is_empty()).is_some() {
            out.definition = Some(doc.cursor);
        }
        if (self.hover.is_some() || self.signature.is_some()) && ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() {
            self.hover = None;
            self.signature = None;
        }
        let open = self.completion.as_ref().is_some_and(|c| c.doc == doc.id);
        if !open {
            return (changed, out);
        }
        let shown = self.filtered(doc);
        if shown.is_empty() {
            return (changed, out);
        }
        let take = |ui: &mut Ui, key: Key| ui.state.take_key(|k| k.key == key && k.mods.is_empty()).is_some();
        let mut pick = false;
        if take(ui, Key::Escape) {
            self.completion = None;
            return (changed, out);
        }
        let c = self.completion.as_mut().unwrap();
        let at = shown.iter().position(|i| *i == c.selected).unwrap_or(0);
        if take(ui, Key::ArrowDown) {
            c.selected = shown[(at + 1) % shown.len()];
        }
        if take(ui, Key::ArrowUp) {
            c.selected = shown[(at + shown.len() - 1) % shown.len()];
        }
        if take(ui, Key::PageDown) {
            c.selected = shown[(at + LIST_ROWS).min(shown.len() - 1)];
        }
        if take(ui, Key::PageUp) {
            c.selected = shown[at.saturating_sub(LIST_ROWS)];
        }
        if take(ui, Key::Tab) || take(ui, Key::Enter) {
            pick = true;
        }
        if pick {
            let item = c.items[c.selected].clone();
            self.completion = None;
            apply(doc, &item, self.utf16, now);
            changed = true;
        }
        (changed, out)
    }

    /// After the editor handled its keys: the popups follow or close,
    /// `.` or `::` asks for completions, `(` or `,` for the signature.
    pub fn after_edit(&mut self, doc: &Doc, text_changed: bool) -> LspOut {
        let mut out = LspOut::default();
        let cur = doc.cursor;
        if let Some(c) = &self.completion
            && (c.doc != doc.id || cur.line != c.anchor.line || cur.col < c.anchor.col)
        {
            self.completion = None;
        }
        if let Some(s) = &self.signature
            && (s.doc != doc.id || cur.line != s.line)
        {
            self.signature = None;
        }
        if !text_changed {
            // The caret moved without typing: the signature is stale.
            if self.signature.is_some() && self.sig_cursor != Some(cur) {
                self.signature = None;
            }
            return out;
        }
        self.hover = None;
        let before = &doc.line(cur.line)[..cur.col];
        if before.ends_with('.') {
            out.complete = Some((cur, Some('.')));
        } else if before.ends_with("::") {
            out.complete = Some((cur, Some(':')));
        }
        match before.chars().next_back() {
            Some(ch @ ('(' | ',')) => {
                out.signature = Some((cur, Some(ch), self.signature.is_some()));
                self.sig_cursor = Some(cur);
            }
            Some(')') => self.signature = None,
            _ => {
                if self.signature.is_some() {
                    out.signature = Some((cur, None, true));
                    self.sig_cursor = Some(cur);
                }
            }
        }
        out
    }

    /// The pointer over the text: a rest asks for a hover, a move away
    /// closes it. Returns the position to ask about.
    pub fn pointer(&mut self, ui: &mut Ui, doc: DocId, at: Pos, now: f64) -> Option<Pos> {
        let p = ui.state.pointer;
        let moved = self.rest.is_none_or(|(r, _)| (r - p).length() > 2.0);
        if moved {
            self.rest = Some((p, now));
            if let Some(h) = &self.hover
                && (h.anchor - p).length() > ui.m.widget_h * 1.5
            {
                self.hover = None;
            }
            ui.state.request_redraw_after(HOVER_REST + 0.02);
            return None;
        }
        let (_, since) = self.rest?;
        if now - since < HOVER_REST {
            ui.state.request_redraw_after(HOVER_REST - (now - since) + 0.02);
            return None;
        }
        if self.hover.as_ref().is_some_and(|h| h.doc == doc && h.pos == at) || self.asked.is_some_and(|(d, q, _)| d == doc && q == at) {
            return None;
        }
        self.asked = Some((doc, at, p));
        Some(at)
    }

    /// The popups, over the text. Returns a completion picked with the
    /// mouse.
    pub fn draw(&mut self, ui: &mut Ui, doc: &Doc, g: Geom, style: &lntrn_text::TextStyle) -> Option<CompletionItem> {
        let Geom { vp, text_x0, origin_y, cell_w, lh } = g;
        let m = ui.m;
        let theme = ui.theme;
        let tab = doc.tab();
        let saved = ui.draw.layer();
        ui.draw.set_layer(saved + 2);
        let mut picked = None;
        if let Some(c) = &self.completion
            && c.doc == doc.id
        {
            let shown = self.filtered(doc);
            if shown.is_empty() {
                ui.draw.set_layer(saved);
                self.completion = None;
                return None;
            }
            let ts = ui.text_style();
            let row_h = m.widget_h;
            let rows = shown.len().min(LIST_ROWS);
            let mut w: f64 = cell_w * 24.0;
            for i in shown.iter().take(60) {
                let it = &c.items[*i];
                w = w.max(ui.measure(&it.label, &ts) + ui.measure(&it.detail, &ts) + m.pad * 4.0);
            }
            let w = w.min(vp.width() * 0.7);
            let h = rows as f64 * row_h + m.gap * 2.0;
            let x = (text_x0 + cell_of_byte(doc.line(c.anchor.line), tab, c.anchor.col) as f64 * cell_w).min(vp.max.x - w - m.gap).max(vp.min.x);
            let below = origin_y + (c.anchor.line + 1) as f64 * lh + m.gap;
            let y = if below + h > vp.max.y { origin_y + c.anchor.line as f64 * lh - m.gap - h } else { below };
            let panel = Rect::from_min_size(Vec2::new(x.round(), y.round()), Vec2::new(w, h));
            ui.floating_panel(panel, theme.panel);
            let at = shown.iter().position(|i| *i == c.selected).unwrap_or(0);
            let first = at.saturating_sub(rows - 1).min(shown.len() - rows);
            let first = if at < first { at } else { first };
            for (row, i) in shown.iter().enumerate().skip(first).take(rows) {
                let it = &c.items[*i];
                let rect = Rect::from_min_size(Vec2::new(panel.min.x, panel.min.y + m.gap + (row - first) as f64 * row_h), Vec2::new(w, row_h));
                let id = ui.id("completion").with_index(row);
                let r = ui.interact(id, rect, Sense::CLICK);
                if r.clicked {
                    picked = Some(it.clone());
                }
                let selected = *i == c.selected;
                if selected {
                    ui.fill_shaded(rect, theme.shaded(theme.selection));
                } else if r.hovered {
                    ui.fill(rect, theme.hover(theme.panel.mid()));
                }
                let color = if selected { theme.selection_text } else { theme.text };
                let dim = if selected { theme.selection_text.fade(0.75) } else { theme.text_dim };
                let dw = ui.measure(&it.detail, &ts);
                ui.text_in_rect(&it.label, &ts, Rect::new(Vec2::new(rect.min.x + m.pad, rect.min.y), Vec2::new((rect.max.x - dw - m.pad * 2.0).max(rect.min.x + m.pad), rect.max.y)), color);
                if !it.detail.is_empty() {
                    ui.text_in_rect(&it.detail, &ts, Rect::new(Vec2::new((rect.max.x - dw - m.pad).max(rect.min.x), rect.min.y), Vec2::new(rect.max.x - m.pad, rect.max.y)), dim);
                }
            }
        }
        if let Some(s) = &self.signature
            && s.doc == doc.id
        {
            let h = &s.help;
            let ts = ui.text_style();
            let under = h.doc.clone().map(|d| if h.count > 1 { format!("{d}   ({} of {})", h.index + 1, h.count) } else { d }).or_else(|| (h.count > 1).then(|| format!("{} of {} overloads", h.index + 1, h.count)));
            let mut w = ui.measure(&h.label, style);
            if let Some(u) = &under {
                w = w.max(ui.measure(u, &ts));
            }
            let w = (w + m.pad * 2.0).min(vp.width() - m.gap * 2.0).max(cell_w * 8.0);
            let hgt = lh + if under.is_some() { m.widget_h } else { 0.0 } + m.pad * 2.0;
            let row_top = origin_y + s.line as f64 * lh;
            let line_text = doc.line(s.line);
            let x = (text_x0 + cell_of_byte(line_text, tab, doc.cursor.col.min(line_text.len())) as f64 * cell_w - m.pad).min(vp.max.x - w - m.gap).max(vp.min.x + m.gap);
            let above = row_top - m.gap - hgt;
            let y = if above < vp.min.y { row_top + lh + m.gap } else { above };
            let panel = Rect::from_min_size(Vec2::new(x.round(), y.round()), Vec2::new(w, hgt));
            ui.floating_panel(panel, theme.header);
            ui.draw.push_clip(panel);
            let (lx, ly) = (panel.min.x + m.pad, panel.min.y + m.pad);
            if let Some((a, b)) = h.active
                && a < b
                && b <= h.label.len()
            {
                let x0 = lx + ui.measure(&h.label[..a], style);
                let x1 = lx + ui.measure(&h.label[..b], style);
                ui.draw.rounded_rect(Rect::new(Vec2::new(x0, ly), Vec2::new(x1, ly + lh)), m.radius * 0.5, theme.accent.fade(0.35));
            }
            let mut quads = Vec::new();
            ui.text.place(&h.label, style, lx as f32, ly as f32, 1.0e6, theme.text.to_gpu(), &mut quads);
            ui.draw.glyphs(&quads);
            if let Some(u) = &under {
                ui.text_in_rect(u, &ts, Rect::new(Vec2::new(lx, ly + lh), Vec2::new(panel.max.x - m.pad, ly + lh + m.widget_h)), theme.text_dim);
            }
            ui.draw.pop_clip();
        }
        if let Some(h) = &self.hover
            && h.doc == doc.id
        {
            let n = h.lines.len().min(14);
            let mut w: f64 = 0.0;
            for l in &h.lines[..n] {
                w = w.max(ui.measure(l, style));
            }
            let w = (w + m.pad * 2.0).min(vp.width() - m.gap * 2.0).max(cell_w * 8.0);
            let hgt = n as f64 * lh + m.pad * 2.0;
            let row_top = origin_y + h.pos.line as f64 * lh;
            let x = h.anchor.x.min(vp.max.x - w - m.gap).max(vp.min.x + m.gap);
            let y = if row_top + lh + m.gap + hgt > vp.max.y { row_top - m.gap - hgt } else { row_top + lh + m.gap };
            let panel = Rect::from_min_size(Vec2::new(x.round(), y.round()), Vec2::new(w, hgt));
            ui.floating_panel(panel, theme.header);
            ui.draw.push_clip(panel);
            let mut quads = Vec::new();
            for (i, l) in h.lines[..n].iter().enumerate() {
                quads.clear();
                ui.text.place(l, style, (panel.min.x + m.pad) as f32, (panel.min.y + m.pad + i as f64 * lh) as f32, 1.0e6, theme.text.to_gpu(), &mut quads);
                ui.draw.glyphs(&quads);
            }
            ui.draw.pop_clip();
        }
        ui.draw.set_layer(saved);
        picked
    }
}

/// Put a picked completion into the document: its extra edits (imports)
/// first, then the word itself, the caret after it.
pub fn apply(doc: &mut Doc, item: &CompletionItem, utf16: bool, now: f64) {
    let main = item.edit.clone().unwrap_or_else(|| {
        let cur = doc.cursor;
        let start = word_start(doc.line(cur.line), cur.col);
        // The editor's own columns: bytes, whatever the server counts.
        let line = doc.line(cur.line);
        let to_units = |b: usize| crate::lsp::pos::to_units(line, b, utf16);
        TextEdit { line: cur.line, col: to_units(start), end_line: cur.line, end_col: to_units(cur.col), text: item.insert.clone() }
    });
    let mut extras: Vec<&TextEdit> = item.extra.iter().collect();
    extras.sort_by_key(|e| std::cmp::Reverse((e.line, e.col)));
    let mut shift = 0usize;
    for e in extras {
        let r = range_of(doc, e, utf16, 0);
        doc.edit(r, &e.text, EditKind::Other, now);
        if e.end_line <= main.line {
            shift += e.text.matches('\n').count();
            shift = shift.saturating_sub(e.end_line - e.line);
        }
    }
    let r = range_of(doc, &main, utf16, shift);
    let end = doc.edit(r, &main.text, EditKind::Other, now);
    doc.set_cursor(end, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_picks() {
        assert_eq!(word_start("let foo_bar = x", 11), 4);
        assert_eq!(word_start("a.b", 2), 2);
        assert_eq!(word_start("", 0), 0);
        let mut doc = Doc::from_text(DocId(1), None, "use std::x;\nfn main() {\n    v.pu\n}\n", 4);
        doc.set_cursor(Pos::new(2, 8), false);
        let item = CompletionItem { label: "push(…)".into(), detail: String::new(), kind: 2, insert: "push(value)".into(), edit: None, extra: vec![TextEdit { line: 0, col: 0, end_line: 0, end_col: 0, text: "use std::y;\n".into() }], filter: "push".into(), sort: String::new() };
        apply(&mut doc, &item, false, 0.0);
        assert_eq!(doc.line(0), "use std::y;");
        assert_eq!(doc.line(3), "    v.push(value)", "the word was replaced one line further down");
        assert_eq!(doc.cursor, Pos::new(3, 17));
        let edit = CompletionItem { label: "len".into(), detail: "fn".into(), kind: 3, insert: "len()".into(), edit: Some(TextEdit { line: 3, col: 6, end_line: 3, end_col: 17, text: "len()".into() }), extra: Vec::new(), filter: "len".into(), sort: String::new() };
        apply(&mut doc, &edit, false, 0.0);
        assert_eq!(doc.line(3), "    v.len()");
    }
}

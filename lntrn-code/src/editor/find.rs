//! Find and replace: the matches of a query in a document, the bar that
//! edits the query, and stepping through and replacing the hits.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, TextOpts, Ui};

use crate::buffer::{Pos, Range};
use crate::doc::{Doc, DocId, EditKind};
use crate::text_util::is_word;

#[derive(Default)]
pub struct Finder {
    pub open: bool,
    pub replace_open: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub matches: Vec<Range>,
    pub current: Option<usize>,
    /// Put the caret in the query field on the next draw.
    want_focus: bool,
    /// What `matches` was computed for.
    seen: Option<(DocId, u64, String, bool, bool)>,
}

/// What the bar did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOut {
    pub closed: bool,
    /// The document changed (a replacement).
    pub changed: bool,
}

/// Every hit of `q` in `line`, as byte ranges.
pub fn find_in_line(line: &str, q: &str, case_sensitive: bool, whole_word: bool, out: &mut Vec<(usize, usize)>) {
    if q.is_empty() || q.len() > line.len() {
        return;
    }
    let (lb, qb) = (line.as_bytes(), q.as_bytes());
    let mut i = 0;
    while i + qb.len() <= lb.len() {
        if !line.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let hit = if case_sensitive { &lb[i..i + qb.len()] == qb } else { lb[i..i + qb.len()].eq_ignore_ascii_case(qb) };
        if hit && line.is_char_boundary(i + qb.len()) {
            let end = i + qb.len();
            let bounded = !whole_word || (!line[..i].chars().next_back().is_some_and(is_word) && !line[end..].chars().next().is_some_and(is_word));
            if bounded {
                out.push((i, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

impl Finder {
    /// Open the bar (and the replace row when `replace`), seeding the
    /// query from a one-line selection.
    pub fn show(&mut self, doc: &Doc, replace: bool) {
        self.open = true;
        self.replace_open |= replace;
        let sel = doc.selection();
        if !sel.is_empty() && sel.start.line == sel.end.line {
            self.query = doc.buffer.text_in(sel);
        }
        self.want_focus = true;
        self.seen = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.replace_open = false;
        self.matches.clear();
        self.current = None;
        self.seen = None;
    }

    /// Bring `matches` up to date with the document and the query.
    pub fn refresh(&mut self, doc: &Doc) {
        let key = (doc.id, doc.buffer.version(), self.query.clone(), self.case_sensitive, self.whole_word);
        if self.seen.as_ref() == Some(&key) {
            return;
        }
        self.seen = Some(key);
        self.matches.clear();
        let mut hits = Vec::new();
        if !self.query.is_empty() && !self.query.contains('\n') {
            for (i, line) in doc.buffer.lines().iter().enumerate() {
                hits.clear();
                find_in_line(line, &self.query, self.case_sensitive, self.whole_word, &mut hits);
                self.matches.extend(hits.iter().map(|&(a, b)| Range::new(Pos::new(i, a), Pos::new(i, b))));
            }
        }
        // The current hit: the one selected, else the first at or after the caret.
        let sel = doc.selection();
        self.current = if self.matches.is_empty() {
            None
        } else if let Some(i) = self.matches.iter().position(|r| *r == sel) {
            Some(i)
        } else {
            Some(self.matches.iter().position(|r| r.start >= sel.start).unwrap_or(0))
        };
    }

    /// Select the next (or previous) hit, wrapping.
    pub fn step(&mut self, doc: &mut Doc, forward: bool) {
        self.refresh(doc);
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        let sel = doc.selection();
        let on_current = self.current.is_some_and(|i| self.matches[i] == sel);
        let i = match (self.current, forward) {
            (Some(i), true) if on_current => (i + 1) % n,
            (Some(i), true) => i,
            (Some(i), false) => (i + n - 1) % n,
            (None, _) => 0,
        };
        self.current = Some(i);
        doc.select(self.matches[i]);
    }

    /// Replace the selected hit and step to the next. Returns whether
    /// anything changed.
    pub fn replace_current(&mut self, doc: &mut Doc, now: f64) -> bool {
        self.refresh(doc);
        let Some(i) = self.current else {
            return false;
        };
        if self.matches[i] != doc.selection() {
            self.step(doc, true);
            return false;
        }
        let r = self.matches[i];
        doc.edit(r, &self.replacement, EditKind::Other, now);
        self.seen = None;
        self.refresh(doc);
        if !self.matches.is_empty() {
            let next = self.matches.iter().position(|m| m.start >= doc.cursor).unwrap_or(0);
            self.current = Some(next);
            doc.select(self.matches[next]);
        }
        true
    }

    /// Replace every hit as one undo step. Returns how many.
    pub fn replace_all(&mut self, doc: &mut Doc, now: f64) -> usize {
        self.refresh(doc);
        if self.matches.is_empty() {
            return 0;
        }
        let count = self.matches.len();
        let mut lines: Vec<String> = doc.buffer.lines().to_vec();
        for m in self.matches.iter().rev() {
            lines[m.start.line].replace_range(m.start.col..m.end.col, &self.replacement);
        }
        let caret = doc.cursor;
        let whole = Range::new(Pos::new(0, 0), doc.buffer.end());
        doc.edit(whole, &lines.join("\n"), EditKind::Other, now);
        doc.set_cursor(caret, false);
        self.seen = None;
        count
    }
}

/// A small toggle that lights up in the accent when on.
fn toggle_button(ui: &mut Ui, label: &str, on: &mut bool, tip: &str) -> bool {
    let style = ui.text_style();
    let w = ui.measure(label, &style) + ui.m.pad * 2.0;
    let rect = ui.alloc(Vec2::new(w, ui.m.widget_h));
    let id = ui.id(label);
    let mut r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    if r.clicked {
        *on = !*on;
    }
    let theme = ui.theme;
    if *on {
        ui.raised(rect, theme.shaded(theme.accent), r.held);
        ui.text_centered(label, &style, rect, theme.accent_text);
    } else {
        ui.button_face(rect, &r);
        ui.text_centered(label, &style, rect, theme.text);
    }
    ui.focus_ring(id, rect);
    ui.tooltip(&r, tip);
    r.clicked
}

/// The bar: query, options, the count, previous/next, replace, close.
pub fn draw_find_bar(ui: &mut Ui, f: &mut Finder, doc: &mut Doc) -> FindOut {
    let mut out = FindOut::default();
    let m = ui.m;
    let now = ui.state.now;
    let mut step: Option<bool> = None;
    let mut replace_one = false;
    let mut replace_all = false;
    ui.row(|ui| {
        let id = ui.id("query");
        if f.want_focus {
            f.want_focus = false;
            ui.state.focus = Some(id);
            let te = ui.state.text_edit(id);
            te.anchor = 0;
            te.cursor = f.query.len();
        }
        let w = (ui.avail_width() * 0.35).max(m.px(220.0));
        let r = ui.alloc(Vec2::new(w, m.widget_h));
        let resp = ui.text_edit_core_with(id, r, &mut f.query, TextOpts { placeholder: "Find", ..TextOpts::default() });
        if resp.committed {
            step = Some(!ui.state.mods.shift());
        }
        if resp.cancelled {
            out.closed = true;
        }
        toggle_button(ui, "Aa", &mut f.case_sensitive, "Match case");
        toggle_button(ui, "Word", &mut f.whole_word, "Whole words only");
        f.refresh(doc);
        let count = match (f.matches.len(), f.current) {
            (0, _) if f.query.is_empty() => String::new(),
            (0, _) => "No results".to_owned(),
            (n, Some(i)) => format!("{} of {n}", i + 1),
            (n, None) => format!("{n} found"),
        };
        ui.label_dim(&count);
        if ui.button("↑").clicked {
            step = Some(false);
        }
        if ui.button("↓").clicked {
            step = Some(true);
        }
        let mut rep = f.replace_open;
        if toggle_button(ui, "Replace", &mut rep, "Show the replace row") {
            f.replace_open = rep;
        }
        let close_w = m.widget_h;
        let spacer = (ui.avail_width() - close_w - m.gap).max(0.0);
        ui.alloc(Vec2::new(spacer, m.widget_h));
        if ui.button_sized("×", Vec2::new(close_w, m.widget_h)).clicked {
            out.closed = true;
        }
    });
    if f.replace_open {
        ui.row(|ui| {
            let id = ui.id("replacement");
            let w = (ui.avail_width() * 0.35).max(m.px(220.0));
            let r = ui.alloc(Vec2::new(w, m.widget_h));
            let resp = ui.text_edit_core_with(id, r, &mut f.replacement, TextOpts { placeholder: "Replace with", ..TextOpts::default() });
            if resp.committed {
                replace_one = true;
            }
            if resp.cancelled {
                out.closed = true;
            }
            if ui.button("Replace").clicked {
                replace_one = true;
            }
            if ui.button("Replace All").clicked {
                replace_all = true;
            }
            let _ = ui.alloc(Vec2::new(FILL, m.widget_h));
        });
    }
    if let Some(forward) = step {
        f.step(doc, forward);
        ui.state.request_rebuild = true;
    }
    if replace_one {
        out.changed |= f.replace_current(doc, now);
        ui.state.request_rebuild = true;
    }
    if replace_all {
        let n = f.replace_all(doc, now);
        out.changed |= n > 0;
        ui.state.request_rebuild = true;
    }
    if out.closed {
        f.close();
        ui.state.request_rebuild = true;
    }
    let _ = Rect::ZERO;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_search() {
        let mut out = Vec::new();
        find_in_line("Foo foo food", "foo", false, false, &mut out);
        assert_eq!(out, vec![(0, 3), (4, 7), (8, 11)]);
        out.clear();
        find_in_line("Foo foo food", "foo", true, false, &mut out);
        assert_eq!(out, vec![(4, 7), (8, 11)]);
        out.clear();
        find_in_line("Foo foo food", "foo", false, true, &mut out);
        assert_eq!(out, vec![(0, 3), (4, 7)]);
        out.clear();
        find_in_line("héllo héllo", "llo", false, false, &mut out);
        assert_eq!(out.len(), 2);
        out.clear();
        find_in_line("aaa", "aa", false, false, &mut out);
        assert_eq!(out, vec![(0, 2)], "no overlapping hits");
    }

    #[test]
    fn stepping_and_replacing() {
        let mut doc = Doc::from_text(DocId(3), None, "x = a\ny = a + a\n", 4);
        let mut f = Finder { query: "a".into(), ..Finder::default() };
        f.refresh(&doc);
        assert_eq!(f.matches.len(), 3);
        assert_eq!(f.current, Some(0));
        f.step(&mut doc, true);
        assert_eq!(doc.selection(), Range::new(Pos::new(0, 4), Pos::new(0, 5)));
        f.step(&mut doc, true);
        assert_eq!(f.current, Some(1));
        f.step(&mut doc, false);
        assert_eq!(f.current, Some(0));
        f.replacement = "bb".into();
        assert!(f.replace_current(&mut doc, 0.0));
        assert_eq!(doc.buffer.line(0), "x = bb");
        assert_eq!(f.matches.len(), 2, "recounted after the edit");
        assert_eq!(doc.selection(), Range::new(Pos::new(1, 4), Pos::new(1, 5)), "moved on to the next hit");
        assert_eq!(f.replace_all(&mut doc, 1.0), 2);
        assert_eq!(doc.buffer.line(1), "y = bb + bb");
        assert!(doc.undo(2.0));
        assert_eq!(doc.buffer.line(1), "y = a + a", "replace all is one step");
    }
}

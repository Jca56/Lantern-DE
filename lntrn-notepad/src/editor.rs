//! A document being edited: the caret and what is selected, typing and
//! deleting, moving about, and undo. What dresses text and paragraphs is
//! in `editor_ops.rs`.
//!
//! Nothing here draws or reads input: the view says what was pressed, and
//! tests say the same.

use std::sync::Arc;

use lntrn_core::Undo;
use lntrn_text::TextEngine;

use crate::doc::{Doc, List, Para, Picture, Pos, Style, TextAttrs};
use crate::view::layout::{Layout, Setting};

/// Typing this soon after the last of it is the same undo step.
const TYPING_STEP: f64 = 1.0;

/// The document and where the caret was, to go back to.
#[derive(Clone)]
struct Snapshot {
    doc: Doc,
    caret: Pos,
    anchor: Option<Pos>,
}

/// What was cut or copied: as text for anywhere, and as it was for
/// another document of ours.
#[derive(Clone, Default)]
pub struct Clip {
    pub text: String,
    pub paras: Vec<Para>,
    pub pictures: Vec<(u64, Arc<Picture>)>,
}

#[derive(Default)]
pub struct Editor {
    pub doc: Doc,
    pub caret: Pos,
    /// Where a selection started; it runs from here to the caret.
    pub anchor: Option<Pos>,
    /// What the next typed character is to have: set by a format chosen
    /// with nothing selected, forgotten when the caret moves.
    pub pending: Option<TextAttrs>,
    pub layout: Layout,
    /// The x the caret keeps to through Up and Down.
    goal_x: Option<f32>,
    undo: Undo<Snapshot>,
    /// Counts every change, and what it was at when last saved.
    pub rev: u64,
    pub saved: u64,
    /// The frame's time, for typing to be one undo step.
    pub now: f64,
    /// The caret moved: the view should keep it in sight.
    pub follow: bool,
}

impl Editor {
    pub fn of(doc: Doc) -> Editor {
        Editor { doc, ..Editor::default() }
    }

    pub fn modified(&self) -> bool {
        self.rev != self.saved
    }

    /// Set the document on a page: the caret's moves up and down need it.
    pub fn sync(&mut self, text: &mut TextEngine, setting: Setting) -> bool {
        self.layout.sync(text, &self.doc, setting)
    }

    // ---- selection ----

    /// The selection's ends in order, when something is selected.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let anchor = self.anchor.filter(|a| *a != self.caret)?;
        Some(if anchor <= self.caret { (anchor, self.caret) } else { (self.caret, anchor) })
    }

    pub fn select(&mut self, from: Pos, to: Pos) {
        (self.anchor, self.caret) = (Some(self.doc.clamp(from)), self.doc.clamp(to));
        self.moved();
    }

    pub fn select_all(&mut self) {
        (self.anchor, self.caret) = (Some(Pos::default()), self.doc.end());
        self.pending = None;
    }

    pub(crate) fn moved(&mut self) {
        (self.pending, self.goal_x, self.follow) = (None, None, true);
    }

    /// Put the caret at `to`; with `selecting`, what it passes over is
    /// selected.
    pub fn move_to(&mut self, to: Pos, selecting: bool) {
        self.anchor = if selecting { self.anchor.or(Some(self.caret)) } else { None };
        self.caret = self.doc.clamp(to);
        self.moved();
    }

    /// A step left or right. With something selected and not selecting,
    /// the caret goes to that end of it instead.
    pub fn step(&mut self, right: bool, word: bool, selecting: bool) {
        let to = match (self.selection(), selecting) {
            (Some((a, b)), false) => if right { b } else { a },
            _ => match (right, word) {
                (false, false) => self.doc.left(self.caret),
                (true, false) => self.doc.right(self.caret),
                (false, true) => self.doc.word_left(self.caret),
                (true, true) => self.doc.word_right(self.caret),
            },
        };
        self.move_to(to, selecting);
    }

    /// Up or down by `pixels` of the page (a row is one of its own
    /// height), keeping to the x the caret set out from.
    pub fn climb(&mut self, pixels: f32, selecting: bool) {
        let (x, y, h) = self.layout.caret(self.caret);
        let goal = self.goal_x.unwrap_or(x);
        let to = if pixels.abs() <= 1.0 {
            self.layout.step(self.caret, if pixels < 0.0 { -1 } else { 1 }, goal)
        } else {
            Some(self.layout.hit(goal, (y + h * 0.5 + pixels).clamp(0.0, (self.layout.height() - 1.0).max(0.0))))
        };
        // From the first row up is the row's start; from the last down,
        // its end.
        let to = to.unwrap_or(if pixels < 0.0 { Pos::default() } else { self.doc.end() });
        self.move_to(to, selecting);
        self.goal_x = Some(goal);
    }

    /// Home and End: the row's ends, or with `whole` the document's.
    pub fn edge(&mut self, end: bool, whole: bool, selecting: bool) {
        let (start, rest) = if whole { (Pos::default(), self.doc.end()) } else { self.layout.row_ends(self.caret) };
        self.move_to(if end { rest } else { start }, selecting);
    }

    // ---- changing ----

    /// Remember things as they are before a change. Typing soon after
    /// typing is one step.
    pub(crate) fn step_back(&mut self, typing: bool) {
        let snapshot = Snapshot { doc: self.doc.clone(), caret: self.caret, anchor: self.anchor };
        if typing { self.undo.record(snapshot, self.now, TYPING_STEP) } else { self.undo.push(snapshot) }
        self.rev += 1;
    }

    fn restore(&mut self, s: Snapshot) {
        (self.doc, self.caret, self.anchor) = (s.doc, s.caret, s.anchor);
        self.rev += 1;
        self.moved();
    }

    pub fn undo(&mut self) -> bool {
        let now = Snapshot { doc: self.doc.clone(), caret: self.caret, anchor: self.anchor };
        self.undo.undo(now).map(|s| self.restore(s)).is_some()
    }

    pub fn redo(&mut self) -> bool {
        let now = Snapshot { doc: self.doc.clone(), caret: self.caret, anchor: self.anchor };
        self.undo.redo(now).map(|s| self.restore(s)).is_some()
    }

    pub fn can_undo(&self) -> (bool, bool) {
        (self.undo.can_undo(), self.undo.can_redo())
    }

    /// Take the selection out, if there is one. The caller has taken the
    /// undo step.
    pub(crate) fn cut_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { return false };
        self.caret = self.doc.delete(a, b);
        self.anchor = None;
        true
    }

    /// What typing at the caret has: what was asked for, else what the
    /// text there has.
    pub fn typing_attrs(&self) -> TextAttrs {
        self.pending.unwrap_or_else(|| self.doc.typing_attrs(self.selection().map_or(self.caret, |(a, _)| self.doc.right(a))))
    }

    /// Type `text` over the selection. Typed text keeps what was pending;
    /// a paste does not need it to.
    pub fn type_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let attrs = self.typing_attrs();
        let typed = !text.contains('\n');
        if typed && self.selection().is_some() {
            // Over a selection: a step of its own, which the typing that
            // follows joins, so one undo gives the selection back.
            let snapshot = Snapshot { doc: self.doc.clone(), caret: self.caret, anchor: self.anchor };
            self.undo.record(snapshot, self.now, 0.0);
            self.rev += 1;
        } else {
            self.step_back(typed);
        }
        self.cut_selection();
        self.caret = self.doc.insert_text(self.caret, text, attrs);
        let keep = self.pending;
        self.moved();
        self.pending = keep;
        if text == " " {
            self.shorthand();
        }
    }

    /// A paragraph that starts with a mark and a blank becomes what the
    /// mark stands for: `- ` a bullet, `1. ` a number, `[] ` a box, `# `
    /// the title and `## ` to `#### ` the headings (as a Markdown file
    /// has them), `> ` a quote. Undo gives the characters back.
    fn shorthand(&mut self) {
        let p = self.doc.para(self.caret.para);
        if p.attrs.list != List::None || p.attrs.style != Style::Body || p.is_picture() {
            return;
        }
        let (list, style) = match &p.text[..self.caret.byte] {
            "- " | "* " => (List::Bullet, Style::Body),
            "1. " | "1) " => (List::Number, Style::Body),
            "[] " | "[ ] " => (List::Check(false), Style::Body),
            "# " => (List::None, Style::Title),
            "## " => (List::None, Style::Heading1),
            "### " => (List::None, Style::Heading2),
            "#### " => (List::None, Style::Heading3),
            "> " => (List::None, Style::Quote),
            _ => return,
        };
        self.undo.push(Snapshot { doc: self.doc.clone(), caret: self.caret, anchor: None });
        let start = Pos::new(self.caret.para, 0);
        self.caret = self.doc.delete(start, self.caret);
        self.doc.set_paras(start.para, start.para, |a| (a.list, a.style) = (list, style));
    }

    /// Enter. On a list item with nothing in it the list ends there.
    pub fn enter(&mut self) {
        self.step_back(false);
        self.cut_selection();
        let p = self.doc.para(self.caret.para);
        if p.attrs.list != List::None && p.text.is_empty() {
            let para = self.caret.para;
            self.doc.set_paras(para, para, |a| if a.level > 0 { a.level -= 1 } else { a.list = List::None });
        } else {
            self.caret = self.doc.split(self.caret);
        }
        self.moved();
    }

    /// Backspace (`back`) or Delete, a character or a `word`. At the
    /// start of a list item Backspace takes the item's mark first.
    pub fn erase(&mut self, back: bool, word: bool) {
        if self.selection().is_some() {
            self.step_back(false);
            self.cut_selection();
            return self.moved();
        }
        let attrs = self.doc.para(self.caret.para).attrs;
        if back && self.caret.byte == 0 && (attrs.list != List::None || attrs.style != Style::Body) {
            self.step_back(false);
            let para = self.caret.para;
            self.doc.set_paras(para, para, |a| if a.list != List::None { a.list = List::None } else { a.style = Style::Body });
            self.doc.set_paras(para, para, |a| a.level = 0);
            return self.moved();
        }
        let to = match (back, word) {
            (true, false) => self.doc.left(self.caret),
            (true, true) => self.doc.word_left(self.caret),
            (false, false) => self.doc.right(self.caret),
            (false, true) => self.doc.word_right(self.caret),
        };
        if to == self.caret {
            return;
        }
        self.step_back(!word);
        self.caret = self.doc.delete(self.caret, to);
        self.moved();
    }

    /// Tab: a list item goes a level in (or out, with `back`); anywhere
    /// else it is a tab in the text.
    pub fn tab(&mut self, back: bool) {
        let (from, to) = self.selection().map_or((self.caret.para, self.caret.para), |(a, b)| (a.para, b.para));
        if (from..=to).any(|i| self.doc.para(i).attrs.list != List::None) {
            self.nest(if back { -1 } else { 1 });
        } else if !back {
            self.type_text("\t");
        }
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;

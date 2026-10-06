//! Find and replace: a bar over the page with what to look for, how many
//! places it is and which one the page is on, and what to put there
//! instead.

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui, WidgetId};

use crate::doc::{Find, Pos};
use crate::editor::Editor;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    /// The replace half shows too.
    pub replacing: bool,
    pub find: Find,
    pub with: String,
    /// Every place it is, and the one the page is on.
    pub hits: Vec<(Pos, Pos)>,
    pub at: Option<usize>,
    /// What the hits were found for: the document as it was, and what
    /// was looked for.
    seen: Option<(u64, Find)>,
    /// The find field takes the keyboard on the next frame.
    focus: bool,
}

impl FindBar {
    /// Open it, looking for `text` when there is some (what was selected).
    pub fn show(&mut self, replacing: bool, text: Option<String>) {
        (self.open, self.replacing, self.focus) = (true, replacing, true);
        if let Some(text) = text.filter(|t| !t.is_empty() && !t.contains('\n')) {
            self.find.text = text;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.hits.clear();
        (self.at, self.seen) = (None, None);
    }

    /// Find again if the document or what is looked for has changed.
    pub fn refresh(&mut self, ed: &Editor) {
        if !self.open || self.seen.as_ref().is_some_and(|(rev, find)| *rev == ed.rev && *find == self.find) {
            return;
        }
        self.hits = self.find.all(&ed.doc);
        self.seen = Some((ed.rev, self.find.clone()));
        // The one the page is on is the first at or after the caret.
        let from = ed.selection().map_or(ed.caret, |(a, _)| a);
        self.at = self.hits.iter().position(|(a, _)| *a >= from).or((!self.hits.is_empty()).then_some(0));
    }

    /// Go to the next place (`by` 1) or the one before (−1), round the
    /// ends, and select it.
    pub fn go(&mut self, ed: &mut Editor, by: i64) {
        self.refresh(ed);
        if self.hits.is_empty() {
            return;
        }
        let n = self.hits.len() as i64;
        // From a place the page is already on, the step; else that place.
        let on = self.at.filter(|i| ed.selection() == Some(self.hits[*i]));
        let i = on.map_or(self.at.unwrap_or(0) as i64, |i| i as i64 + by).rem_euclid(n) as usize;
        self.at = Some(i);
        ed.select(self.hits[i].0, self.hits[i].1);
    }

    /// Replace the place the page is on and go to the next.
    pub fn replace(&mut self, ed: &mut Editor) {
        self.refresh(ed);
        let Some((a, b)) = self.at.and_then(|i| self.hits.get(i).copied()) else { return };
        ed.replace(a, b, &self.with);
        self.refresh(ed);
        if !self.hits.is_empty() {
            self.at = self.hits.iter().position(|(start, _)| *start >= ed.caret).or(Some(0));
            let (a, b) = self.hits[self.at.unwrap_or(0)];
            ed.select(a, b);
        }
    }

    /// Draw the bar. Returns whether the page should take the keyboard
    /// back (it was closed).
    pub fn draw(&mut self, ui: &mut Ui, ed: &mut Editor) -> bool {
        if !self.open {
            return false;
        }
        self.refresh(ed);
        let m = ui.m;
        let (h, gap) = (m.px(controls::BUTTON_H), m.px(8.0));
        let rows = if self.replacing { 2.0 } else { 1.0 };
        let area = ui.alloc(Vec2::new(FILL, (h + gap) * rows));
        let id = ui.id("find");
        let style = ui.text_style();
        let field_w = (area.width() * 0.42).clamp(m.px(180.0), m.px(460.0));
        let mut x = area.min.x;
        // The next control's place on a row: where the pen is, which
        // then moves past it.
        let next = |x: &mut f64, w: f64, row: f64| {
            let rect = Rect::from_min_size(Vec2::new(*x, area.min.y + row * (h + gap)), Vec2::new(w, h));
            *x += w + gap;
            rect
        };
        let mut closed = false;

        // ---- what to look for ----
        let field = id.with("text");
        if std::mem::take(&mut self.focus) {
            ui.state.focus = Some(field);
        }
        let r = controls::text_field(ui, field, next(&mut x, field_w, 0.0), &mut self.find.text, "Find");
        if r.changed {
            self.refresh(ed);
            if let Some((a, b)) = self.at.map(|i| self.hits[i]) {
                ed.select(a, b);
            }
        }
        if r.committed {
            self.go(ed, if ui.state.mods.shift() { -1 } else { 1 });
            ui.state.focus = Some(field);
        }
        closed |= r.cancelled;
        let count = match (self.at, self.hits.len()) {
            (_, 0) if self.find.text.is_empty() => String::new(),
            (_, 0) => "None".to_owned(),
            (Some(i), n) => format!("{} of {n}", i + 1),
            (None, n) => format!("{n}"),
        };
        ui.text_centered(&count, &style, next(&mut x, m.px(110.0), 0.0), if self.hits.is_empty() { look::BAD } else { look::TEXT_DIM });
        for (name, text, by) in [("prev", "\u{2191}", -1), ("next", "\u{2193}", 1)] {
            if controls::button(ui, id.with(name), next(&mut x, h, 0.0), text) {
                self.go(ed, by);
            }
        }
        for (name, text, tip, on) in [("case", "Aa", "Match capitals", &mut self.find.exact_case), ("word", "\u{201C}W\u{201D}", "Whole words only", &mut self.find.whole_word)] {
            if toggle(ui, id.with(name), next(&mut x, m.px(64.0), 0.0), text, *on, tip) {
                *on = !*on;
            }
        }
        x = area.max.x - h;
        closed |= controls::button(ui, id.with("close"), next(&mut x, h, 0.0), "\u{00D7}");

        // ---- what to put there ----
        if self.replacing {
            x = area.min.x;
            let r = controls::text_field(ui, id.with("with"), next(&mut x, field_w, 1.0), &mut self.with, "Replace with");
            closed |= r.cancelled;
            let any = !self.hits.is_empty();
            let one = controls::button_of(ui, id.with("one"), next(&mut x, m.px(130.0), 1.0), "Replace", Kind::Plain, any) || (r.committed && any);
            if one {
                self.replace(ed);
            }
            if controls::button_of(ui, id.with("all"), next(&mut x, m.px(150.0), 1.0), "Replace All", Kind::Plain, any) {
                ed.replace_all(&self.find, &self.with);
                self.refresh(ed);
            }
        }
        if closed {
            self.close();
        }
        closed
    }
}

/// A button that stays down while what it stands for is on.
fn toggle(ui: &mut Ui, id: WidgetId, rect: Rect, text: &str, on: bool, tip: &str) -> bool {
    let m = ui.m;
    let r = ui.interact(id, rect, Sense::CLICK);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let (accent, radius) = (ui.theme.accent, m.px(12.0));
    ui.draw.rounded_rect(rect, radius, if on { accent.fade(0.22) } else if r.hovered { look::BUTTON.scale_rgb(1.35) } else { look::BUTTON });
    ui.draw.stroke_rect(rect, m.px(1.0), radius, if on { accent } else { look::TRACK });
    let style = ui.text_style();
    ui.text_centered(text, &style, rect, if on { accent.lerp(Color::WHITE, 0.25) } else { look::TEXT });
    ui.tooltip(&r, tip);
    r.clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    #[test]
    fn the_bar_walks_the_places_and_replaces_them() {
        let mut ed = Editor::of(Doc::from_text("a cat, a Cat\nand a cat"));
        let mut bar = FindBar::default();
        bar.show(false, Some("cat".into()));
        bar.refresh(&ed);
        assert_eq!((bar.hits.len(), bar.at), (3, Some(0)));
        // Next from nowhere is the first; then on, round the end.
        let picked = |ed: &Editor| ed.selection().map(|(a, b)| (a.para, a.byte, b.byte));
        bar.go(&mut ed, 1);
        assert_eq!(picked(&ed), Some((0, 2, 5)));
        bar.go(&mut ed, 1);
        bar.go(&mut ed, 1);
        assert_eq!(picked(&ed), Some((1, 6, 9)));
        bar.go(&mut ed, 1);
        assert_eq!((picked(&ed), bar.at), (Some((0, 2, 5)), Some(0)));
        bar.go(&mut ed, -1);
        assert_eq!(picked(&ed), Some((1, 6, 9)));
        // Capitals count when asked: two places left.
        bar.find.exact_case = true;
        bar.refresh(&ed);
        assert_eq!(bar.hits.len(), 2);
        // Replacing the one it is on goes to the next.
        bar.go(&mut ed, 1);
        bar.with = "dog".into();
        bar.replace(&mut ed);
        assert_eq!((ed.doc.text(), bar.hits.len()), ("a dog, a Cat\nand a cat".to_owned(), 1));
        assert_eq!(picked(&ed), Some((1, 6, 9)));
        bar.replace(&mut ed);
        assert_eq!((ed.doc.text(), bar.hits.len(), bar.at), ("a dog, a Cat\nand a dog".to_owned(), 0, None));
        // What was selected is looked for, unless it is more than a line.
        bar.show(true, Some("two\nlines".into()));
        assert_eq!((bar.find.text.as_str(), bar.replacing), ("cat", true));
        bar.close();
        assert!(!bar.open && bar.hits.is_empty());
    }
}

//! What an editor does to how things look, and with whole pieces of the
//! document: dressing runs and paragraphs, lists, cut and paste,
//! pictures, and replacing what was found.

use lntrn_image::Image;

use crate::doc::{Align, Find, List, MAX_LEVEL, Para, ParaAttrs, Placed, Pos, Style, TextAttrs};
use crate::editor::{Clip, Editor};

/// Something a run has or hasn't.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Toggle {
    pub fn get(self, a: &TextAttrs) -> bool {
        match self {
            Toggle::Bold => a.bold,
            Toggle::Italic => a.italic,
            Toggle::Underline => a.underline,
            Toggle::Strike => a.strike,
        }
    }

    fn set(self, a: &mut TextAttrs, on: bool) {
        match self {
            Toggle::Bold => a.bold = on,
            Toggle::Italic => a.italic = on,
            Toggle::Underline => a.underline = on,
            Toggle::Strike => a.strike = on,
        }
    }
}

impl Editor {
    /// What the selection has in common, or what typing would have: what
    /// the toolbar shows.
    pub fn format_state(&self) -> TextAttrs {
        match self.selection() {
            Some((a, b)) => self.doc.common(a, b),
            None => self.typing_attrs(),
        }
    }

    /// Change what the selection has; with nothing selected, what is
    /// typed next.
    pub fn format(&mut self, change: impl Fn(&mut TextAttrs)) {
        match self.selection() {
            Some((a, b)) => {
                self.step_back(false);
                self.doc.format(a, b, change);
            }
            None => {
                let mut attrs = self.typing_attrs();
                change(&mut attrs);
                self.pending = Some(attrs);
            }
        }
    }

    /// On if any of the selection is without it, off if all of it has it.
    pub fn toggle(&mut self, which: Toggle) {
        let on = !which.get(&self.format_state());
        self.format(|a| which.set(a, on));
    }

    /// The paragraphs the selection touches, or the caret's. One the
    /// selection only reaches the start of is not touched.
    pub fn paras(&self) -> (usize, usize) {
        match self.selection() {
            Some((a, b)) => (a.para, if b.byte == 0 && b.para > a.para { b.para - 1 } else { b.para }),
            None => (self.caret.para, self.caret.para),
        }
    }

    /// What the first of them has: what the toolbar shows.
    pub fn para_state(&self) -> ParaAttrs {
        self.doc.para(self.paras().0).attrs
    }

    pub fn set_paras(&mut self, change: impl Fn(&mut ParaAttrs)) {
        let (from, to) = self.paras();
        self.step_back(false);
        self.doc.set_paras(from, to, change);
    }

    pub fn set_style(&mut self, style: Style) {
        self.set_paras(|a| a.style = style);
    }

    pub fn set_align(&mut self, align: Align) {
        self.set_paras(|a| a.align = align);
    }

    /// Make the paragraphs items of a list of this kind; if they all are
    /// already, plain paragraphs again.
    pub fn set_list(&mut self, kind: List) {
        let (from, to) = self.paras();
        let all = (from..=to).all(|i| self.doc.para(i).attrs.list.same_kind(kind));
        self.set_paras(|a| {
            if all {
                (a.list, a.level) = (List::None, 0);
            } else if !a.list.same_kind(kind) {
                a.list = kind;
            }
        });
    }

    /// The list items a level in (`by` 1) or out (−1).
    pub fn nest(&mut self, by: i32) {
        self.set_paras(|a| {
            if a.list != List::None {
                a.level = (i32::from(a.level) + by).clamp(0, i32::from(MAX_LEVEL)) as u8;
            }
        });
    }

    /// Tick a box, or untick it.
    pub fn tick(&mut self, para: usize) {
        if let List::Check(on) = self.doc.para(para).attrs.list {
            self.step_back(false);
            self.doc.set_paras(para, para, |a| a.list = List::Check(!on));
        }
    }

    // ---- cut, copy, paste ----

    pub fn copy(&self) -> Option<Clip> {
        let (a, b) = self.selection()?;
        let paras = self.doc.slice(a, b);
        let pictures = paras.iter().filter_map(|p| p.picture).filter_map(|placed| self.doc.pictures.get(placed.id).map(|picture| (placed.id, picture.clone()))).collect();
        Some(Clip { text: self.doc.plain(a, b), paras, pictures })
    }

    pub fn cut(&mut self) -> Option<Clip> {
        let clip = self.copy()?;
        self.step_back(false);
        self.cut_selection();
        self.moved();
        Some(clip)
    }

    /// Put back what one of our documents had, as it was.
    pub fn paste(&mut self, clip: &Clip) {
        self.step_back(false);
        self.cut_selection();
        for (id, picture) in &clip.pictures {
            self.doc.pictures.share(*id, picture);
        }
        self.caret = self.doc.insert_paras(self.caret, &clip.paras);
        self.moved();
    }

    /// Put in text from anywhere, dressed as what it lands in.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let attrs = self.typing_attrs();
        self.step_back(false);
        self.cut_selection();
        self.caret = self.doc.insert_text(self.caret, text, attrs);
        self.moved();
    }

    // ---- pictures ----

    fn place_picture(&mut self, id: u64) {
        self.step_back(false);
        self.cut_selection();
        self.caret = self.doc.insert_paras(self.caret, &[Para::of_picture(Placed { id, width: 0.0 })]);
        self.moved();
    }

    /// Put a picture file in at the caret, as a paragraph of its own.
    pub fn insert_picture(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let id = self.doc.pictures.add(bytes)?;
        self.place_picture(id);
        Ok(())
    }

    /// The same for pixels that came as no file.
    pub fn insert_image(&mut self, image: &Image) -> Result<(), String> {
        let id = self.doc.pictures.add_image(image)?;
        self.place_picture(id);
        Ok(())
    }

    /// Show a picture `width` logical pixels wide. A drag calls this as
    /// it goes: only its `first` call is an undo step.
    pub fn resize_picture(&mut self, para: usize, width: f32, first: bool) {
        let Some(placed) = self.doc.para(para).picture else { return };
        if first {
            self.step_back(false);
        } else {
            self.rev += 1;
        }
        self.doc.para_mut(para).picture = Some(Placed { width: width.max(16.0), ..placed });
    }

    /// Show a picture this wide, or at its own width.
    pub fn picture_width(&mut self, para: usize, width: Option<f32>) {
        let Some(placed) = self.doc.para(para).picture else { return };
        self.step_back(false);
        self.doc.para_mut(para).picture = Some(Placed { width: width.unwrap_or(0.0), ..placed });
    }

    // ---- replacing ----

    /// Put `with` where `a..b` is, dressed as what it replaces.
    pub fn replace(&mut self, a: Pos, b: Pos, with: &str) {
        self.step_back(false);
        let attrs = self.doc.typing_attrs(self.doc.right(a.min(b)));
        let at = self.doc.delete(a, b);
        self.caret = self.doc.insert_text(at, with, attrs);
        self.anchor = None;
        self.moved();
    }

    /// Replace every place `find` is. Returns how many there were.
    pub fn replace_all(&mut self, find: &Find, with: &str) -> usize {
        let hits = find.all(&self.doc);
        if hits.is_empty() {
            return 0;
        }
        self.step_back(false);
        // Last first: a replacement moves only what is after it.
        for (a, b) in hits.iter().rev() {
            let attrs = self.doc.typing_attrs(self.doc.right(*a));
            let at = self.doc.delete(*a, *b);
            self.doc.insert_text(at, with, attrs);
        }
        (self.caret, self.anchor) = (self.doc.clamp(self.caret), None);
        self.moved();
        hits.len()
    }
}

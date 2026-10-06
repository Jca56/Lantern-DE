//! Setting a document on the page: each paragraph broken into rows of
//! pieces (a piece is a stretch of one look on one row), every piece
//! knowing where each of its characters starts. From that come where the
//! caret is, what is under the pointer, and what to draw.
//!
//! Everything is in pixels, x from the left of the text column and y
//! from the top of the document. A paragraph's layout is kept for as long
//! as the paragraph is the same one and the page is set the same way, so
//! typing lays out the paragraph typed in and nothing else.

use std::collections::HashMap;
use std::sync::Arc;

use lntrn_text::{TextEngine, TextStyle};

use crate::doc::{Align, Doc, List, Para, ParaAttrs, Pos, Style, TextAttrs};

/// How far in each level of a list is, and where a quote starts, in
/// logical pixels.
pub const LIST_STEP: f32 = 30.0;
pub const QUOTE_INDENT: f32 = 22.0;
/// A tab, in spaces of body text.
const TAB_SPACES: f32 = 4.0;

/// What the layout depends on besides the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    /// The text column's width, in pixels.
    pub width: f32,
    /// Pixels to a logical pixel.
    pub scale: f32,
    /// Body text's size, in logical pixels.
    pub body: f32,
}

/// How a run is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub style: TextStyle,
    pub attrs: TextAttrs,
    pub ascent: f32,
    pub line: f32,
}

impl Look {
    pub fn of(text: &mut TextEngine, para: &ParaAttrs, attrs: TextAttrs, s: &Setting) -> Look {
        let mut style = TextStyle::new(attrs.size.unwrap_or(s.body * para.style.scale()) * s.scale);
        if para.style.bold() || attrs.bold {
            style = style.bold();
        }
        if para.style.italic() || attrs.italic {
            style = style.italic();
        }
        if let Some(font) = attrs.font {
            style = style.family(font.name());
        }
        Look { ascent: text.ascent(&style), line: style.line_height(), style, attrs }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Bytes of the paragraph's text.
    pub start: usize,
    pub end: usize,
    /// Its left edge in the column, and its width.
    pub x: f32,
    pub width: f32,
    /// Which of the paragraph's looks.
    pub look: usize,
    /// A tab: room, with nothing to draw.
    pub blank: bool,
    /// Where each character of it starts, and where the last ends:
    /// `(byte of the paragraph, x from the piece's left)`.
    pub edges: Vec<(usize, f32)>,
}

impl Piece {
    /// The x in the column of the edge before `byte`.
    fn x_of(&self, byte: usize) -> f32 {
        self.x + self.edges.iter().rev().find(|(b, _)| *b <= byte).map_or(0.0, |(_, x)| *x)
    }

    /// The edge nearest to `x` in the column.
    fn byte_at(&self, x: f32) -> usize {
        let x = x - self.x;
        self.edges.iter().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map_or(self.start, |(b, _)| *b)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    /// From the paragraph's top.
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// Where its text starts and how wide it is, blanks at its end left
    /// out.
    pub x: f32,
    pub width: f32,
    pub pieces: Vec<Piece>,
}

impl Row {
    pub fn x_of(&self, byte: usize) -> f32 {
        match self.pieces.iter().rev().find(|p| p.start <= byte) {
            Some(p) => p.x_of(byte.min(p.end)),
            None => self.x,
        }
    }

    pub fn byte_at(&self, x: f32) -> usize {
        match self.pieces.iter().find(|p| x < p.x + p.width).or(self.pieces.last()) {
            Some(p) => p.byte_at(x),
            None => self.start,
        }
    }

    /// The last place on the row the caret rests: before the blank a
    /// wrapped row ends with, which belongs to the wrap.
    pub fn rest(&self, text: &str, last: bool) -> usize {
        if !last && text[self.start..self.end].ends_with(' ') { self.end - 1 } else { self.end }
    }
}

/// A paragraph set in rows.
#[derive(Clone, Debug)]
pub struct Set {
    /// The paragraph it was set from: while the document still holds
    /// this very one, the layout stands.
    pub src: Arc<Para>,
    pub looks: Vec<Look>,
    pub rows: Vec<Row>,
    /// With the room over and under it.
    pub height: f32,
    /// Where its text column starts: past a list's marker, a quote's bar.
    pub indent: f32,
    /// A picture's size as shown, when it is one.
    pub picture: Option<(f32, f32)>,
}

struct Cluster {
    start: usize,
    end: usize,
    width: f32,
    look: usize,
    space: bool,
    tab: bool,
    /// A row may end after it.
    breaks: bool,
}

impl Set {
    pub fn new(text: &mut TextEngine, src: &Arc<Para>, doc: &Doc, s: &Setting) -> Set {
        let attrs = src.attrs;
        let base = Look::of(text, &attrs, src.spans.at(0), s);
        let (over, under) = attrs.style.space();
        let (before, after) = (over * base.style.size + attrs.space_before * s.scale, under * base.style.size + attrs.space_after * s.scale);
        let indent = match (attrs.list, attrs.style) {
            (List::None, Style::Quote) => QUOTE_INDENT * s.scale,
            (List::None, _) => 0.0,
            _ => (f32::from(attrs.level) + 1.0) * LIST_STEP * s.scale,
        };
        let room = (s.width - indent).max(base.style.size * 2.0);
        if let Some(placed) = src.picture {
            // As wide as asked, or as it is, and never wider than the
            // column.
            let (pw, ph) = doc.pictures.get(placed.id).map_or((64.0, 64.0), |p| (p.image.width as f32, p.image.height as f32));
            let wide = (if placed.width > 0.0 { placed.width } else { pw } * s.scale).clamp(8.0, room);
            let tall = (wide * ph / pw.max(1.0)).max(1.0);
            let x = indent + align_shift(attrs.align, room - wide);
            let piece = Piece { start: 0, end: src.text.len(), x, width: wide, look: 0, blank: true, edges: vec![(0, 0.0), (src.text.len(), wide)] };
            let row = Row { start: 0, end: src.text.len(), top: before, height: tall, baseline: tall, x, width: wide, pieces: vec![piece] };
            return Set { src: src.clone(), looks: vec![base], rows: vec![row], height: before + tall + after, indent, picture: Some((wide, tall)) };
        }
        // ---- the runs as looks, and the text as clusters ----
        let mut looks: Vec<Look> = Vec::new();
        let mut clusters: Vec<Cluster> = Vec::new();
        let mut edges = Vec::new();
        let tab = text.measure(" ", &TextStyle::new(s.body * s.scale)) * TAB_SPACES;
        for run in src.spans.runs(0, src.text.len()) {
            let look = Look::of(text, &attrs, run.attrs, s);
            let li = looks.iter().position(|l| *l == look).unwrap_or_else(|| {
                looks.push(look.clone());
                looks.len() - 1
            });
            let run_text = &src.text[run.start..run.end];
            text.advances(run_text, &look.style, &mut edges);
            for pair in edges.windows(2) {
                let (a, b) = (run.start + pair[0].0 as usize, run.start + pair[1].0 as usize);
                let piece = &src.text[a..b];
                let (space, is_tab) = (piece.chars().all(char::is_whitespace), piece == "\t");
                clusters.push(Cluster { start: a, end: b, width: if is_tab { tab } else { pair[1].1 - pair[0].1 }, look: li, space, tab: is_tab, breaks: space || piece.ends_with('-') });
            }
        }
        if looks.is_empty() {
            looks.push(base.clone());
        }
        // ---- rows: as many clusters as fit, parted where a word ends ----
        let mut cuts: Vec<usize> = Vec::new();
        let (mut from, mut x, mut last_break) = (0, 0.0_f32, None::<usize>);
        let avail = |row: usize| if row == 0 { (room - attrs.first_indent * s.scale).max(base.style.size * 2.0) } else { room };
        for (i, c) in clusters.iter().enumerate() {
            if x + c.width > avail(cuts.len()) && !c.space && i > from {
                let cut = last_break.filter(|b| *b > from).unwrap_or(i);
                cuts.push(cut);
                x = clusters[cut..i].iter().map(|c| c.width).sum();
                (from, last_break) = (cut, None);
            }
            x += c.width;
            if c.breaks {
                last_break = Some(i + 1);
            }
        }
        cuts.push(clusters.len());
        // ---- each row's pieces, set where the paragraph is aligned to ----
        let mut rows: Vec<Row> = Vec::with_capacity(cuts.len());
        let (mut top, mut from) = (before, 0);
        for (r, &to) in cuts.iter().enumerate() {
            let row = &clusters[from..to];
            let shown = row.len() - row.iter().rev().take_while(|c| c.space).count();
            let width: f32 = row[..shown].iter().map(|c| c.width).sum();
            let first = if r == 0 { attrs.first_indent * s.scale } else { 0.0 };
            let spare = (avail(r) - width).max(0.0);
            let gaps = row[..shown].iter().filter(|c| c.space).count();
            // A justified row's blanks take up its slack; a paragraph's
            // last row, and one with no blanks, is left as it falls.
            let stretch = if attrs.align == Align::Justify && r + 1 < cuts.len() && gaps > 0 { spare / gaps as f32 } else { 0.0 };
            let x0 = indent + first + if stretch > 0.0 { 0.0 } else { align_shift(attrs.align, spare) };
            let mut pieces: Vec<Piece> = Vec::new();
            let mut pen = x0;
            for (k, c) in row.iter().enumerate() {
                let wide = c.width + if c.space && k < shown { stretch } else { 0.0 };
                // A stretched blank, a tab and a change of look each end
                // a piece: what is after starts where the pen is.
                match pieces.last_mut().filter(|p| p.look == c.look && !p.blank && !c.tab && p.end == c.start && !(stretch > 0.0 && (c.space || src.text[..c.start].ends_with(char::is_whitespace)))) {
                    Some(p) => {
                        p.width += wide;
                        p.end = c.end;
                        p.edges.push((c.end, p.width));
                    }
                    None => pieces.push(Piece { start: c.start, end: c.end, x: pen, width: wide, look: c.look, blank: c.tab, edges: vec![(c.start, 0.0), (c.end, wide)] }),
                }
                pen += wide;
            }
            let used = |pick: fn(&Look) -> f32| pieces.iter().map(|p| pick(&looks[p.look])).fold(0.0, f32::max);
            let (height, baseline) = if pieces.is_empty() { (looks[0].line, looks[0].ascent) } else { (used(|l| l.line), used(|l| l.ascent)) };
            let (start, end) = (row.first().map_or(0, |c| c.start), row.last().map_or(0, |c| c.end));
            rows.push(Row { start, end, top, height, baseline, x: x0, width: width + stretch * gaps as f32, pieces });
            top += height;
            from = to;
        }
        Set { src: src.clone(), looks, rows, height: top + after, indent, picture: None }
    }

    /// The row a place is on: the one it is the start of, when it is
    /// where a row wrapped.
    pub fn row_of(&self, byte: usize) -> usize {
        self.rows.iter().position(|r| byte < r.end).unwrap_or(self.rows.len() - 1)
    }

    pub fn row_at(&self, y: f32) -> usize {
        self.rows.iter().position(|r| y < r.top + r.height).unwrap_or(self.rows.len() - 1)
    }
}

/// How far right a row with `spare` room to it starts.
fn align_shift(align: Align, spare: f32) -> f32 {
    match align {
        Align::Left | Align::Justify => 0.0,
        Align::Center => (spare * 0.5).floor(),
        Align::Right => spare,
    }
}

/// A whole document set.
#[derive(Default)]
pub struct Layout {
    pub sets: Vec<Set>,
    /// Each paragraph's top, and after the last the document's height.
    pub tops: Vec<f32>,
    setting: Option<Setting>,
}

impl Layout {
    pub fn height(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0)
    }

    /// Bring it in step with `doc` set as `s` says. Returns whether
    /// anything was set anew.
    pub fn sync(&mut self, text: &mut TextEngine, doc: &Doc, s: Setting) -> bool {
        let same_setting = self.setting == Some(s);
        let fresh = |old: &Set, para: &Arc<Para>| same_setting && Arc::ptr_eq(&old.src, para);
        if self.sets.len() == doc.paras.len() && self.sets.iter().zip(&doc.paras).all(|(old, para)| fresh(old, para)) {
            return false;
        }
        // Paragraphs move when one is added or taken away: each is found
        // again by being the same paragraph, wherever it is now.
        let mut old: HashMap<*const Para, Set> = if same_setting { std::mem::take(&mut self.sets).into_iter().map(|set| (Arc::as_ptr(&set.src), set)).collect() } else { HashMap::new() };
        self.sets = doc.paras.iter().map(|para| old.remove(&Arc::as_ptr(para)).unwrap_or_else(|| Set::new(text, para, doc, &s))).collect();
        self.tops.clear();
        let mut y = 0.0;
        for set in &self.sets {
            self.tops.push(y);
            y += set.height;
        }
        self.tops.push(y);
        self.setting = Some(s);
        true
    }

    fn set(&self, para: usize) -> (&Set, f32) {
        let i = para.min(self.sets.len() - 1);
        (&self.sets[i], self.tops[i])
    }

    /// Where the caret stands at a place: its x, its top and its height.
    pub fn caret(&self, p: Pos) -> (f32, f32, f32) {
        let (set, top) = self.set(p.para);
        let row = &set.rows[set.row_of(p.byte)];
        (row.x_of(p.byte), top + row.top, row.height)
    }

    pub fn para_at(&self, y: f32) -> usize {
        self.tops.partition_point(|top| *top <= y).saturating_sub(1).min(self.sets.len() - 1)
    }

    /// The place nearest a point.
    pub fn hit(&self, x: f32, y: f32) -> Pos {
        let para = self.para_at(y);
        let (set, top) = self.set(para);
        let r = set.row_at(y - top);
        let row = &set.rows[r];
        Pos::new(para, row.byte_at(x).min(row.rest(&set.src.text, r + 1 == set.rows.len())))
    }

    /// The place a row up (`by` −1) or down (+1) from one, as near `x`
    /// as that row gets. `None` at the document's top or bottom.
    pub fn step(&self, p: Pos, by: i32, x: f32) -> Option<Pos> {
        let (set, _) = self.set(p.para);
        let r = set.row_of(p.byte) as i32 + by;
        let (para, r) = if r < 0 {
            let para = p.para.checked_sub(1)?;
            (para, self.sets[para].rows.len() - 1)
        } else if r as usize >= set.rows.len() {
            (Some(p.para + 1).filter(|n| *n < self.sets.len())?, 0)
        } else {
            (p.para, r as usize)
        };
        let set = &self.sets[para];
        let row = &set.rows[r];
        Some(Pos::new(para, row.byte_at(x).min(row.rest(&set.src.text, r + 1 == set.rows.len()))))
    }

    /// The start and the resting end of the row a place is on.
    pub fn row_ends(&self, p: Pos) -> (Pos, Pos) {
        let (set, _) = self.set(p.para);
        let r = set.row_of(p.byte);
        let row = &set.rows[r];
        (Pos::new(p.para, row.start), Pos::new(p.para, row.rest(&set.src.text, r + 1 == set.rows.len())))
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;

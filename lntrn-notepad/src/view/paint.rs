//! What a paragraph puts on the page, as marks: a stretch of text, a
//! rectangle, a line, a picture. The screen draws them, and so does the
//! PDF writer, so a document printed is the document seen.
//!
//! A mark's place is in pixels from the text column's left and the
//! document's top, like the layout it comes from.

use lntrn_text::{TextEngine, TextStyle};

use super::layout::{LIST_STEP, QUOTE_INDENT, Set, Setting};
use crate::doc::{Doc, List, Style};

/// The colours of a page, 0xRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inks {
    pub page: u32,
    pub text: u32,
    /// Quotes, list marks.
    pub dim: u32,
    pub accent: u32,
}

impl Inks {
    pub const PAPER: Inks = Inks { page: 0xFCFBF9, text: 0x1C1B18, dim: 0x706C64, accent: 0xC89600 };
    pub const DARK: Inks = Inks { page: 0x1D1914, text: 0xE8DCC8, dim: 0xA99C86, accent: 0xFAC800 };
    /// Ink on paper, whatever the screen shows.
    pub const PRINT: Inks = Inks { page: 0xFFFFFF, text: 0x000000, dim: 0x555555, accent: 0x333333 };
}

pub enum Mark<'a> {
    Rect { x: f32, y: f32, w: f32, h: f32, color: u32 },
    Line { x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: u32 },
    /// `ascent` is how far over the baseline its line's top is.
    Text { text: &'a str, style: &'a TextStyle, x: f32, baseline: f32, ascent: f32, color: u32 },
    Picture { id: u64, x: f32, y: f32, w: f32, h: f32 },
}

/// The ink for text on a highlight: the page's own where that can be
/// read there, else the dark or the light one, whichever can. (A dark
/// page's pale ink is lost on a yellow highlighter.)
fn ink_on(ground: u32, ink: u32) -> u32 {
    let light = |c: u32| 0.299 * f32::from((c >> 16) as u8) + 0.587 * f32::from((c >> 8) as u8) + 0.114 * f32::from(c as u8);
    if (light(ground) - light(ink)).abs() >= 100.0 {
        ink
    } else if light(ground) > 140.0 {
        Inks::PAPER.text
    } else {
        Inks::DARK.text
    }
}

/// Where a list item's box to tick is: its left, top and side.
pub fn check_box(set: &Set, s: &Setting) -> (f32, f32, f32) {
    let row = &set.rows[0];
    let side = (set.looks[0].style.size * 0.72).round();
    (set.indent - LIST_STEP * s.scale * 0.9, row.top + ((row.height - side) * 0.5).round(), side)
}

/// Everything paragraph `i` of `doc` shows, its top at `top`. Each mark
/// comes with the text engine, for the one who draws it.
#[allow(clippy::too_many_arguments)] // a paragraph, where it is, how it is set and inked, and who is told
pub fn marks(text: &mut TextEngine, set: &Set, doc: &Doc, i: usize, top: f32, s: &Setting, inks: &Inks, emit: &mut dyn FnMut(&mut TextEngine, Mark<'_>)) {
    let attrs = set.src.attrs;
    let first = &set.rows[0];
    if let (Some(placed), Some((w, h))) = (set.src.picture, set.picture) {
        return emit(text, Mark::Picture { id: placed.id, x: first.x, y: top + first.top, w, h });
    }
    let base = &set.looks[0];
    let thin = (s.scale * 1.6).max(1.0);
    if attrs.style == Style::Quote && attrs.list == List::None {
        let last = &set.rows[set.rows.len() - 1];
        emit(text, Mark::Rect { x: set.indent - QUOTE_INDENT * s.scale, y: top + first.top, w: (3.0 * s.scale).round(), h: last.top + last.height - first.top, color: inks.accent });
    }
    // ---- a list item's mark, in the room before its text ----
    let baseline = top + first.top + first.baseline;
    match attrs.list {
        List::None => {}
        List::Bullet => emit(text, Mark::Text { text: "\u{2022}", style: &base.style, x: set.indent - LIST_STEP * s.scale * 0.72, baseline, ascent: base.ascent, color: inks.dim }),
        List::Number => {
            let label = format!("{}.", doc.number(i));
            let x = set.indent - text.measure(&label, &base.style) - base.style.size * 0.35;
            emit(text, Mark::Text { text: &label, style: &base.style, x, baseline, ascent: base.ascent, color: inks.dim });
        }
        List::Check(on) => {
            let (x, y, side) = check_box(set, s);
            let y = top + y;
            if on {
                emit(text, Mark::Rect { x, y, w: side, h: side, color: inks.accent });
                let tick = [(0.22, 0.52), (0.42, 0.72), (0.78, 0.28)].map(|(u, v)| (x + side * u, y + side * v));
                for pair in tick.windows(2) {
                    emit(text, Mark::Line { x0: pair[0].0, y0: pair[0].1, x1: pair[1].0, y1: pair[1].1, width: thin * 1.4, color: inks.page });
                }
            } else {
                for (a, b) in [((0.0, 0.0), (1.0, 0.0)), ((1.0, 0.0), (1.0, 1.0)), ((1.0, 1.0), (0.0, 1.0)), ((0.0, 1.0), (0.0, 0.0))] {
                    emit(text, Mark::Line { x0: x + side * a.0, y0: y + side * a.1, x1: x + side * b.0, y1: y + side * b.1, width: thin, color: inks.dim });
                }
            }
        }
    }
    // ---- the text: what is behind it, then it, then what is ruled on it ----
    let ticked = attrs.list == List::Check(true);
    for row in &set.rows {
        let baseline = top + row.top + row.baseline;
        for piece in row.pieces.iter().filter(|p| !p.blank) {
            let look = &set.looks[piece.look];
            if let Some(color) = look.attrs.highlight {
                emit(text, Mark::Rect { x: piece.x, y: top + row.top, w: piece.width, h: row.height, color });
            }
            let own = if attrs.style == Style::Quote || ticked { inks.dim } else { inks.text };
            let color = look.attrs.color.unwrap_or_else(|| look.attrs.highlight.map_or(own, |ground| ink_on(ground, own)));
            emit(text, Mark::Text { text: &set.src.text[piece.start..piece.end], style: &look.style, x: piece.x, baseline, ascent: look.ascent, color });
            let rule = (look.style.size / 14.0).max(1.0);
            if look.attrs.underline {
                let y = baseline + look.style.size * 0.12;
                emit(text, Mark::Line { x0: piece.x, y0: y, x1: piece.x + piece.width, y1: y, width: rule, color });
            }
            if look.attrs.strike || ticked {
                let y = baseline - look.style.size * 0.28;
                emit(text, Mark::Line { x0: piece.x, y0: y, x1: piece.x + piece.width, y1: y, width: rule, color });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Pos, TextAttrs};
    use crate::view::layout::Layout;

    fn told(mark: &Mark) -> String {
        match mark {
            Mark::Rect { x, w, color, .. } => format!("rect {x:.0}+{w:.0} {color:06x}"),
            Mark::Line { color, .. } => format!("line {color:06x}"),
            Mark::Text { text, color, x, .. } => format!("text {text:?} at {x:.0} {color:06x}"),
            Mark::Picture { w, h, .. } => format!("picture {w:.0}x{h:.0}"),
        }
    }

    #[test]
    fn a_paragraph_is_told_as_marks() {
        let mut doc = Doc::from_text("plain marked\nfirst\nsecond\ndone\nsaid");
        doc.format(Pos::new(0, 6), Pos::new(0, 12), |a| *a = TextAttrs { underline: true, strike: true, color: Some(0xcc3a34), highlight: Some(0xfff0c0), ..TextAttrs::default() });
        doc.set_paras(1, 2, |a| a.list = List::Number);
        doc.set_paras(3, 3, |a| a.list = List::Check(true));
        doc.set_paras(4, 4, |a| a.style = Style::Quote);
        let id = doc.pictures.add(lntrn_image::encode_png(&lntrn_image::Image::solid(40, 20, [1, 2, 3, 255]))).unwrap();
        doc.insert_paras(doc.end(), &[crate::doc::Para::of_picture(crate::doc::Placed { id, width: 0.0 })]);
        let mut text = TextEngine::new("Inter", "JetBrains Mono");
        let s = Setting { width: 600.0, scale: 1.0, body: 20.0 };
        let mut layout = Layout::default();
        layout.sync(&mut text, &doc, s);
        let of = |text: &mut TextEngine, i: usize| {
            let mut out = Vec::new();
            marks(text, &layout.sets[i], &doc, i, layout.tops[i], &s, &Inks::PAPER, &mut |_, m| out.push(told(&m)));
            out
        };
        let dressed = of(&mut text, 0);
        assert_eq!(dressed[0], "text \"plain \" at 0 1c1b18");
        assert!(dressed[1].starts_with("rect ") && dressed[1].ends_with(" fff0c0"), "the highlight is behind its text: {dressed:?}");
        assert_eq!((dressed[2].starts_with("text \"marked\""), &dressed[3..]), (true, &["line cc3a34".to_owned(), "line cc3a34".to_owned()][..]));
        // A list's numbers count on; a ticked box is filled, ticked and
        // its text struck out in the quiet ink; a quote has its bar.
        assert!(of(&mut text, 1)[0].starts_with("text \"1.\"") && of(&mut text, 2)[0].starts_with("text \"2.\""));
        let done = of(&mut text, 3);
        assert_eq!((done[0].ends_with("c89600"), done[1].as_str(), done[2].as_str(), done[3].as_str(), done[4].as_str()), (true, "line fcfbf9", "line fcfbf9", "text \"done\" at 30 706c64", "line 706c64"));
        let said = of(&mut text, 4);
        assert_eq!((said[0].as_str(), said[1].as_str()), ("rect 0+3 c89600", "text \"said\" at 22 706c64"));
        assert_eq!(of(&mut text, 5), ["picture 40x20"]);
        // On the dark page, highlighted text takes an ink that shows
        // on its highlight; text in a colour of its own keeps it.
        let mut dark = Doc::from_text("pale bright");
        dark.format(Pos::new(0, 0), Pos::new(0, 4), |a| a.highlight = Some(0xfff0a0));
        dark.format(Pos::new(0, 5), Pos::new(0, 11), |a| (a.highlight, a.color) = (Some(0xfff0a0), Some(0x2563eb)));
        let mut on_dark = Layout::default();
        on_dark.sync(&mut text, &dark, s);
        let mut out = Vec::new();
        marks(&mut text, &on_dark.sets[0], &dark, 0, 0.0, &s, &Inks::DARK, &mut |_, m| out.push(told(&m)));
        let inked: Vec<&str> = out.iter().filter(|m| m.starts_with("text")).map(|m| &m[m.len() - 6..]).collect();
        assert_eq!(inked, ["1c1b18", "e8dcc8", "2563eb"], "{out:?}");
        assert_eq!((ink_on(0xfff0a0, Inks::PAPER.text), ink_on(0x203040, Inks::PAPER.text)), (Inks::PAPER.text, Inks::DARK.text));
        // The box is where a click on it is looked for.
        let (x, y, side) = check_box(&layout.sets[3], &s);
        assert!(x >= 0.0 && x + side <= layout.sets[3].indent && y >= 0.0 && side >= 12.0);
    }
}

//! The document: paragraphs of text, each with the runs it has dressed
//! and what the paragraph itself has, and the pictures it shows. Nothing
//! here knows of windows or files.
//!
//! - `attrs`: what runs and paragraphs can have, and the named styles.
//! - `spans`: a paragraph's runs.
//! - `edit`: typing, cutting, parting and joining, formatting.
//! - `nav`: moving through the text by character and word, and finding.
//! - `pictures`: the pictures a document holds.
//!
//! A paragraph is behind an [`Arc`]: a copy of a document shares all of
//! them, and only the ones edited afterwards are made anew. That is what
//! makes an undo step, and a copy handed to the thread that saves, cost
//! next to nothing.

pub mod attrs;
mod edit;
mod nav;
pub mod pictures;
pub mod spans;

use std::sync::Arc;

pub use attrs::{Align, Font, List, MAX_LEVEL, ParaAttrs, Style, TextAttrs};
pub use nav::Find;
pub use pictures::{Picture, Pictures};
pub use spans::Spans;

/// What a picture's paragraph holds for text: the one character that
/// stands for an object.
pub const OBJECT: &str = "\u{FFFC}";

/// A picture as a paragraph shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Which of the document's pictures.
    pub id: u64,
    /// How wide it shows, in logical pixels; zero for its own width. The
    /// page's width is the most it gets either way.
    pub width: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Para {
    pub text: String,
    pub spans: Spans,
    pub attrs: ParaAttrs,
    /// A paragraph that is a picture: its text is [`OBJECT`] and nothing
    /// else, so the caret can be before it or after it.
    pub picture: Option<Placed>,
}

impl Para {
    pub fn plain(text: &str) -> Para {
        Para { text: text.to_owned(), ..Para::default() }
    }

    pub fn of_picture(placed: Placed) -> Para {
        Para { text: OBJECT.to_owned(), picture: Some(placed), ..Para::default() }
    }

    pub fn is_picture(&self) -> bool {
        self.picture.is_some()
    }

    /// A picture's paragraph that lost its one character is a picture no
    /// more.
    fn settle(&mut self) {
        if self.picture.is_some() && self.text != OBJECT {
            self.picture = None;
            self.text.retain(|c| c != '\u{FFFC}');
        }
    }
}

/// A place in the text: before byte `byte` of paragraph `para`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub para: usize,
    pub byte: usize,
}

impl Pos {
    pub fn new(para: usize, byte: usize) -> Pos {
        Pos { para, byte }
    }
}

/// Never without a paragraph: an empty document is one empty paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub paras: Vec<Arc<Para>>,
    pub pictures: Pictures,
}

impl Default for Doc {
    fn default() -> Self {
        Doc { paras: vec![Arc::new(Para::default())], pictures: Pictures::default() }
    }
}

impl Doc {
    pub fn from_paras(paras: Vec<Para>) -> Doc {
        let mut doc = Doc { paras: paras.into_iter().map(Arc::new).collect(), pictures: Pictures::default() };
        if doc.paras.is_empty() {
            doc.paras.push(Arc::new(Para::default()));
        }
        doc
    }

    /// A document of plain paragraphs, one per line of `text`.
    pub fn from_text(text: &str) -> Doc {
        Doc::from_paras(text.split('\n').map(|line| Para::plain(line.strip_suffix('\r').unwrap_or(line))).collect())
    }

    pub fn para(&self, i: usize) -> &Para {
        &self.paras[i.min(self.paras.len() - 1)]
    }

    /// Paragraph `i` to change: its own copy, if it was shared.
    pub(crate) fn para_mut(&mut self, i: usize) -> &mut Para {
        Arc::make_mut(&mut self.paras[i])
    }

    pub fn end(&self) -> Pos {
        Pos::new(self.paras.len() - 1, self.para(self.paras.len() - 1).text.len())
    }

    /// The nearest place there is to `p`: in a paragraph the document
    /// has, on a character's edge.
    pub fn clamp(&self, p: Pos) -> Pos {
        let para = p.para.min(self.paras.len() - 1);
        let text = &self.para(para).text;
        let mut byte = if p.para > para { text.len() } else { p.byte.min(text.len()) };
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        Pos::new(para, byte)
    }

    /// The text between two places, paragraphs parted by newlines, a
    /// picture left out.
    pub fn plain(&self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        for i in a.para..=b.para {
            let text = &self.para(i).text;
            let (from, to) = (if i == a.para { a.byte } else { 0 }, if i == b.para { b.byte } else { text.len() });
            if i > a.para {
                out.push('\n');
            }
            out.extend(text[from..to].chars().filter(|c| *c != '\u{FFFC}'));
        }
        out
    }

    /// All of it as text.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.plain(Pos::default(), self.end())
    }

    /// Whether there is nothing in it at all.
    pub fn is_blank(&self) -> bool {
        self.paras.len() == 1 && self.para(0).text.is_empty()
    }

    /// The number a numbered item shows: its place among the items of its
    /// list, which runs back until something that is not of it.
    pub fn number(&self, i: usize) -> u32 {
        let me = self.para(i).attrs;
        let mut n = 1;
        for p in self.paras[..i.min(self.paras.len())].iter().rev() {
            if p.attrs.list == List::None || p.attrs.level < me.level {
                break;
            }
            if p.attrs.level == me.level {
                if p.attrs.list != List::Number {
                    break;
                }
                n += 1;
            }
        }
        n
    }

    /// Words and characters in it, for the status line.
    pub fn count(&self) -> (usize, usize) {
        let mut out = (0, 0);
        for p in &self.paras {
            out.0 += p.text.split_whitespace().filter(|w| *w != OBJECT).count();
            out.1 += p.text.chars().filter(|c| *c != '\u{FFFC}').count();
        }
        out
    }
}

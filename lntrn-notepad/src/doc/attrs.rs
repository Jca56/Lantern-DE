//! How text and paragraphs are dressed: what a run of characters has of
//! its own, what a paragraph has, and the named styles (a heading, a
//! quote) that give a paragraph its look in one word.

use std::sync::Mutex;

/// A font family by name, kept as a number so a run's attributes stay a
/// plain value. Names are kept for the life of the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Font(u16);

static FAMILIES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

impl Font {
    /// The family called `name`.
    pub fn named(name: &str) -> Font {
        let mut families = FAMILIES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = families.iter().position(|f| *f == name) {
            return Font(i as u16);
        }
        families.push(Box::leak(name.to_owned().into_boxed_str()));
        Font((families.len() - 1) as u16)
    }

    pub fn name(self) -> &'static str {
        FAMILIES.lock().unwrap_or_else(|e| e.into_inner()).get(self.0 as usize).copied().unwrap_or("")
    }
}

/// What a run of text has of its own, over what its paragraph's style
/// gives it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Size in logical pixels; the style's when `None`.
    pub size: Option<f32>,
    /// The default family when `None`.
    pub font: Option<Font>,
    /// 0xRRGGBB; the page's ink when `None`.
    pub color: Option<u32>,
    /// A highlighter behind the text, 0xRRGGBB.
    pub highlight: Option<u32>,
}

impl TextAttrs {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// What two runs have in common: an on/off attribute is on only if
    /// it is on in both, a value is kept only if both have the same.
    pub fn common(self, other: TextAttrs) -> TextAttrs {
        fn same<T: PartialEq>(a: Option<T>, b: Option<T>) -> Option<T> {
            if a == b { a } else { None }
        }
        TextAttrs { bold: self.bold && other.bold, italic: self.italic && other.italic, underline: self.underline && other.underline, strike: self.strike && other.strike, size: same(self.size, other.size), font: same(self.font, other.font), color: same(self.color, other.color), highlight: same(self.highlight, other.highlight) }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// A paragraph's named look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Body,
    Title,
    Heading1,
    Heading2,
    Heading3,
    Quote,
}

impl Style {
    pub const ALL: [Style; 6] = [Style::Body, Style::Title, Style::Heading1, Style::Heading2, Style::Heading3, Style::Quote];

    pub fn label(self) -> &'static str {
        match self {
            Style::Body => "Body",
            Style::Title => "Title",
            Style::Heading1 => "Heading 1",
            Style::Heading2 => "Heading 2",
            Style::Heading3 => "Heading 3",
            Style::Quote => "Quote",
        }
    }

    /// The word a file knows it by.
    pub fn word(self) -> &'static str {
        match self {
            Style::Body => "body",
            Style::Title => "title",
            Style::Heading1 => "h1",
            Style::Heading2 => "h2",
            Style::Heading3 => "h3",
            Style::Quote => "quote",
        }
    }

    pub fn from_word(word: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|s| s.word() == word)
    }

    /// Its text's size, as a multiple of body text.
    pub fn scale(self) -> f32 {
        match self {
            Style::Title => 2.0,
            Style::Heading1 => 1.6,
            Style::Heading2 => 1.3,
            Style::Heading3 => 1.12,
            Style::Body | Style::Quote => 1.0,
        }
    }

    pub fn bold(self) -> bool {
        matches!(self, Style::Title | Style::Heading1 | Style::Heading2 | Style::Heading3)
    }

    pub fn italic(self) -> bool {
        self == Style::Quote
    }

    /// Room over and under a paragraph of it, as multiples of its text
    /// size.
    pub fn space(self) -> (f32, f32) {
        match self {
            Style::Title => (0.2, 0.35),
            Style::Heading1 => (0.55, 0.25),
            Style::Heading2 => (0.5, 0.2),
            Style::Heading3 => (0.45, 0.15),
            Style::Quote => (0.25, 0.25),
            Style::Body => (0.0, 0.0),
        }
    }

    /// What Enter at the end of a paragraph of it starts: a heading is
    /// followed by body text, body by more of the same.
    pub fn next(self) -> Style {
        match self {
            Style::Body | Style::Quote => self,
            _ => Style::Body,
        }
    }
}

/// What a list item is marked with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum List {
    #[default]
    None,
    Bullet,
    Number,
    /// A box to tick, and whether it is.
    Check(bool),
}

impl List {
    /// What the item after one of these is: the same, a box unticked.
    pub fn next(self) -> List {
        match self {
            List::Check(_) => List::Check(false),
            other => other,
        }
    }

    /// Whether two items are of one list (a ticked box and an unticked
    /// one are).
    pub fn same_kind(self, other: List) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// The deepest a list nests.
pub const MAX_LEVEL: u8 = 6;

/// What a paragraph has.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParaAttrs {
    pub style: Style,
    pub align: Align,
    pub list: List,
    /// How far in a list item is nested, from 0.
    pub level: u8,
    /// Room over and under it in logical pixels, on top of its style's.
    pub space_before: f32,
    pub space_after: f32,
    /// How far in its first line starts, in logical pixels.
    pub first_indent: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_named_once_and_runs_share_what_they_have_in_common() {
        let (lora, again, inter) = (Font::named("Lora"), Font::named("Lora"), Font::named("Inter"));
        assert_eq!((lora, lora.name(), inter.name()), (again, "Lora", "Inter"));
        assert_ne!(lora, inter);
        let a = TextAttrs { bold: true, italic: true, size: Some(30.0), font: Some(lora), color: Some(0xff0000), ..TextAttrs::default() };
        let b = TextAttrs { bold: true, size: Some(30.0), font: Some(inter), color: Some(0xff0000), ..TextAttrs::default() };
        assert_eq!(a.common(b), TextAttrs { bold: true, size: Some(30.0), color: Some(0xff0000), ..TextAttrs::default() });
        for style in Style::ALL {
            assert_eq!(Style::from_word(style.word()), Some(style));
        }
        assert_eq!((Style::Heading2.next(), Style::Quote.next(), List::Check(true).next()), (Style::Body, Style::Quote, List::Check(false)));
        assert!(List::Check(true).same_kind(List::Check(false)) && !List::Bullet.same_kind(List::Number));
    }
}

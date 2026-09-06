//! Soft wrap for prose: a long line shown as several rows, broken after
//! spaces where it can be, the rows after the first hanging in under the
//! line's indent (and its list marker). Computed for the whole document
//! once per width and edit, so the view and the caret keys agree on
//! where the rows are.

use crate::buffer::Buffer;
use crate::editor::prose::list_item;
use crate::syntax::Language;
use crate::text_util::{cell_width, expand_tabs, indent_of};

/// A row after a line's first: the byte it starts at and that byte's
/// display cell in the unwrapped line.
pub type Break = (u32, u32);

/// The rows every line breaks into at one width.
#[derive(Default)]
pub struct Wrap {
    /// Cells per row; 0 when wrapping is off.
    pub width: usize,
    tab: usize,
    version: u64,
    lang: Option<Language>,
    breaks: Vec<Vec<Break>>,
    /// Cells the rows after the first hang in by, per line.
    hangs: Vec<u16>,
}

impl Wrap {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn active(&self) -> bool {
        self.width > 0
    }

    /// Bring the rows up to date for `buffer` at `width` cells (0: off).
    pub fn ensure(&mut self, buffer: &Buffer, tab: usize, width: usize, lang: Language) {
        let version = buffer.version();
        if self.width == width && self.tab == tab && self.version == version && self.lang == Some(lang) && self.breaks.len() == buffer.line_count() {
            return;
        }
        self.width = width;
        self.tab = tab;
        self.version = version;
        self.lang = Some(lang);
        let n = buffer.line_count();
        self.breaks.clear();
        self.hangs.clear();
        if width == 0 {
            return;
        }
        let mut expanded = String::new();
        let mut cells = Vec::new();
        for i in 0..n {
            let text = buffer.line(i);
            let hang = hang_of(text, tab, width, lang);
            self.hangs.push(hang as u16);
            self.breaks.push(wrap_line(text, tab, width, hang, &mut expanded, &mut cells));
        }
    }

    /// How many rows `line` takes.
    pub fn rows_of(&self, line: usize) -> usize {
        self.breaks.get(line).map_or(1, |b| b.len() + 1)
    }

    /// The row (within its line) byte `col` of `line` is on.
    pub fn seg_of(&self, line: usize, col: usize) -> usize {
        self.breaks.get(line).map_or(0, |b| b.iter().take_while(|(byte, _)| *byte as usize <= col).count())
    }

    /// Where row `seg` of `line` starts: the byte and its cell.
    pub fn seg_start(&self, line: usize, seg: usize) -> (usize, usize) {
        if seg == 0 {
            return (0, 0);
        }
        self.breaks.get(line).and_then(|b| b.get(seg - 1)).map_or((0, 0), |(byte, cell)| (*byte as usize, *cell as usize))
    }

    /// The byte row `seg` of `line` ends before, or `None` for the last row.
    pub fn seg_end(&self, line: usize, seg: usize) -> Option<usize> {
        self.breaks.get(line).and_then(|b| b.get(seg)).map(|(byte, _)| *byte as usize)
    }

    /// Cells row `seg` of `line` hangs in by.
    pub fn hang(&self, line: usize, seg: usize) -> usize {
        if seg == 0 { 0 } else { self.hangs.get(line).copied().unwrap_or(0) as usize }
    }
}

/// The indent (and list marker) a wrapped line's rows hang under, at
/// most half the width.
fn hang_of(text: &str, tab: usize, width: usize, lang: Language) -> usize {
    let body = match lang {
        Language::Markdown => list_item(text).map_or(indent_of(text).len(), |it| it.body),
        _ => indent_of(text).len(),
    };
    cell_width(&text[..body], tab).min(width / 2)
}

/// Where `text` breaks at `width` cells: after the last space that fits,
/// or inside a word that has none. Rows after the first hold `width -
/// hang` cells.
pub fn wrap_line(text: &str, tab: usize, width: usize, hang: usize, expanded: &mut String, cells: &mut Vec<u32>) -> Vec<Break> {
    let mut out = Vec::new();
    if width == 0 || cell_width(text, tab) <= width {
        return out;
    }
    expand_tabs(text, tab, expanded, cells);
    let width = width.max(4);
    let mut avail = width;
    let mut row_byte = 0usize;
    let mut row_cell = 0usize;
    let mut last_space: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        let end_cell = cells[i + ch.len_utf8()] as usize;
        if end_cell - row_cell > avail && i > row_byte {
            let brk = last_space.filter(|b| *b > row_byte).unwrap_or(i);
            out.push((brk as u32, cells[brk]));
            row_byte = brk;
            row_cell = cells[brk] as usize;
            last_space = None;
            avail = (width - hang).max(4);
        }
        if ch == ' ' {
            last_space = Some(i + ch.len_utf8());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaks(text: &str, width: usize, hang: usize) -> Vec<usize> {
        let (mut e, mut c) = (String::new(), Vec::new());
        wrap_line(text, 4, width, hang, &mut e, &mut c).into_iter().map(|(b, _)| b as usize).collect()
    }

    #[test]
    fn breaks_after_spaces_or_inside_long_words() {
        assert!(breaks("short", 10, 0).is_empty());
        assert_eq!(breaks("aaa bbb ccc ddd", 8, 0), vec![8], "a row holds what fits, spaces at its end");
        assert_eq!(breaks("aaa bbb ccc ddd", 6, 0), vec![4, 8, 12], "after each space that fits");
        assert_eq!(breaks("abcdefghijkl", 5, 0), vec![5, 10], "no spaces: hard breaks");
        assert_eq!(breaks("aaaa bbbb cccc dddd", 10, 0), vec![10]);
        assert_eq!(breaks("aaaa bbbb cccc dddd", 10, 3), vec![10, 15], "rows after the first are narrower by the hang");
    }

    #[test]
    fn rows_and_segments() {
        let b = Buffer::from_text("- one two three four\nx");
        let mut w = Wrap::none();
        w.ensure(&b, 4, 10, Language::Markdown);
        assert_eq!(w.rows_of(0), 3);
        assert_eq!(w.rows_of(1), 1);
        assert_eq!(w.hang(0, 1), 2, "rows hang under the list marker");
        assert_eq!(w.seg_of(0, 8), 0);
        assert_eq!(w.seg_of(0, 10), 1, "the break byte starts the next row");
        assert_eq!(w.seg_start(0, 1), (10, 10));
        assert_eq!(w.seg_end(0, 0), Some(10));
        assert_eq!(w.seg_end(0, 2), None);
        w.ensure(&b, 4, 0, Language::Markdown);
        assert!(!w.active());
        assert_eq!(w.rows_of(0), 1);
    }
}

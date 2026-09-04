//! Small text helpers shared by the editor, the finder and the terminal:
//! character classes, word boundaries, indentation, tab expansion and
//! bracket pairs. Columns are byte offsets into a line; cells are what the
//! monospace grid shows (a tab is several, a CJK character is two).

use crate::charwidth::char_cells;

/// A word character: letters, digits and underscore run together.
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The byte offset of the character before `i` (0 at the start).
pub fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i].chars().next_back().map_or(0, |c| i - c.len_utf8())
}

/// The byte offset after the character at `i` (`i` at the end).
pub fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Punct,
}

fn class(c: char) -> Class {
    if is_word(c) {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Punct
    }
}

/// Where Ctrl+Left lands from `i`: back over spaces, then over the run of
/// characters of one class.
pub fn word_left(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 {
        let p = prev_boundary(s, i);
        if class(s[p..].chars().next().unwrap_or(' ')) != Class::Space {
            break;
        }
        i = p;
    }
    if i == 0 {
        return 0;
    }
    let cls = class(s[prev_boundary(s, i)..].chars().next().unwrap_or(' '));
    while i > 0 {
        let p = prev_boundary(s, i);
        if class(s[p..].chars().next().unwrap_or(' ')) != cls {
            break;
        }
        i = p;
    }
    i
}

/// Where Ctrl+Right lands from `i`: over the run of characters of one
/// class, then over the spaces after it.
pub fn word_right(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    let Some(first) = s[i..].chars().next() else {
        return i;
    };
    let cls = class(first);
    while i < s.len() && class(s[i..].chars().next().unwrap_or(' ')) == cls {
        i = next_boundary(s, i);
    }
    while i < s.len() && class(s[i..].chars().next().unwrap_or('x')) == Class::Space {
        i = next_boundary(s, i);
    }
    i
}

/// The word around byte `i` (what a double click selects): a run of word
/// characters, or the single other character there.
pub fn word_at(s: &str, i: usize) -> (usize, usize) {
    let i = i.min(s.len());
    let here = s[i..].chars().next().or_else(|| s[..i].chars().next_back());
    let Some(c) = here else {
        return (i, i);
    };
    if !is_word(c) {
        // Prefer the word just before the caret when it sits after one.
        if i > 0 && s[..i].chars().next_back().is_some_and(is_word) {
            return word_at(s, prev_boundary(s, i));
        }
        let start = if s[i..].chars().next().is_some() { i } else { prev_boundary(s, i) };
        return (start, next_boundary(s, start));
    }
    let mut start = i;
    while start > 0 {
        let p = prev_boundary(s, start);
        if !s[p..].chars().next().is_some_and(is_word) {
            break;
        }
        start = p;
    }
    let mut end = i;
    while end < s.len() && s[end..].chars().next().is_some_and(is_word) {
        end = next_boundary(s, end);
    }
    (start, end)
}

/// The leading whitespace of a line.
pub fn indent_of(s: &str) -> &str {
    let n = s.len() - s.trim_start_matches([' ', '\t']).len();
    &s[..n]
}

/// Display cells taken by `s` with tab stops every `tab` cells.
pub fn cell_width(s: &str, tab: usize) -> usize {
    let tab = tab.max(1);
    let mut w = 0;
    for c in s.chars() {
        w += if c == '\t' { tab - w % tab } else { char_cells(c) };
    }
    w
}

/// `s` with tabs turned into spaces, and the display cell of every byte
/// offset `0..=len` in `cells` (so callers can place carets and colors
/// without walking the string again).
pub fn expand_tabs(s: &str, tab: usize, out: &mut String, cells: &mut Vec<u32>) {
    let tab = tab.max(1);
    out.clear();
    cells.clear();
    let mut w = 0usize;
    for c in s.chars() {
        for _ in 0..c.len_utf8() {
            cells.push(w as u32);
        }
        if c == '\t' {
            let n = tab - w % tab;
            out.extend(std::iter::repeat_n(' ', n));
            w += n;
        } else {
            out.push(c);
            w += char_cells(c);
        }
    }
    cells.push(w as u32);
}

/// The display cell at byte `byte` of `s`.
pub fn cell_of_byte(s: &str, tab: usize, byte: usize) -> usize {
    cell_width(&s[..byte.min(s.len())], tab)
}

/// The byte offset in `s` whose display cell is nearest `cell` (rounding
/// to the closer edge of a wide character or tab).
pub fn byte_at_cell(s: &str, tab: usize, cell: usize) -> usize {
    let tab = tab.max(1);
    let mut w = 0usize;
    let mut b = 0usize;
    for c in s.chars() {
        let cw = if c == '\t' { tab - w % tab } else { char_cells(c) };
        if cell < w + cw {
            return if cell - w < cw.div_ceil(2) { b } else { b + c.len_utf8() };
        }
        w += cw;
        b += c.len_utf8();
    }
    s.len()
}

/// Bracket pairs the editor matches and auto-closes.
pub const BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// `(open, close, is_open)` when `c` is a bracket.
pub fn bracket_pair(c: char) -> Option<(char, char, bool)> {
    BRACKETS.iter().find_map(|&(o, cl)| if c == o { Some((o, cl, true)) } else if c == cl { Some((o, cl, false)) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        let s = "let foo_bar = baz(1);";
        assert_eq!(word_at(s, 5), (4, 11));
        assert_eq!(word_at(s, 11), (4, 11), "after the word still means the word");
        assert_eq!(word_at(s, 12), (12, 13), "punctuation is its own word");
        assert_eq!(word_right(s, 0), 4);
        assert_eq!(word_right(s, 4), 12);
        assert_eq!(word_left(s, 12), 4);
        assert_eq!(word_left(s, 4), 0);
        assert_eq!(word_left("  ", 2), 0);
    }

    #[test]
    fn tabs_and_cells() {
        assert_eq!(cell_width("\tx", 4), 5);
        assert_eq!(cell_width("ab\tx", 4), 5);
        let mut out = String::new();
        let mut cells = Vec::new();
        expand_tabs("a\tb", 4, &mut out, &mut cells);
        assert_eq!(out, "a   b");
        assert_eq!(cells, vec![0, 1, 4, 5]);
        assert_eq!(byte_at_cell("a\tb", 4, 2), 1, "left half of the tab");
        assert_eq!(byte_at_cell("a\tb", 4, 3), 2, "right half of the tab");
        assert_eq!(byte_at_cell("abc", 4, 99), 3);
        assert_eq!(cell_of_byte("\t\tx", 4, 2), 8);
        assert_eq!(indent_of("  \tfoo"), "  \t");
        assert_eq!(cell_width("日本", 4), 4);
    }
}

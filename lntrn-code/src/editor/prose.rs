//! Markdown as prose: what a list line is, and what Enter and Tab do on
//! one. Enter continues a list (`- `, `1. `, `- [ ] `, `> `), numbers
//! counting up; Enter on an item with nothing in it ends the list.

use crate::buffer::{Pos, Range};
use crate::doc::{Doc, EditKind};
use crate::syntax::Language;
use crate::text_util::indent_of;

/// A line's list marker, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// The leading whitespace.
    pub indent: String,
    /// The marker after it, spaces included: `- `, `3. `, `- [ ] `, `> `.
    pub marker: String,
    /// Where the item's text starts.
    pub body: usize,
}

impl ListItem {
    /// The marker the next item wears: a number one up, a box unticked.
    pub fn next_marker(&self) -> String {
        let m = &self.marker;
        let digits = m.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            let n: u64 = m[..digits].parse().unwrap_or(0);
            return format!("{}{}", n + 1, &m[digits..]);
        }
        m.replace("[x] ", "[ ] ").replace("[X] ", "[ ] ")
    }
}

/// The list marker (or quote mark) at the start of `line`, if any.
pub fn list_item(line: &str) -> Option<ListItem> {
    let indent = indent_of(line);
    let t = &line[indent.len()..];
    let b = t.as_bytes();
    let mut i = if matches!(b.first(), Some(b'-' | b'*' | b'+' | b'>')) && b.get(1) == Some(&b' ') {
        2
    } else {
        let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 || !matches!(b.get(digits), Some(b'.' | b')')) || b.get(digits + 1) != Some(&b' ') {
            return None;
        }
        digits + 2
    };
    if b[0] != b'>' && (t[i..].starts_with("[ ] ") || t[i..].starts_with("[x] ") || t[i..].starts_with("[X] ")) {
        i += 4;
    }
    Some(ListItem { indent: indent.to_owned(), marker: t[..i].to_owned(), body: indent.len() + i })
}

/// Whether Enter on this document continues lists.
pub fn is_prose(doc: &Doc) -> bool {
    doc.lang() == Language::Markdown
}

/// Enter on a list line: the next item, or the end of the list when the
/// item was empty. `false` when the line is no list item (a plain
/// newline is the caller's).
pub fn continue_list(doc: &mut Doc, now: f64) -> bool {
    let r = doc.selection();
    if !r.is_empty() {
        return false;
    }
    let line = doc.buffer.line(r.start.line);
    let Some(item) = list_item(line) else {
        return false;
    };
    if r.start.col < item.body {
        return false;
    }
    if line[item.body..].trim().is_empty() {
        // An empty item ends the list: the marker goes, the line stays.
        let whole = Range::new(Pos::new(r.start.line, 0), Pos::new(r.start.line, line.len()));
        let end = doc.edit(whole, "", EditKind::Other, now);
        doc.set_cursor(end, false);
        return true;
    }
    let text = format!("\n{}{}", item.indent, item.next_marker());
    let end = doc.edit(r, &text, EditKind::Other, now);
    doc.set_cursor(end, false);
    true
}

/// Tab with the caret on a list line: the whole item steps in, not the
/// caret. `true` when the line is a list item.
pub fn on_list_line(doc: &Doc) -> bool {
    !doc.has_selection() && list_item(doc.buffer.line(doc.cursor.line)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::DocId;
    use std::path::PathBuf;

    #[test]
    fn markers_parse_and_count_up() {
        assert_eq!(list_item("  - [ ] task").unwrap().marker, "- [ ] ");
        assert_eq!(list_item("12. twelve").unwrap().next_marker(), "13. ");
        assert_eq!(list_item("- [x] done").unwrap().next_marker(), "- [ ] ");
        assert_eq!(list_item("> quote").unwrap().next_marker(), "> ");
        assert!(list_item("-not a list").is_none());
        assert!(list_item("plain").is_none());
    }

    #[test]
    fn enter_continues_then_ends() {
        let mut d = Doc::from_text(DocId(1), Some(PathBuf::from("/t/a.md")), "1. one", 4);
        d.set_cursor(Pos::new(0, 6), false);
        assert!(continue_list(&mut d, 0.0));
        assert_eq!(d.buffer.to_text(), "1. one\n2. ");
        assert_eq!(d.cursor, Pos::new(1, 3));
        assert!(continue_list(&mut d, 0.0), "an empty item ends the list");
        assert_eq!(d.buffer.to_text(), "1. one\n");
        assert_eq!(d.cursor, Pos::new(1, 0));
        let mut p = Doc::from_text(DocId(2), Some(PathBuf::from("/t/a.md")), "text", 4);
        p.set_cursor(Pos::new(0, 4), false);
        assert!(!continue_list(&mut p, 0.0), "no list, no continuation");
    }
}

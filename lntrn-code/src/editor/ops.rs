//! Editing operations on a document: typing with bracket pairs, new
//! lines that keep the indent, indenting and commenting whole selections,
//! moving and copying lines, word deletion, and bracket matching. Each is
//! one undo step.

use crate::buffer::{Pos, Range};
use crate::doc::{Doc, EditKind};
use crate::settings::Settings;
use crate::text_util::{bracket_pair, indent_of, is_word, word_left, word_right};

/// What one press of Tab inserts.
pub fn indent_unit(settings: &Settings) -> String {
    if settings.insert_spaces { " ".repeat(settings.tab()) } else { "\t".to_owned() }
}

/// The lines a selection covers: a selection ending at the start of a
/// line does not include that line.
pub fn selected_lines(doc: &Doc) -> (usize, usize) {
    let r = doc.selection();
    let last = if r.end.line > r.start.line && r.end.col == 0 { r.end.line - 1 } else { r.end.line };
    (r.start.line, last)
}

/// Typed text, with brackets and quotes paired: an opener wraps the
/// selection or gets its closer typed ahead; a closer typed onto the one
/// already there steps over it.
pub fn type_text(doc: &mut Doc, t: &str, now: f64) {
    let mut chars = t.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        doc.insert(t, now);
        return;
    };
    let quote = matches!(c, '"' | '\'' | '`');
    let pair = bracket_pair(c);
    let next = doc.buffer.char_at(doc.cursor);
    let prev = doc.buffer.char_before(doc.cursor);
    if doc.has_selection() {
        if let Some((o, cl, true)) = pair {
            wrap_selection(doc, o, cl, now);
            return;
        }
        if quote {
            wrap_selection(doc, c, c, now);
            return;
        }
        doc.insert(t, now);
        return;
    }
    // Step over the closer that is already there.
    if next == Some(c) && (matches!(pair, Some((_, _, false))) || (quote && prev != Some('\\'))) {
        doc.set_cursor(doc.buffer.next_pos(doc.cursor), false);
        return;
    }
    let room_after = next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ';' | ',' | '"' | '\'' | '`'));
    let open_ok = match pair {
        Some((_, _, true)) => room_after,
        _ => quote && room_after && !prev.is_some_and(|p| is_word(p) || p == c) && doc.lang() != crate::syntax::Language::Plain,
    };
    if open_ok {
        let close = pair.map_or(c, |(_, cl, _)| cl);
        let mut s = String::new();
        s.push(c);
        s.push(close);
        let end = doc.insert(&s, now);
        doc.set_cursor(doc.buffer.prev_pos(end), false);
    } else {
        doc.insert(t, now);
    }
}

fn wrap_selection(doc: &mut Doc, open: char, close: char, now: f64) {
    let r = doc.selection();
    let inner = doc.buffer.text_in(r);
    let text = format!("{open}{inner}{close}");
    let end = doc.edit(r, &text, EditKind::Other, now);
    let start = doc.buffer.next_pos(r.start);
    doc.select(Range::new(start, doc.buffer.prev_pos(end)));
}

/// Enter: a new line with the indent of the one before, one level deeper
/// after an opener (and the closer moved to a line of its own when it
/// sits right after the caret).
pub fn newline(doc: &mut Doc, settings: &Settings, now: f64) {
    let r = doc.selection();
    let line = doc.buffer.line(r.start.line);
    let mut indent = indent_of(line).to_owned();
    let prev = line[..r.start.col].trim_end().chars().next_back();
    let next = doc.buffer.char_at(r.end);
    let opener = prev.is_some_and(|p| matches!(bracket_pair(p), Some((_, _, true))) || (p == ':' && doc.lang() == crate::syntax::Language::Python));
    let unit = indent_unit(settings);
    let mut text = format!("\n{indent}");
    let mut caret_after = None;
    if opener {
        text.push_str(&unit);
        if let (Some(p), Some(n)) = (prev, next)
            && bracket_pair(p).is_some_and(|(_, cl, _)| cl == n)
        {
            // `{|}` → the closer goes on its own line below the caret.
            let inner_len = text.len();
            text.push('\n');
            text.push_str(&indent);
            caret_after = Some(inner_len);
        }
    }
    indent.clear();
    let start = r.start;
    let end = doc.edit(r, &text, EditKind::Other, now);
    if let Some(len) = caret_after {
        // The caret sits at the end of the indented middle line.
        let mid = Pos::new(start.line + 1, len - 1 - text[..len].rfind('\n').unwrap_or(0));
        doc.set_cursor(mid, false);
    } else {
        doc.set_cursor(end, false);
    }
}

/// Ctrl+Enter: a fresh line below (or above) the caret's, indented like it.
pub fn insert_line(doc: &mut Doc, above: bool, now: f64) {
    let line = doc.cursor.line;
    let indent = indent_of(doc.buffer.line(line)).to_owned();
    if above {
        let end = doc.edit(Range::at(Pos::new(line, 0)), &format!("{indent}\n"), EditKind::Other, now);
        doc.set_cursor(Pos::new(end.line - 1, indent.len()), false);
    } else {
        let at = Pos::new(line, doc.buffer.line(line).len());
        let end = doc.edit(Range::at(at), &format!("\n{indent}"), EditKind::Other, now);
        doc.set_cursor(end, false);
    }
}

/// Replace whole lines `first..=last` with `new_lines` as one step, then
/// select them (or put the caret back on its line when there was no
/// selection).
fn replace_lines(doc: &mut Doc, first: usize, last: usize, new_lines: Vec<String>, keep_caret: bool, now: f64) {
    let had_selection = doc.has_selection();
    let caret = doc.cursor;
    let count = new_lines.len();
    let text = new_lines.join("\n");
    let r = Range::new(Pos::new(first, 0), Pos::new(last, doc.buffer.line(last).len()));
    doc.edit(r, &text, EditKind::Other, now);
    if had_selection || !keep_caret {
        let end_line = first + count.saturating_sub(1);
        doc.select(Range::new(Pos::new(first, 0), Pos::new(end_line, doc.buffer.line(end_line).len())));
    } else {
        doc.set_cursor(Pos::new(caret.line.min(first + count.saturating_sub(1)), caret.col), false);
    }
}

/// Tab with a selection, or Ctrl+]: one unit deeper on every line.
pub fn indent_lines(doc: &mut Doc, settings: &Settings, now: f64) {
    let (first, last) = selected_lines(doc);
    let unit = indent_unit(settings);
    let caret_col = doc.cursor.col;
    let had_selection = doc.has_selection();
    let lines: Vec<String> = (first..=last).map(|i| if doc.buffer.line(i).is_empty() { String::new() } else { format!("{unit}{}", doc.buffer.line(i)) }).collect();
    replace_lines(doc, first, last, lines, true, now);
    if !had_selection {
        doc.set_cursor(Pos::new(first, caret_col + unit.len()), false);
    }
}

/// Shift+Tab or Ctrl+[: one unit shallower on every line that has one.
pub fn dedent_lines(doc: &mut Doc, settings: &Settings, now: f64) {
    let (first, last) = selected_lines(doc);
    let tab = settings.tab();
    let caret = doc.cursor;
    let had_selection = doc.has_selection();
    let mut removed_on_caret = 0;
    let lines: Vec<String> = (first..=last)
        .map(|i| {
            let l = doc.buffer.line(i);
            let cut = if l.starts_with('\t') {
                1
            } else {
                l.bytes().take(tab).take_while(|&b| b == b' ').count()
            };
            if i == caret.line {
                removed_on_caret = cut;
            }
            l[cut..].to_owned()
        })
        .collect();
    replace_lines(doc, first, last, lines, true, now);
    if !had_selection {
        doc.set_cursor(Pos::new(caret.line, caret.col.saturating_sub(removed_on_caret)), false);
    }
}

/// Ctrl+/: comment every selected line out, or back in when they all are.
pub fn toggle_comment(doc: &mut Doc, now: f64) {
    let Some(marker) = doc.lang().line_comment() else {
        return;
    };
    let (first, last) = selected_lines(doc);
    let caret = doc.cursor;
    let had_selection = doc.has_selection();
    let all_commented = (first..=last).filter(|&i| !doc.buffer.line(i).trim().is_empty()).all(|i| doc.buffer.line(i).trim_start().starts_with(marker));
    let min_indent = (first..=last).filter(|&i| !doc.buffer.line(i).trim().is_empty()).map(|i| indent_of(doc.buffer.line(i)).len()).min().unwrap_or(0);
    let mut caret_shift = 0isize;
    let lines: Vec<String> = (first..=last)
        .map(|i| {
            let l = doc.buffer.line(i);
            if l.trim().is_empty() {
                return l.to_owned();
            }
            if all_commented {
                let ind = indent_of(l).len();
                let rest = &l[ind + marker.len()..];
                let extra = usize::from(rest.starts_with(' '));
                if i == caret.line {
                    caret_shift = -((marker.len() + extra) as isize);
                }
                format!("{}{}", &l[..ind], &rest[extra..])
            } else {
                if i == caret.line {
                    caret_shift = (marker.len() + 1) as isize;
                }
                format!("{}{marker} {}", &l[..min_indent], &l[min_indent..])
            }
        })
        .collect();
    replace_lines(doc, first, last, lines, true, now);
    if !had_selection {
        let col = (caret.col as isize + caret_shift).max(0) as usize;
        doc.set_cursor(Pos::new(caret.line, col), false);
    }
}

/// Alt+Up/Down: the selected lines trade places with their neighbour.
pub fn move_lines(doc: &mut Doc, down: bool, now: f64) {
    let (first, last) = selected_lines(doc);
    let n = doc.buffer.line_count();
    if (down && last + 1 >= n) || (!down && first == 0) {
        return;
    }
    let (cursor, anchor) = (doc.cursor, doc.anchor);
    let (lo, hi) = if down { (first, last + 1) } else { (first - 1, last) };
    let mut lines: Vec<String> = (lo..=hi).map(|i| doc.buffer.line(i).to_owned()).collect();
    if down {
        let moved = lines.pop().expect("neighbour");
        lines.insert(0, moved);
    } else {
        let moved = lines.remove(0);
        lines.push(moved);
    }
    replace_lines(doc, lo, hi, lines, false, now);
    let by = if down { 1 } else { -1isize };
    let shift = |p: Pos| Pos::new((p.line as isize + by) as usize, p.col);
    doc.anchor = doc.buffer.clamp(shift(anchor));
    doc.cursor = doc.buffer.clamp(shift(cursor));
}

/// Shift+Alt+Down / Ctrl+Shift+D: the selected lines again, below.
pub fn duplicate_lines(doc: &mut Doc, now: f64) {
    let (first, last) = selected_lines(doc);
    let (cursor, anchor) = (doc.cursor, doc.anchor);
    let block: Vec<String> = (first..=last).map(|i| doc.buffer.line(i).to_owned()).collect();
    let at = Pos::new(last, doc.buffer.line(last).len());
    doc.edit(Range::at(at), &format!("\n{}", block.join("\n")), EditKind::Other, now);
    let by = last - first + 1;
    doc.anchor = doc.buffer.clamp(Pos::new(anchor.line + by, anchor.col));
    doc.cursor = doc.buffer.clamp(Pos::new(cursor.line + by, cursor.col));
}

/// Ctrl+Shift+K: the selected lines, gone.
pub fn delete_lines(doc: &mut Doc, now: f64) {
    let (first, last) = selected_lines(doc);
    let n = doc.buffer.line_count();
    let col = doc.cursor.col;
    let r = if last + 1 < n {
        Range::new(Pos::new(first, 0), Pos::new(last + 1, 0))
    } else if first > 0 {
        Range::new(Pos::new(first - 1, doc.buffer.line(first - 1).len()), Pos::new(last, doc.buffer.line(last).len()))
    } else {
        Range::new(Pos::new(0, 0), doc.buffer.end())
    };
    doc.edit(r, "", EditKind::Other, now);
    let line = first.min(doc.buffer.line_count() - 1);
    doc.set_cursor(Pos::new(line, col), false);
}

/// Ctrl+L: the caret's line, or one more line of the selection.
pub fn select_line(doc: &mut Doc) {
    let (first, last) = selected_lines(doc);
    let n = doc.buffer.line_count();
    let end = if last + 1 < n { Pos::new(last + 1, 0) } else { Pos::new(last, doc.buffer.line(last).len()) };
    doc.select(Range::new(Pos::new(first, 0), end));
}

/// Backspace: the selection, a whole indent level inside leading
/// whitespace, both halves of an empty pair, or one character.
pub fn backspace(doc: &mut Doc, settings: &Settings, now: f64) {
    if doc.has_selection() {
        doc.delete(doc.selection(), now);
        return;
    }
    let p = doc.cursor;
    if p.col == 0 {
        if p.line > 0 {
            doc.delete(Range::new(doc.buffer.prev_pos(p), p), now);
        }
        return;
    }
    let line = doc.buffer.line(p.line);
    let before = &line[..p.col];
    if settings.insert_spaces && !before.is_empty() && before.bytes().all(|b| b == b' ') {
        let tab = settings.tab();
        let back = ((before.len() - 1) % tab) + 1;
        doc.delete(Range::new(Pos::new(p.line, p.col - back), p), now);
        return;
    }
    let prev = doc.buffer.char_before(p);
    let next = doc.buffer.char_at(p);
    let empty_pair = match (prev, next) {
        (Some(a), Some(b)) => bracket_pair(a).is_some_and(|(_, cl, open)| open && cl == b) || (matches!(a, '"' | '\'' | '`') && a == b),
        _ => false,
    };
    let end = if empty_pair { doc.buffer.next_pos(p) } else { p };
    doc.delete(Range::new(doc.buffer.prev_pos(p), end), now);
}

pub fn delete_forward(doc: &mut Doc, now: f64) {
    if doc.has_selection() {
        doc.delete(doc.selection(), now);
    } else {
        let p = doc.cursor;
        doc.delete(Range::new(p, doc.buffer.next_pos(p)), now);
    }
}

/// Ctrl+Backspace / Ctrl+Delete.
pub fn delete_word(doc: &mut Doc, forward: bool, now: f64) {
    if doc.has_selection() {
        doc.delete(doc.selection(), now);
        return;
    }
    let p = doc.cursor;
    let line = doc.buffer.line(p.line);
    let other = if forward {
        if p.col >= line.len() { doc.buffer.next_pos(p) } else { Pos::new(p.line, word_right(line, p.col)) }
    } else if p.col == 0 {
        doc.buffer.prev_pos(p)
    } else {
        Pos::new(p.line, word_left(line, p.col))
    };
    doc.delete(Range::new(p, other), now);
}

/// Where Home goes: the first non-blank column, or column 0 from there.
pub fn smart_home(doc: &Doc) -> Pos {
    let p = doc.cursor;
    let first = indent_of(doc.buffer.line(p.line)).len();
    Pos::new(p.line, if p.col == first { 0 } else { first })
}

/// The bracket paired with the one at or just before `at`, within a
/// reasonable distance. Returns `(this, other)`.
pub fn matching_bracket(doc: &Doc, at: Pos) -> Option<(Pos, Pos)> {
    let here = doc.buffer.char_at(at).and_then(|c| bracket_pair(c).map(|p| (at, p)));
    let before = || doc.buffer.char_before(at).and_then(|c| bracket_pair(c).map(|p| (doc.buffer.prev_pos(at), p)));
    let (pos, (open, close, is_open)) = here.or_else(before)?;
    let mut depth = 0i32;
    let mut p = pos;
    let mut steps = 0;
    loop {
        let c = doc.buffer.char_at(p);
        if c == Some(open) {
            depth += if is_open { 1 } else { -1 };
        } else if c == Some(close) {
            depth += if is_open { -1 } else { 1 };
        }
        if depth == 0 {
            return Some((pos, p));
        }
        let next = if is_open { doc.buffer.next_pos(p) } else { doc.buffer.prev_pos(p) };
        if next == p {
            return None;
        }
        p = next;
        steps += 1;
        if steps > 200_000 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::DocId;

    fn doc(text: &str) -> Doc {
        let mut d = Doc::from_text(DocId(1), Some("x.rs".into()), text, 4);
        d.set_cursor(d.buffer.end(), false);
        d
    }

    #[test]
    fn pairs_and_newlines() {
        let s = Settings::default();
        let mut d = doc("fn f() ");
        type_text(&mut d, "{", 0.0);
        assert_eq!(d.buffer.line(0), "fn f() {}");
        assert_eq!(d.cursor, Pos::new(0, 8));
        newline(&mut d, &s, 0.1);
        assert_eq!(d.buffer.lines(), &["fn f() {", "    ", "}"]);
        assert_eq!(d.cursor, Pos::new(1, 4));
        type_text(&mut d, "}", 0.2);
        assert_eq!(d.buffer.line(1), "    }", "no closer to step over on this line");
        let mut d = doc("(");
        type_text(&mut d, ")", 0.0);
        assert_eq!(d.buffer.line(0), "()", "typed onto the missing closer");
        let mut d = doc("()");
        d.set_cursor(Pos::new(0, 1), false);
        type_text(&mut d, ")", 0.0);
        assert_eq!(d.buffer.line(0), "()", "stepped over");
        assert_eq!(d.cursor.col, 2);
        backspace(&mut d, &s, 0.0);
        backspace(&mut d, &s, 0.1);
        assert_eq!(d.buffer.line(0), "");
        let mut d = doc("x");
        d.select_all();
        type_text(&mut d, "\"", 0.0);
        assert_eq!(d.buffer.line(0), "\"x\"");
        assert_eq!(d.selected_text(), "x");
    }

    #[test]
    fn line_operations() {
        let s = Settings::default();
        let mut d = doc("a\nb\nc");
        d.select(Range::new(Pos::new(0, 0), Pos::new(2, 0)));
        indent_lines(&mut d, &s, 0.0);
        assert_eq!(d.buffer.lines(), &["    a", "    b", "c"]);
        dedent_lines(&mut d, &s, 0.1);
        assert_eq!(d.buffer.lines(), &["a", "b", "c"]);
        toggle_comment(&mut d, 0.2);
        assert_eq!(d.buffer.lines(), &["// a", "// b", "c"]);
        toggle_comment(&mut d, 0.3);
        assert_eq!(d.buffer.lines(), &["a", "b", "c"]);
        d.set_cursor(Pos::new(0, 1), false);
        move_lines(&mut d, true, 0.4);
        assert_eq!(d.buffer.lines(), &["b", "a", "c"]);
        assert_eq!(d.cursor, Pos::new(1, 1));
        duplicate_lines(&mut d, 0.5);
        assert_eq!(d.buffer.lines(), &["b", "a", "a", "c"]);
        assert_eq!(d.cursor.line, 2);
        delete_lines(&mut d, 0.6);
        assert_eq!(d.buffer.lines(), &["b", "a", "c"]);
        select_line(&mut d);
        assert_eq!(d.selected_text(), "c");
        assert!(d.undo(1.0) && d.undo(1.1));
        assert_eq!(d.buffer.lines(), &["b", "a", "c"], "duplicate then delete undone as two steps");
    }

    #[test]
    fn words_and_brackets() {
        let mut d = doc("foo bar(baz)");
        d.set_cursor(Pos::new(0, 7), false);
        delete_word(&mut d, false, 0.0);
        assert_eq!(d.buffer.line(0), "foo (baz)");
        let (a, b) = matching_bracket(&d, Pos::new(0, 4)).unwrap();
        assert_eq!((a, b), (Pos::new(0, 4), Pos::new(0, 8)));
        let (a, b) = matching_bracket(&d, Pos::new(0, 9)).unwrap();
        assert_eq!((a, b), (Pos::new(0, 8), Pos::new(0, 4)), "the bracket before the caret");
        assert!(matching_bracket(&d, Pos::new(0, 1)).is_none());
        d.set_cursor(Pos::new(0, 4), false);
        assert_eq!(smart_home(&d), Pos::new(0, 0));
    }
}

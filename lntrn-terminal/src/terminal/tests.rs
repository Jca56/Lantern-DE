//! Copy/selection extraction tests — soft-wrap joining vs hard newlines.

use super::*;

fn term(cols: usize, rows: usize, input: &[u8]) -> TerminalState {
    let mut t = TerminalState::new(cols, rows);
    t.process(input);
    t
}

fn select(t: &mut TerminalState, start: (usize, usize), end: (usize, usize)) -> String {
    t.set_selection_anchor(start.0, start.1);
    t.set_selection_end(end.0, end.1);
    t.selected_text().unwrap_or_default()
}

#[test]
fn soft_wrapped_rows_join_without_newline() {
    // 22 chars across 10 cols → 3 rows, all one logical line
    let mut t = term(10, 5, b"this is a long command");
    assert_eq!(
        select(&mut t, (0, 0), (2, 9)),
        "this is a long command"
    );
}

#[test]
fn hard_newlines_are_preserved() {
    let mut t = term(10, 5, b"foo\r\nbar");
    assert_eq!(select(&mut t, (0, 0), (1, 9)), "foo\nbar");
}

#[test]
fn exact_width_line_with_crlf_is_not_joined() {
    // Line fills the row exactly, then a real newline: CR clears the
    // pending wrap, so the rows must stay separate lines.
    let mut t = term(10, 5, b"1234567890\r\nnext");
    assert_eq!(select(&mut t, (0, 0), (1, 9)), "1234567890\nnext");
}

#[test]
fn spaces_at_wrap_boundary_survive_copy() {
    // "ab   " fills the row; the trailing spaces are real content in the
    // middle of the logical line and must not be trimmed away.
    let mut t = term(5, 5, b"ab   cd");
    assert_eq!(select(&mut t, (0, 0), (1, 4)), "ab   cd");
}

#[test]
fn wide_char_wrap_filler_is_not_copied() {
    // '日' doesn't fit in the last column: a filler blank pads the row and
    // the char wraps. Copy must yield "abcd日" with no phantom space.
    let mut t = term(5, 5, "abcd日".as_bytes());
    assert_eq!(select(&mut t, (0, 0), (1, 4)), "abcd日");
}

#[test]
fn wrapped_rows_joined_across_scrollback() {
    // Wrap a long line, then push it into scrollback with newlines so the
    // join has to read absolute (scrollback) rows — the wrap flag must
    // travel with the line when scroll_up moves it out of the grid.
    let mut t = term(10, 3, b"this is a long command\r\n\r\n\r\n\r\n");
    assert!(!t.scrollback.is_empty(), "line never reached scrollback");
    // Selection in absolute coordinates: from the very first line ever
    // printed through the last visible row.
    t.selection_anchor = Some((0, 0));
    t.selection_end = Some((t.scrollback.len() + t.rows - 1, 9));
    let all = t.selected_text().unwrap_or_default();
    assert!(
        all.starts_with("this is a long command"),
        "wrapped line split after scrollback: {all:?}"
    );
}

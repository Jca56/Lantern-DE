//! Find the bracket that matches the one next to the cursor. Used by the
//! renderer to draw both brackets in bold for visual matching.
//!
//! The cursor is considered "next to" a bracket when it sits either right
//! before an open bracket (`|(...)`) or right after a close bracket
//! (`(...)|`). Walks across line boundaries; gives up after a small line
//! budget so massive files don't stall the renderer.

use crate::editor::{Editor, Pos};

const MAX_LINE_BUDGET: usize = 5000;

/// Split a span's clipped byte range at any bracket-match positions that
/// fall within it. Returns a list of `(start, end, is_bracket)` segments in
/// order. ASCII brackets are 1 byte, so each match contributes a 1-byte
/// segment. Used by `render.rs` to render matched brackets in bold.
pub fn split_at_bracket_cols(
    start: usize,
    end: usize,
    bracket_cols: &[usize],
) -> Vec<(usize, usize, bool)> {
    if bracket_cols.is_empty() {
        return vec![(start, end, false)];
    }
    let mut hits: Vec<usize> = bracket_cols
        .iter()
        .copied()
        .filter(|&b| b >= start && b < end)
        .collect();
    if hits.is_empty() {
        return vec![(start, end, false)];
    }
    hits.sort_unstable();
    hits.dedup();
    let mut out = Vec::with_capacity(hits.len() * 2 + 1);
    let mut pos = start;
    for b in hits {
        if pos < b {
            out.push((pos, b, false));
        }
        let bracket_end = (b + 1).min(end);
        out.push((b, bracket_end, true));
        pos = bracket_end;
    }
    if pos < end {
        out.push((pos, end, false));
    }
    out
}

/// Returns `Some((open_pos, close_pos))` if the cursor is next to a bracket
/// with a balanced match somewhere in the document. Both positions are byte
/// offsets pointing AT the respective bracket character.
pub fn find_matching(editor: &Editor) -> Option<(Pos, Pos)> {
    if editor.lines.is_empty() {
        return None;
    }
    let line_idx = editor.cursor_line.min(editor.lines.len() - 1);
    let line = &editor.lines[line_idx];
    let col = editor.cursor_col.min(line.len());

    // Char immediately at the cursor (open-bracket case).
    if let Some(ch) = char_at(line, col) {
        if let Some(close) = match_open(ch) {
            let from = col + ch.len_utf8();
            if let Some(end) = scan_forward(editor, line_idx, from, ch, close) {
                return Some((Pos::new(line_idx, col), end));
            }
        }
    }
    // Char immediately before the cursor (close-bracket case).
    if col > 0 {
        let prev_col = prev_char_boundary(line, col);
        if let Some(ch) = char_at(line, prev_col) {
            if let Some(open) = match_close(ch) {
                if let Some(start) = scan_backward(editor, line_idx, prev_col, ch, open) {
                    return Some((start, Pos::new(line_idx, prev_col)));
                }
            }
        }
    }
    None
}

fn char_at(line: &str, col: usize) -> Option<char> {
    if col >= line.len() {
        return None;
    }
    line[col..].chars().next()
}

fn prev_char_boundary(line: &str, col: usize) -> usize {
    let mut p = col.saturating_sub(1);
    while p > 0 && !line.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn match_open(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn match_close(ch: char) -> Option<char> {
    match ch {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

fn scan_forward(
    editor: &Editor,
    start_line: usize,
    start_col: usize,
    open: char,
    close: char,
) -> Option<Pos> {
    let mut depth: i32 = 1;
    let mut line_idx = start_line;
    let mut col = start_col;
    let mut budget = MAX_LINE_BUDGET;
    while line_idx < editor.lines.len() && budget > 0 {
        let line = &editor.lines[line_idx];
        while col < line.len() {
            let ch = line[col..].chars().next()?;
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return Some(Pos::new(line_idx, col));
                }
            }
            col += ch.len_utf8();
        }
        line_idx += 1;
        col = 0;
        budget -= 1;
    }
    None
}

fn scan_backward(
    editor: &Editor,
    start_line: usize,
    start_col: usize,
    close: char,
    open: char,
) -> Option<Pos> {
    let mut depth: i32 = 1;
    let mut line_idx = start_line as i64;
    let mut col = start_col;
    let mut budget = MAX_LINE_BUDGET;
    while line_idx >= 0 && budget > 0 {
        let line = &editor.lines[line_idx as usize];
        while col > 0 {
            let prev = prev_char_boundary(line, col);
            let ch = line[prev..col].chars().next()?;
            if ch == close {
                depth += 1;
            } else if ch == open {
                depth -= 1;
                if depth == 0 {
                    return Some(Pos::new(line_idx as usize, prev));
                }
            }
            col = prev;
        }
        line_idx -= 1;
        if line_idx >= 0 {
            col = editor.lines[line_idx as usize].len();
        }
        budget -= 1;
    }
    None
}

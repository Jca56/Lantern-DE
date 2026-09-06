//! Keys and typed text for a focused document, replayed in arrival order.
//! Only the keys the editor answers to are taken; the rest stay for the
//! app's key bindings (so Ctrl+S saves while typing).

use lntrn_ui::{Key, KeyPress, Ui, UiState};

use crate::buffer::{Pos, Range};
use crate::doc::Doc;
use crate::editor::ops;
use crate::settings::Settings;
use crate::text_util::{cell_of_byte, byte_at_cell, word_at, word_left, word_right};

enum Edit {
    Key(KeyPress),
    Text(String),
}

/// Whether the editor takes this key (the app's bindings get the rest).
fn handles(k: &KeyPress) -> bool {
    let m = k.mods;
    if m.super_key() {
        return false;
    }
    let (ctrl, shift, alt) = (m.ctrl(), m.shift(), m.alt());
    match k.key {
        Key::Tab => !ctrl && !alt,
        Key::Enter => !alt,
        Key::Backspace | Key::Delete => !alt,
        Key::ArrowLeft | Key::ArrowRight => !alt,
        Key::ArrowUp | Key::ArrowDown => !(alt && ctrl),
        Key::Home | Key::End => !alt,
        Key::PageUp | Key::PageDown => !ctrl && !alt,
        Key::Escape => m.is_empty(),
        Key::Insert => shift && !ctrl,
        Key::Char(c) => ctrl && !alt && matches!(c.to_ascii_lowercase(), 'a' | 'c' | 'x' | 'v' | 'z' | 'y' | 'l' | '[' | ']' | '{' | '}'),
        _ => false,
    }
}

fn take_edits(state: &mut UiState) -> Vec<Edit> {
    let mut out: Vec<(u32, Edit)> = Vec::new();
    state.keys.retain(|k| {
        if handles(k) {
            out.push((k.seq, Edit::Key(*k)));
            false
        } else {
            true
        }
    });
    let alt = state.mods.alt();
    for (seq, t) in state.text_input.drain(..) {
        // Alt+letter is a shortcut, not typing; AltGr letters are not ASCII.
        if alt && t.is_ascii() {
            continue;
        }
        out.push((seq, Edit::Text(t)));
    }
    out.sort_by_key(|(seq, _)| *seq);
    out.into_iter().map(|(_, e)| e).collect()
}

/// Apply this frame's keys and text to `doc`. `page` is how many lines a
/// page holds. Returns whether the text changed.
pub fn handle(ui: &mut Ui, doc: &mut Doc, settings: &Settings, page: usize) -> bool {
    let mut changed = false;
    for e in take_edits(ui.state) {
        let now = ui.state.now;
        match e {
            Edit::Text(t) => {
                ops::type_text(doc, &t, now);
                changed = true;
            }
            Edit::Key(k) => changed |= key(ui, doc, k, settings, page, now),
        }
    }
    changed
}

/// The caret moved `by` lines, keeping its display column.
fn vertical(doc: &mut Doc, by: isize, extend: bool) {
    let tab = doc.tab();
    let n = doc.buffer.line_count() as isize;
    let from = doc.cursor;
    let target = from.line as isize + by;
    if target < 0 {
        doc.set_cursor(Pos::new(0, 0), extend);
        return;
    }
    if target >= n {
        doc.set_cursor(doc.buffer.end(), extend);
        return;
    }
    // Folded lines are skipped, the way they are on screen.
    let mut target = target as usize;
    while doc.is_hidden(target) {
        if by < 0 {
            if target == 0 {
                break;
            }
            target -= 1;
        } else if target + 1 < n as usize {
            target += 1;
        } else {
            break;
        }
    }
    let goal = doc.goal_cell.unwrap_or_else(|| cell_of_byte(doc.buffer.line(from.line), tab, from.col));
    let line = doc.buffer.line(target);
    let col = byte_at_cell(line, tab, goal);
    doc.set_cursor(Pos::new(target, col), extend);
    doc.goal_cell = Some(goal);
}

fn key(ui: &mut Ui, doc: &mut Doc, k: KeyPress, settings: &Settings, page: usize, now: f64) -> bool {
    let (ctrl, shift, alt) = (k.mods.ctrl(), k.mods.shift(), k.mods.alt());
    let sel = doc.selection();
    let cur = doc.cursor;
    let mut changed = true;
    match k.key {
        Key::ArrowLeft | Key::ArrowRight => {
            changed = false;
            let right = k.key == Key::ArrowRight;
            let p = if ctrl {
                let line = doc.buffer.line(cur.line);
                if right {
                    if cur.col >= line.len() { doc.buffer.next_pos(cur) } else { Pos::new(cur.line, word_right(line, cur.col)) }
                } else if cur.col == 0 {
                    doc.buffer.prev_pos(cur)
                } else {
                    Pos::new(cur.line, word_left(line, cur.col))
                }
            } else if doc.has_selection() && !shift {
                if right { sel.end } else { sel.start }
            } else if right {
                doc.buffer.next_pos(cur)
            } else {
                doc.buffer.prev_pos(cur)
            };
            doc.set_cursor(p, shift);
        }
        Key::ArrowUp | Key::ArrowDown => {
            let down = k.key == Key::ArrowDown;
            if alt && shift {
                if down {
                    ops::duplicate_lines(doc, now);
                } else {
                    // Copy upward: the copy lands above, the caret stays put.
                    let (cursor, anchor) = (doc.cursor, doc.anchor);
                    ops::duplicate_lines(doc, now);
                    doc.anchor = anchor;
                    doc.cursor = cursor;
                }
            } else if alt {
                ops::move_lines(doc, down, now);
            } else {
                changed = false;
                vertical(doc, if down { 1 } else { -1 }, shift);
            }
        }
        Key::PageUp | Key::PageDown => {
            changed = false;
            let by = page.max(1) as isize;
            vertical(doc, if k.key == Key::PageDown { by } else { -by }, shift);
        }
        Key::Home => {
            changed = false;
            let p = if ctrl { Pos::new(0, 0) } else { ops::smart_home(doc) };
            doc.set_cursor(p, shift);
        }
        Key::End => {
            changed = false;
            let p = if ctrl { doc.buffer.end() } else { Pos::new(cur.line, doc.buffer.line(cur.line).len()) };
            doc.set_cursor(p, shift);
        }
        Key::Enter => {
            if ctrl {
                ops::insert_line(doc, shift, now);
            } else {
                ops::newline(doc, settings, now);
            }
        }
        Key::Tab => {
            let multi_line = sel.start.line != sel.end.line;
            if shift {
                ops::dedent_lines(doc, settings, now);
            } else if multi_line {
                ops::indent_lines(doc, settings, now);
            } else if settings.insert_spaces {
                let tab = settings.tab();
                let cell = cell_of_byte(doc.buffer.line(sel.start.line), tab, sel.start.col);
                let n = tab - cell % tab;
                doc.insert(&" ".repeat(n), now);
            } else {
                doc.insert("\t", now);
            }
        }
        Key::Backspace => {
            if ctrl {
                ops::delete_word(doc, false, now);
            } else {
                ops::backspace(doc, settings, now);
            }
        }
        Key::Delete => {
            if ctrl {
                ops::delete_word(doc, true, now);
            } else {
                ops::delete_forward(doc, now);
            }
        }
        Key::Escape => {
            changed = false;
            doc.anchor = doc.cursor;
        }
        Key::Insert => paste(ui, doc, now),
        Key::Char(c) => match c.to_ascii_lowercase() {
            'a' => {
                changed = false;
                doc.select_all();
            }
            'c' => {
                changed = false;
                copy(ui, doc, false, now);
            }
            'x' => copy(ui, doc, true, now),
            'v' => paste(ui, doc, now),
            'z' if shift => changed = doc.redo(now),
            'z' => changed = doc.undo(now),
            'y' => changed = doc.redo(now),
            'l' => {
                changed = false;
                ops::select_line(doc);
            }
            '[' | '{' if shift => {
                changed = false;
                doc.fold_at(cur.line);
            }
            ']' | '}' if shift => {
                changed = false;
                doc.unfold_here(cur.line);
            }
            ']' => ops::indent_lines(doc, settings, now),
            '[' => ops::dedent_lines(doc, settings, now),
            _ => changed = false,
        },
        _ => changed = false,
    }
    // Any key that is not a vertical move forgets the remembered column.
    if !matches!(k.key, Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown) {
        doc.goal_cell = None;
    }
    if changed {
        ui.state.request_rebuild = true;
    }
    changed
}

/// Ctrl+C / Ctrl+X: the selection, or the whole line when there is none.
fn copy(ui: &mut Ui, doc: &mut Doc, cut: bool, now: f64) {
    let (text, range) = if doc.has_selection() {
        (doc.selected_text(), doc.selection())
    } else {
        let l = doc.cursor.line;
        let n = doc.buffer.line_count();
        let end = if l + 1 < n { Pos::new(l + 1, 0) } else { Pos::new(l, doc.buffer.line(l).len()) };
        (format!("{}\n", doc.buffer.line(l)), Range::new(Pos::new(l, 0), end))
    };
    if text.is_empty() {
        return;
    }
    ui.state.set_clipboard(text);
    if cut {
        let col = doc.cursor.col;
        let whole_line = !doc.has_selection();
        doc.delete(range, now);
        if whole_line {
            doc.set_cursor(Pos::new(doc.cursor.line, col), false);
        }
    }
}

fn paste(ui: &mut Ui, doc: &mut Doc, now: f64) {
    let text = ui.state.clipboard.clone();
    if text.is_empty() {
        return;
    }
    // Text copied as whole lines pastes as whole lines.
    if text.ends_with('\n') && !doc.has_selection() && !text[..text.len() - 1].contains('\n') {
        let l = doc.cursor.line;
        let col = doc.cursor.col;
        doc.edit(Range::at(Pos::new(l, 0)), &text, crate::doc::EditKind::Other, now);
        doc.set_cursor(Pos::new(l + 1, col), false);
    } else {
        doc.insert(&text, now);
    }
}

/// What a double click selects.
pub fn word_range(doc: &Doc, p: Pos) -> Range {
    let (a, b) = word_at(doc.buffer.line(p.line), p.col);
    Range::new(Pos::new(p.line, a), Pos::new(p.line, b))
}

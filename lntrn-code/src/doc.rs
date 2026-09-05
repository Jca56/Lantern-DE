//! A document: its buffer, caret and selection, undo history, file path
//! and syntax state. Every change goes through [`Doc::edit`], which
//! records an invertible step; typing and deleting runs coalesce so Ctrl+Z
//! steps back a word, not a letter.

use std::io;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Pos, Range};
use crate::syntax::{Highlighter, Language};
use crate::text_util::cell_width;

/// Names a document for as long as it is open; never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DocId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// One invertible change: `removed` was replaced by `inserted` at `start`.
#[derive(Clone, Debug)]
struct EditRec {
    start: Pos,
    removed: String,
    inserted: String,
    /// `(cursor, anchor)` before and after.
    before: (Pos, Pos),
    after: (Pos, Pos),
    at: f64,
    kind: EditKind,
}

/// Seconds within which edits of one kind fold into one undo step.
const COALESCE_SECS: f64 = 1.5;
const UNDO_CAP: usize = 500;

pub struct Doc {
    pub id: DocId,
    pub path: Option<PathBuf>,
    /// The file name, or `Untitled-N`.
    pub title: String,
    pub buffer: Buffer,
    pub cursor: Pos,
    pub anchor: Pos,
    /// The display cell Up and Down aim for, kept across shorter lines.
    pub goal_cell: Option<usize>,
    pub highlight: Highlighter,
    /// Display cells of every line at the current tab width.
    line_cells: Vec<u32>,
    tab: usize,
    undo: Vec<EditRec>,
    redo: Vec<EditRec>,
    saved_version: u64,
    /// Frame time of the last edit (caret blink phase, undo coalescing).
    pub last_edit: f64,
    /// The file changed on disk while this copy has unsaved edits.
    pub disk_changed: bool,
    /// The file is gone from disk.
    pub disk_missing: bool,
}

/// Where `text` ends when inserted at `start`.
fn end_of(start: Pos, text: &str) -> Pos {
    match text.rsplit_once('\n') {
        None => Pos::new(start.line, start.col + text.len()),
        Some((head, last)) => Pos::new(start.line + head.matches('\n').count() + 1, last.len()),
    }
}

impl Doc {
    pub fn from_text(id: DocId, path: Option<PathBuf>, text: &str, tab: usize) -> Self {
        let buffer = Buffer::from_text(text);
        let lang = path.as_deref().map_or(Language::Plain, |p| Language::detect(p, buffer.line(0)));
        let title = path.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".to_owned());
        let line_cells = buffer.lines().iter().map(|l| cell_width(l, tab) as u32).collect();
        let saved_version = buffer.version();
        Self {
            id,
            path,
            title,
            buffer,
            cursor: Pos::default(),
            anchor: Pos::default(),
            goal_cell: None,
            highlight: Highlighter::new(lang),
            line_cells,
            tab,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_version,
            last_edit: 0.0,
            disk_changed: false,
            disk_missing: false,
        }
    }

    pub fn untitled(id: DocId, n: usize, tab: usize) -> Self {
        let mut d = Self::from_text(id, None, "", tab);
        d.title = format!("Untitled-{n}");
        d
    }

    /// Read a file. Bytes that are not UTF-8 are replaced.
    pub fn open(id: DocId, path: &Path, tab: usize) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(Self::from_text(id, Some(path.to_path_buf()), &text, tab))
    }

    /// Give the document a (new) file: the title and language follow.
    pub fn set_path(&mut self, path: PathBuf) {
        self.title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        let lang = Language::detect(&path, self.buffer.line(0));
        self.highlight.set_lang(lang);
        self.path = Some(path);
    }

    pub fn lang(&self) -> Language {
        self.highlight.lang()
    }

    pub fn set_lang(&mut self, lang: Language) {
        self.highlight.set_lang(lang);
    }

    /// Write to `path` (the document's own when `None`). `trim` strips
    /// trailing spaces first, as an ordinary undoable edit.
    pub fn save(&mut self, trim: bool, now: f64) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(io::Error::other("no file name"));
        };
        if trim {
            for i in 0..self.buffer.line_count() {
                let l = self.buffer.line(i);
                let kept = l.trim_end_matches([' ', '\t']).len();
                if kept < l.len() {
                    self.edit(Range::new(Pos::new(i, kept), Pos::new(i, l.len())), "", EditKind::Other, now);
                }
            }
        }
        std::fs::write(&path, self.buffer.to_text())?;
        self.saved_version = self.buffer.version();
        self.disk_changed = false;
        self.disk_missing = false;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.version() != self.saved_version
    }

    /// The buffer version last saved (or loaded).
    pub fn saved_version(&self) -> u64 {
        self.saved_version
    }

    /// The text on disk is now this: one undoable step, then clean.
    pub fn replace_all(&mut self, text: &str, now: f64) {
        let whole = Range::new(Pos::new(0, 0), self.buffer.end());
        let caret = self.cursor;
        self.edit(whole, text, EditKind::Other, now);
        self.set_cursor(caret, false);
        self.saved_version = self.buffer.version();
        self.disk_changed = false;
        self.disk_missing = false;
    }

    pub fn selection(&self) -> Range {
        Range::new(self.cursor, self.anchor)
    }

    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    pub fn selected_text(&self) -> String {
        self.buffer.text_in(self.selection())
    }

    /// Move the caret; `extend` keeps the anchor (a selection).
    pub fn set_cursor(&mut self, p: Pos, extend: bool) {
        self.cursor = self.buffer.clamp(p);
        if !extend {
            self.anchor = self.cursor;
        }
        self.goal_cell = None;
    }

    pub fn select(&mut self, r: Range) {
        self.anchor = self.buffer.clamp(r.start);
        self.cursor = self.buffer.clamp(r.end);
        self.goal_cell = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = Pos::default();
        self.cursor = self.buffer.end();
    }

    /// Replace `r` with `text`, recording the step. The caret lands after
    /// the inserted text. Returns where that is.
    pub fn edit(&mut self, r: Range, text: &str, kind: EditKind, now: f64) -> Pos {
        let r = Range::new(self.buffer.clamp(r.start), self.buffer.clamp(r.end));
        let removed = self.buffer.text_in(r);
        if removed.is_empty() && text.is_empty() {
            return r.start;
        }
        let before = (self.cursor, self.anchor);
        let old_lines = r.end.line - r.start.line + 1;
        let end = self.buffer.replace(r, text);
        self.after_change(r.start.line, old_lines, end.line - r.start.line + 1);
        self.cursor = end;
        self.anchor = end;
        self.goal_cell = None;
        self.last_edit = now;
        let rec = EditRec { start: r.start, removed, inserted: text.to_owned(), before, after: (end, end), at: now, kind };
        self.redo.clear();
        if !self.coalesce(&rec) {
            self.undo.push(rec);
            if self.undo.len() > UNDO_CAP {
                self.undo.remove(0);
            }
        }
        end
    }

    /// Fold `rec` into the last step when it continues it: more typing
    /// right after the last typed text (up to a word boundary), or more
    /// deleting next to the last deletion.
    fn coalesce(&mut self, rec: &EditRec) -> bool {
        let Some(last) = self.undo.last_mut() else {
            return false;
        };
        if last.kind != rec.kind || rec.kind == EditKind::Other || rec.at - last.at > COALESCE_SECS {
            return false;
        }
        match rec.kind {
            EditKind::Typing => {
                let contiguous = rec.removed.is_empty() && last.removed.is_empty() && rec.start == end_of(last.start, &last.inserted);
                let word_break = rec.inserted.starts_with(char::is_whitespace) && !last.inserted.ends_with(char::is_whitespace);
                if !contiguous || word_break || rec.inserted.contains('\n') {
                    return false;
                }
                last.inserted.push_str(&rec.inserted);
            }
            EditKind::Deleting => {
                if !rec.inserted.is_empty() || !last.inserted.is_empty() {
                    return false;
                }
                if end_of(rec.start, &rec.removed) == last.start {
                    // Backspacing: the new deletion sits just before the last.
                    last.removed.insert_str(0, &rec.removed);
                    last.start = rec.start;
                } else if rec.start == last.start {
                    // Delete key: the new deletion follows the last.
                    last.removed.push_str(&rec.removed);
                } else {
                    return false;
                }
            }
            EditKind::Other => return false,
        }
        last.after = rec.after;
        last.at = rec.at;
        true
    }

    /// Replace the selection with `text` (typing).
    pub fn insert(&mut self, text: &str, now: f64) -> Pos {
        let kind = if self.has_selection() { EditKind::Other } else { EditKind::Typing };
        self.edit(self.selection(), text, kind, now)
    }

    pub fn delete(&mut self, r: Range, now: f64) {
        self.edit(r, "", EditKind::Deleting, now);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, now: f64) -> bool {
        let Some(rec) = self.undo.pop() else {
            return false;
        };
        let r = Range::new(rec.start, end_of(rec.start, &rec.inserted));
        let old_lines = r.end.line - r.start.line + 1;
        let end = self.buffer.replace(r, &rec.removed);
        self.after_change(r.start.line, old_lines, end.line - r.start.line + 1);
        self.cursor = self.buffer.clamp(rec.before.0);
        self.anchor = self.buffer.clamp(rec.before.1);
        self.goal_cell = None;
        self.last_edit = now;
        self.redo.push(rec);
        true
    }

    pub fn redo(&mut self, now: f64) -> bool {
        let Some(rec) = self.redo.pop() else {
            return false;
        };
        let r = Range::new(rec.start, end_of(rec.start, &rec.removed));
        let old_lines = r.end.line - r.start.line + 1;
        let end = self.buffer.replace(r, &rec.inserted);
        self.after_change(r.start.line, old_lines, end.line - r.start.line + 1);
        self.cursor = self.buffer.clamp(rec.after.0);
        self.anchor = self.buffer.clamp(rec.after.1);
        self.goal_cell = None;
        self.last_edit = now;
        self.undo.push(rec);
        true
    }

    fn after_change(&mut self, start_line: usize, old_lines: usize, new_lines: usize) {
        let tab = self.tab;
        let fresh = (start_line..start_line + new_lines).map(|i| cell_width(self.buffer.line(i), tab) as u32);
        let fresh: Vec<u32> = fresh.collect();
        let end = (start_line + old_lines).min(self.line_cells.len());
        self.line_cells.splice(start_line..end, fresh);
        self.highlight.invalidate_from(start_line);
    }

    /// Display cells of line `i`.
    pub fn line_cells(&self, i: usize) -> usize {
        self.line_cells.get(i).copied().unwrap_or(0) as usize
    }

    /// The widest line, in cells.
    pub fn max_cells(&self) -> usize {
        self.line_cells.iter().copied().max().unwrap_or(0) as usize
    }

    pub fn tab(&self) -> usize {
        self.tab
    }

    /// Tab width changed: line widths follow.
    pub fn set_tab(&mut self, tab: usize) {
        if tab != self.tab {
            self.tab = tab;
            self.line_cells = self.buffer.lines().iter().map(|l| cell_width(l, tab) as u32).collect();
        }
    }

    /// The tab-width-aware text of a line.
    pub fn line(&self, i: usize) -> &str {
        self.buffer.line(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Doc {
        Doc::from_text(DocId(1), None, text, 4)
    }

    #[test]
    fn typing_coalesces_into_words() {
        let mut d = doc("");
        for (i, c) in ["h", "e", "y", " ", "y", "o"].iter().enumerate() {
            d.insert(c, i as f64 * 0.1);
        }
        assert_eq!(d.buffer.line(0), "hey yo");
        assert!(d.undo(1.0));
        assert_eq!(d.buffer.line(0), "hey", "one word undone");
        assert!(d.undo(1.1));
        assert_eq!(d.buffer.line(0), "");
        assert!(d.redo(1.2));
        assert_eq!(d.buffer.line(0), "hey");
        assert!(d.redo(1.3));
        assert_eq!(d.buffer.line(0), "hey yo");
        assert!(!d.redo(1.4));
        assert!(d.is_dirty());
    }

    #[test]
    fn deleting_coalesces_both_ways() {
        let mut d = doc("abcdef");
        d.set_cursor(Pos::new(0, 3), false);
        // Two backspaces, then two deletes.
        d.delete(Range::new(Pos::new(0, 2), Pos::new(0, 3)), 0.0);
        d.delete(Range::new(Pos::new(0, 1), Pos::new(0, 2)), 0.1);
        assert_eq!(d.buffer.line(0), "adef");
        d.delete(Range::new(Pos::new(0, 1), Pos::new(0, 2)), 0.2);
        d.delete(Range::new(Pos::new(0, 1), Pos::new(0, 2)), 0.3);
        assert_eq!(d.buffer.line(0), "af");
        assert!(d.undo(1.0));
        assert_eq!(d.buffer.line(0), "abcdef", "all four deletions were one step");
        assert_eq!(d.cursor, Pos::new(0, 3), "caret goes back to where it was");
    }

    #[test]
    fn multi_line_edit_keeps_widths() {
        let mut d = doc("one\ntwo\nthree");
        assert_eq!(d.max_cells(), 5);
        d.edit(Range::new(Pos::new(0, 1), Pos::new(2, 2)), "X\n\tY", EditKind::Other, 0.0);
        assert_eq!(d.buffer.lines(), &["oX", "\tYree"]);
        assert_eq!(d.line_cells(1), 8);
        assert_eq!(d.max_cells(), 8);
        d.undo(1.0);
        assert_eq!(d.buffer.lines(), &["one", "two", "three"]);
        assert_eq!(d.max_cells(), 5);
        d.set_tab(2);
        assert_eq!(d.line_cells(0), 3);
    }
}

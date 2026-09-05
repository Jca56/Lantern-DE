//! Applying what a server wants changed: edits to a document in the
//! server's columns, last first so the earlier ones stay where they
//! were; the same to a file that is not open; and a whole workspace
//! edit with its file operations.

use std::cmp::Reverse;
use std::path::Path;

use crate::app::App;
use crate::buffer::{Pos, Range};
use crate::doc::{Doc, DocId, EditKind};
use crate::lsp::pos::from_units;
use crate::lsp::{Change, TextEdit, WorkspaceEdit};

/// A server's edit as a range in the document, `line_shift` lines down.
pub fn range_of(doc: &Doc, e: &TextEdit, utf16: bool, line_shift: usize) -> Range {
    let n = doc.buffer.line_count();
    let l0 = (e.line + line_shift).min(n - 1);
    let l1 = (e.end_line + line_shift).min(n - 1);
    Range::new(Pos::new(l0, from_units(doc.line(l0), e.col, utf16)), Pos::new(l1, from_units(doc.line(l1), e.end_col, utf16)))
}

/// A server's edit as the range and text to put in the document. A
/// position past the last line means "after the final newline", which
/// the buffer keeps as a flag rather than a line: such an end lands at
/// the end of the last line and gives up the text's own final newline,
/// such a start also puts a newline first.
fn fit(doc: &Doc, e: &TextEdit, utf16: bool) -> (Range, String) {
    let n = doc.buffer.line_count();
    let last = n.saturating_sub(1);
    let last_end = Pos::new(last, doc.line(last).len());
    let start_virtual = e.line >= n;
    let end_virtual = e.end_line >= n;
    let start = if start_virtual { last_end } else { Pos::new(e.line, from_units(doc.line(e.line), e.col, utf16)) };
    let end = if end_virtual { last_end } else { Pos::new(e.end_line, from_units(doc.line(e.end_line), e.end_col, utf16)) };
    let mut text = e.text.clone();
    if end_virtual && let Some(t) = text.strip_suffix('\n') {
        text.truncate(t.len());
    }
    if start_virtual {
        text.insert(0, '\n');
    }
    (Range::new(start, end), text)
}

/// Put `edits` into `doc`, last first so earlier positions hold; the
/// caret stays where it was as far as the text allows. Returns how many.
pub fn apply_edits(doc: &mut Doc, edits: &[TextEdit], utf16: bool, now: f64) -> usize {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| Reverse((e.line, e.col, e.end_line, e.end_col)));
    let cursor = doc.cursor;
    for e in &sorted {
        let (r, text) = fit(doc, e, utf16);
        doc.edit(r, &text, EditKind::Other, now);
    }
    let line = cursor.line.min(doc.buffer.line_count().saturating_sub(1));
    let text = doc.line(line);
    let mut col = cursor.col.min(text.len());
    while !text.is_char_boundary(col) {
        col -= 1;
    }
    doc.set_cursor(Pos::new(line, col), false);
    sorted.len()
}

/// The same for a file on disk, through a document of its own.
pub fn apply_to_file(path: &Path, edits: &[TextEdit], utf16: bool) -> std::io::Result<usize> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut doc = Doc::from_text(DocId(0), None, &text, 4);
    let n = apply_edits(&mut doc, edits, utf16, 0.0);
    std::fs::write(path, doc.buffer.to_text())?;
    Ok(n)
}

/// What applying a workspace edit came to.
#[derive(Debug, Default)]
pub struct Applied {
    pub edits: usize,
    pub files: usize,
    pub errors: Vec<String>,
}

impl Applied {
    /// A line for a toast: `Renamed: 12 edits in 3 files`.
    pub fn summary(&self, verb: &str) -> String {
        let mut s = format!("{verb} {} edit{} in {} file{}", self.edits, if self.edits == 1 { "" } else { "s" }, self.files, if self.files == 1 { "" } else { "s" });
        if !self.errors.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.errors.join("; "));
        }
        s
    }
}

impl App {
    /// Open documents' paths follow a file (or folder) moved from `from`
    /// to `to`.
    pub(crate) fn retarget_docs(&mut self, from: &Path, to: &Path) {
        for d in &mut self.docs {
            if d.path.as_deref() == Some(from) {
                d.set_path(to.to_path_buf());
            } else if let Some(rest) = d.path.as_ref().and_then(|p| p.strip_prefix(from).ok()).map(Path::to_path_buf) {
                d.set_path(to.join(rest));
            }
        }
        self.session_dirty = true;
    }

    /// Make a server's changes: open documents in place, other files on
    /// disk, files created, renamed and deleted as asked.
    pub(crate) fn apply_workspace_edit(&mut self, edit: WorkspaceEdit, now: f64) -> Applied {
        let mut out = Applied::default();
        let utf16 = edit.utf16;
        let mut touched_tree = false;
        for change in edit.changes {
            match change {
                Change::Edits(path, edits) => {
                    if edits.is_empty() {
                        continue;
                    }
                    let n = match self.doc_by_path(&path) {
                        Some(i) => Ok(apply_edits(&mut self.docs[i], &edits, utf16, now)),
                        None => apply_to_file(&path, &edits, utf16),
                    };
                    match n {
                        Ok(n) => {
                            out.edits += n;
                            out.files += 1;
                        }
                        Err(e) => out.errors.push(format!("{}: {e}", path.display())),
                    }
                }
                Change::Create(path) => {
                    let made = path.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|()| if path.exists() { Ok(()) } else { std::fs::write(&path, "") });
                    match made {
                        Ok(()) => out.files += 1,
                        Err(e) => out.errors.push(format!("create {}: {e}", path.display())),
                    }
                    touched_tree = true;
                }
                Change::Rename(from, to) => {
                    match std::fs::rename(&from, &to) {
                        Ok(()) => {
                            self.retarget_docs(&from, &to);
                            out.files += 1;
                        }
                        Err(e) => out.errors.push(format!("rename {}: {e}", from.display())),
                    }
                    touched_tree = true;
                }
                Change::Delete(path) => {
                    let gone = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                    match gone {
                        Ok(()) => {
                            let closing: Vec<DocId> = self.docs.iter().filter(|d| d.path.as_ref().is_some_and(|p| p == &path || p.starts_with(&path))).map(|d| d.id).collect();
                            self.pending_close.extend(closing);
                            out.files += 1;
                        }
                        Err(e) => out.errors.push(format!("delete {}: {e}", path.display())),
                    }
                    touched_tree = true;
                }
            }
        }
        if touched_tree && let Some(p) = self.project.as_mut() {
            p.refresh();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_back_to_front_keep_the_caret() {
        let mut doc = Doc::from_text(DocId(1), None, "alpha beta\ngamma\n", 4);
        doc.set_cursor(Pos::new(1, 3), false);
        let edits = vec![
            TextEdit { line: 0, col: 0, end_line: 0, end_col: 5, text: "A".into() },
            TextEdit { line: 0, col: 6, end_line: 0, end_col: 10, text: "BETA".into() },
            TextEdit { line: 1, col: 0, end_line: 1, end_col: 0, text: "> ".into() },
        ];
        assert_eq!(apply_edits(&mut doc, &edits, false, 0.0), 3);
        assert_eq!(doc.buffer.to_text(), "A BETA\n> gamma\n", "given in order, applied from the end");
        assert_eq!(doc.cursor, Pos::new(1, 3), "the caret keeps its place");
        let whole = vec![TextEdit { line: 0, col: 0, end_line: 2, end_col: 0, text: "x\n".into() }];
        apply_edits(&mut doc, &whole, false, 0.0);
        assert_eq!(doc.buffer.to_text(), "x\n", "a replace up to after the final newline");
        assert_eq!(doc.cursor, Pos::new(0, 1), "clamped into what is left");
        let append = vec![TextEdit { line: 1, col: 0, end_line: 1, end_col: 0, text: "tail\n".into() }];
        apply_edits(&mut doc, &append, false, 0.0);
        assert_eq!(doc.buffer.to_text(), "x\ntail\n", "an insert after the final newline is a new last line");
    }

    #[test]
    fn files_on_disk() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-edits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "one\ntwo\n").unwrap();
        let n = apply_to_file(&p, &[TextEdit { line: 1, col: 0, end_line: 1, end_col: 3, text: "2".into() }], false).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\n2\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

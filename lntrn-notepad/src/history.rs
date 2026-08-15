//! Undo/redo history — full-document snapshots, split out of `editor.rs`
//! so that file stays focused on state + editing ops.

use crate::editor::{Editor, Pos};
use crate::format::DocFormats;

/// Undo/redo snapshot.
#[derive(Clone)]
pub(crate) struct Snapshot {
    lines: Vec<String>,
    formats: DocFormats,
    cursor: Pos,
    sel_anchor: Option<Pos>,
}

const MAX_UNDO: usize = 200;

impl Editor {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            formats: self.formats.clone(),
            cursor: Pos::new(self.cursor_line, self.cursor_col),
            sel_anchor: self.sel_anchor,
        }
    }

    pub fn push_undo(&mut self) {
        let snap = self.snapshot();
        self.undo_stack.push(snap);
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some(snap) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());
            self.restore(snap);
        }
    }

    pub fn redo(&mut self) {
        if let Some(snap) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());
            self.restore(snap);
        }
    }

    /// Drop all undo/redo history (fresh file loads).
    pub(crate) fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    fn restore(&mut self, snap: Snapshot) {
        self.lines = snap.lines;
        self.formats = snap.formats;
        self.cursor_line = snap.cursor.line;
        self.cursor_col = snap.cursor.col;
        self.sel_anchor = snap.sel_anchor;
        self.modified = true;
    }
}

//! Rename + path-bar editing + save-name buffer editing.

use super::App;

/// A typed name must be exactly one ordinary path component. ".", ".." and
/// anything holding a '/' would make `parent.join(name)` point at another
/// folder (or the parent itself), and the conflict dialog's Replace would
/// then delete that.
pub(crate) fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// True when both paths are the same directory entry on disk — a rename that
/// only changes letter case on a case-insensitive filesystem (vfat, exfat).
/// `symlink_metadata` on purpose: a link and its target are different entries.
pub(crate) fn is_same_entry(a: &std::path::Path, b: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// The rename and save-name buffers keep BYTE cursors (start_rename's
/// `rfind('.')` and `len()` are bytes). Every step must land on a char
/// boundary or `String::remove`/`insert` panics on a name like "Café.txt".
pub(crate) fn floor_boundary(buf: &str, cursor: usize) -> usize {
    let mut i = cursor.min(buf.len());
    while !buf.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Byte offset of the char that ends at `cursor` (0 at the start).
pub(crate) fn prev_boundary(buf: &str, cursor: usize) -> usize {
    let cursor = floor_boundary(buf, cursor);
    buf[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(i, _)| i)
}

/// Byte offset just past the char that starts at `cursor` (len at the end).
pub(crate) fn next_boundary(buf: &str, cursor: usize) -> usize {
    let cursor = floor_boundary(buf, cursor);
    buf[cursor..]
        .chars()
        .next()
        .map_or(buf.len(), |c| cursor + c.len_utf8())
}

/// The text widget counts in chars; convert a byte offset for it.
fn char_offset(buf: &str, byte: usize) -> usize {
    buf[..floor_boundary(buf, byte)].chars().count()
}

impl App {
    /// Rename cursor and selection in chars, for the text widget.
    pub fn rename_cursor_chars(&self) -> usize {
        char_offset(&self.rename_buf, self.rename_cursor)
    }

    pub fn rename_selection_chars(&self) -> Option<(usize, usize)> {
        self.rename_selection
            .map(|(a, b)| (char_offset(&self.rename_buf, a), char_offset(&self.rename_buf, b)))
    }

    /// Save-name cursor and selection in chars, for the text widget.
    pub fn save_name_cursor_chars(&self) -> usize {
        char_offset(&self.save_name_buf, self.save_name_cursor)
    }

    pub fn save_name_selection_chars(&self) -> Option<(usize, usize)> {
        self.save_name_selection.map(|(a, b)| {
            (
                char_offset(&self.save_name_buf, a),
                char_offset(&self.save_name_buf, b),
            )
        })
    }

    // ── Rename ────────────────────────────────────────────────────────

    pub fn start_rename(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        self.rename_buf = self.entries[index].name.clone();
        // Files: select the basename (everything before the final '.'). If
        // there's no extension or it's a dotfile, select all. Folders: select all.
        // Selection and cursor use byte offsets — same units the rest of the
        // rename input uses.
        let select_end = if self.entries[index].is_dir {
            self.rename_buf.len()
        } else {
            match self.rename_buf.rfind('.') {
                Some(0) | None => self.rename_buf.len(),
                Some(dot) => dot,
            }
        };
        self.rename_selection = if select_end > 0 {
            Some((0, select_end))
        } else {
            None
        };
        self.rename_cursor = select_end;
        self.renaming = Some(index);
    }

    pub fn commit_rename(&mut self) {
        if let Some(idx) = self.renaming.take() {
            if idx < self.entries.len() && is_plain_file_name(&self.rename_buf) {
                let old = self.entries[idx].path.clone();
                let new_path = old.parent().unwrap_or(&old).join(&self.rename_buf);
                if new_path != old {
                    // Pop the shared conflict dialog if we'd clobber a real
                    // existing entry (case-insensitive renames on the same
                    // file are allowed through — fs::rename handles those).
                    if new_path.exists() && !self.root_mode && !is_same_entry(&old, &new_path) {
                        let dialog = crate::conflict::ConflictDialog {
                            target: new_path.clone(),
                            source_meta: crate::conflict::ConflictMeta::read(&old),
                            target_meta: crate::conflict::ConflictMeta::read(&new_path),
                            apply_to_all: false,
                            remaining_count: 0,
                        };
                        self.pending_rename = Some(crate::conflict::PendingRename {
                            from: old,
                            to: new_path,
                        });
                        self.conflict_dialog = Some(dialog);
                        self.rename_buf.clear();
                        self.rename_cursor = 0;
                        self.rename_selection = None;
                        return;
                    }
                    self.perform_rename(old, new_path);
                }
            }
            self.rename_buf.clear();
            self.rename_cursor = 0;
            self.rename_selection = None;
            self.reload();
        }
    }

    /// Execute a rename + push an Undo entry. Used by both the normal commit
    /// path and the conflict-resolved Replace / Keep Both paths.
    pub(crate) fn perform_rename(&mut self, from: std::path::PathBuf, to: std::path::PathBuf) {
        if self.root_mode {
            let from_cmd = from.clone();
            let to_cmd = to.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("pkexec")
                    .args(["mv", "--"])
                    .arg(&from_cmd)
                    .arg(&to_cmd)
                    .status();
            });
        } else if let Err(e) = std::fs::rename(&from, &to) {
            // No undo entry for a rename that never happened.
            eprintln!("[fox] rename {} failed: {e}", from.display());
            return;
        }
        self.undo_stack
            .push(crate::undo::UndoAction::Rename { from, to });
    }

    pub fn cancel_rename(&mut self) {
        self.renaming = None;
        self.rename_buf.clear();
        self.rename_cursor = 0;
        self.rename_selection = None;
    }

    /// Delete the currently selected text in the save-name buffer (if any).
    /// Returns true if anything was deleted. Mirrors `rename_delete_selection`.
    pub fn save_name_delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.save_name_selection.take() else {
            return false;
        };
        let start = floor_boundary(&self.save_name_buf, a.min(b));
        let end = floor_boundary(&self.save_name_buf, a.max(b));
        if start == end {
            return false;
        }
        self.save_name_buf.replace_range(start..end, "");
        self.save_name_cursor = start;
        true
    }

    /// Delete the currently selected text in the rename buffer (if any).
    /// Returns true if anything was deleted. Cursor lands at the start of the
    /// former selection. Selection offsets are byte indices.
    pub fn rename_delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.rename_selection.take() else {
            return false;
        };
        let start = floor_boundary(&self.rename_buf, a.min(b));
        let end = floor_boundary(&self.rename_buf, a.max(b));
        if start == end {
            return false;
        }
        self.rename_buf.replace_range(start..end, "");
        self.rename_cursor = start;
        true
    }

    // ── Path bar editing ──────────────────────────────────────────────

    pub fn start_path_edit(&mut self) {
        self.path_buf = self.current_dir.to_string_lossy().to_string();
        self.path_cursor = self.path_buf.chars().count();
        self.path_selection = None;
        self.path_editing = true;
    }

    pub fn commit_path_edit(&mut self) {
        let path = self.resolve_typed_path(&self.path_buf);
        if path.is_dir() {
            self.navigate_to(path);
        }
        self.path_editing = false;
        self.path_buf.clear();
        self.path_cursor = 0;
        self.path_selection = None;
    }

    /// Turn what was typed into the path bar into an absolute, normalised
    /// path: `~` is home, a relative path hangs off the shown folder, and
    /// `.` / `..` are folded away. Lexical on purpose — `canonicalize` would
    /// jump a symlinked folder to its real location (and block on a slow
    /// mount). Breadcrumbs, Up and the slow-path checks all assume this shape.
    fn resolve_typed_path(&self, typed: &str) -> std::path::PathBuf {
        use std::path::{Component, Path, PathBuf};
        let typed = typed.trim();
        let start = if typed == "~" {
            super::dirs_home()
        } else if let Some(rest) = typed.strip_prefix("~/") {
            super::dirs_home().join(rest)
        } else if Path::new(typed).is_absolute() {
            PathBuf::from(typed)
        } else {
            self.current_dir.join(typed)
        };
        let mut out = PathBuf::new();
        for comp in start.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other),
            }
        }
        out
    }

    pub fn cancel_path_edit(&mut self) {
        self.path_editing = false;
        self.path_buf.clear();
        self.path_cursor = 0;
        self.path_selection = None;
    }

    /// Get the currently selected text in the path bar, or the full path if all selected.
    pub fn path_selected_text(&self) -> Option<String> {
        let (start, end) = self.path_selection?;
        if start == end {
            return None;
        }
        let s = start.min(end);
        let e = start.max(end);
        let text: String = self.path_buf.chars().skip(s).take(e - s).collect();
        Some(text)
    }
}

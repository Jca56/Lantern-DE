//! Rename + path-bar editing + save-name buffer editing.

use super::App;

impl App {
    // ── Rename ────────────────────────────────────────────────────────

    pub fn start_rename(&mut self, index: usize) {
        if index >= self.entries.len() { return; }
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
        self.rename_selection = if select_end > 0 { Some((0, select_end)) } else { None };
        self.rename_cursor = select_end;
        self.renaming = Some(index);
    }

    pub fn commit_rename(&mut self) {
        if let Some(idx) = self.renaming.take() {
            if idx < self.entries.len() && !self.rename_buf.is_empty() {
                let old = &self.entries[idx].path;
                let new_path = old.parent().unwrap_or(old).join(&self.rename_buf);
                if new_path != *old {
                    let old_clone = old.clone();
                    if self.root_mode {
                        let old = old.clone();
                        let new_path = new_path.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("pkexec")
                                .args(["mv", "--"])
                                .arg(&old).arg(&new_path)
                                .status();
                        });
                    } else {
                        let _ = std::fs::rename(old, &new_path);
                    }
                    self.undo_stack.push(crate::undo::UndoAction::Rename {
                        from: old_clone, to: new_path,
                    });
                }
            }
            self.rename_buf.clear();
            self.rename_cursor = 0;
            self.rename_selection = None;
            self.reload();
        }
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
        let Some((a, b)) = self.save_name_selection.take() else { return false };
        let start = a.min(b).min(self.save_name_buf.len());
        let end = a.max(b).min(self.save_name_buf.len());
        if start == end { return false; }
        self.save_name_buf.replace_range(start..end, "");
        self.save_name_cursor = start;
        true
    }

    /// Delete the currently selected text in the rename buffer (if any).
    /// Returns true if anything was deleted. Cursor lands at the start of the
    /// former selection. Selection offsets are byte indices.
    pub fn rename_delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.rename_selection.take() else { return false };
        let start = a.min(b).min(self.rename_buf.len());
        let end = a.max(b).min(self.rename_buf.len());
        if start == end { return false; }
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
        let path = std::path::PathBuf::from(&self.path_buf);
        if path.is_dir() {
            self.navigate_to(path);
        }
        self.path_editing = false;
        self.path_buf.clear();
        self.path_cursor = 0;
        self.path_selection = None;
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
        if start == end { return None; }
        let s = start.min(end);
        let e = start.max(end);
        let text: String = self.path_buf.chars().skip(s).take(e - s).collect();
        Some(text)
    }
}

//! What the user does with the Trash: move to it, restore from it, delete
//! for good, empty it. The mechanics are in `trash`; this is the App side.
//!
//! The Trash view is the home trash's `files/` folder, into which
//! `fs::list_directory` also lists every drive's own trash, so "the Trash"
//! is one place however many volumes are mounted. Each entry keeps its real
//! path, which is how restore finds the right trash and record again.

use std::path::PathBuf;

use crate::app::App;
use crate::ops::{OpFailure, OpItem, OpKind, OpRequest};

/// The entries of `dir`, but only when `dir` itself is a real directory.
/// A `files` or `info` that is a link would hand the delete whatever the
/// link points at (a drive someone else prepared can carry one).
fn entries_of_real_dir(dir: &std::path::Path) -> Vec<PathBuf> {
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return Vec::new();
    }
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

impl App {
    /// True if the current directory is a trash (any volume's) or a real
    /// folder inside one. Not below a trashed link to a folder: the rows
    /// shown there are the live files the link points at.
    pub fn in_trash(&self) -> bool {
        crate::trash::shows_trash(&self.current_dir)
    }

    /// Restore every selected item (or the row the context menu was opened
    /// on) to where its record says it came from. A name that is taken
    /// there gets a " (restored N)" suffix.
    pub fn restore_selected(&mut self) {
        use crate::trash::RestoreError;
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut failures: Vec<OpFailure> = Vec::new();
        // Items whose original place is on another drive than their trash.
        let mut far: Vec<OpItem> = Vec::new();
        let mut restored = 0usize;
        for path in self.selected_paths() {
            if !crate::trash::is_trashed(&path) {
                continue;
            }
            // An item inside a trashed folder restores that whole folder;
            // two of them from the same folder must not try it twice.
            let Some(loc) = crate::trash::locate(&path) else {
                continue;
            };
            let Some(top) = loc.rel.components().next() else {
                continue;
            };
            let key = loc.trash.files().join(top);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            match crate::trash::restore_item(&path) {
                Ok(_) => restored += 1,
                Err(RestoreError::OtherDrive { trashed, dest }) => far.push(OpItem {
                    src: trashed,
                    target: dest,
                    replace: false,
                }),
                Err(RestoreError::Failed(reason)) => failures.push(OpFailure { path, reason }),
            }
        }
        if restored > 0 {
            eprintln!("[fox] restored {restored} item(s) from trash");
        }
        if !far.is_empty() {
            // A rename cannot cross drives: the worker copies each item
            // back, checks the copy and only then takes it out of the
            // Trash. `finish_op` drops the restore record when it is out.
            self.ops.push(OpRequest {
                kind: OpKind::Move,
                label: "Restoring",
                items: far,
                dest: self.current_dir.clone(),
                rearm_clipboard: None,
            });
        }
        if !failures.is_empty() {
            self.report_failures("restored", &failures);
        }
        self.reload();
    }

    /// The Delete key, and the menu's "Move to Trash" / "Delete
    /// Permanently", on the selection or the row the menu was opened on.
    ///
    /// An item that already is in a trash cannot be trashed again (it used
    /// to be renamed inside the Trash and lose its restore record): for it
    /// this is a permanent delete, and that always asks first. Everything
    /// else goes to the Trash of its drive. Root mode is no exception:
    /// what can be trashed is trashed, and what cannot is asked about
    /// before sudo deletes it.
    pub fn trash_selected(&mut self) {
        // `is_trashed`, not the shape of the path: a row below a trashed
        // link to a folder is a live file, and "delete permanently" on it
        // would delete the real thing while saying it leaves the Trash.
        let (in_trash, live): (Vec<PathBuf>, Vec<PathBuf>) = self
            .selected_paths()
            .into_iter()
            .partition(|p| crate::trash::is_trashed(p));
        if !live.is_empty() {
            self.trash_paths(live);
        }
        if !in_trash.is_empty() {
            self.confirm_delete_permanently(in_trash);
        }
    }

    /// Items dropped on the Trash (its sidebar place, a tab or pane showing
    /// it, a folder inside it): they are trashed, restore record and undo
    /// entry included. Items that already are in a trash stay as they are.
    pub fn drop_on_trash(&mut self, sources: Vec<PathBuf>) {
        let live: Vec<PathBuf> = sources
            .into_iter()
            .filter(|p| !crate::trash::is_trashed(p))
            .collect();
        if !live.is_empty() {
            self.trash_paths(live);
        }
    }

    /// Move `paths` to the Trash of their drive, with one undo entry for
    /// those that went. For the rest (no Trash there, or not permitted) Fox
    /// says why and asks before anything is deleted for good.
    ///
    /// Callers pass items that are not in a trash already (`trash_selected`
    /// and `drop_on_trash` sort those out).
    pub fn trash_paths(&mut self, paths: Vec<PathBuf>) {
        use crate::trash::TrashError;
        let mut undo_entries = Vec::new();
        let mut stuck: Vec<(PathBuf, String)> = Vec::new();
        let mut mounted: Vec<OpFailure> = Vec::new();
        for path in paths {
            match crate::trash::trash(&path) {
                Ok(t) => undo_entries.push((t.original, t.trashed, t.info)),
                // Never offered for permanent delete: that would empty the
                // drive, phone or share mounted there.
                Err(TrashError::Mounted) => mounted.push(OpFailure {
                    path,
                    reason: crate::mount_guard::REASON.into(),
                }),
                Err(e) => stuck.push((path, e.reason())),
            }
        }
        if !undo_entries.is_empty() {
            self.undo_stack
                .push(crate::undo::UndoAction::Trash(undo_entries));
        }
        self.report_failures("moved to the Trash", &mounted);
        if !stuck.is_empty() {
            self.ask_delete_permanently(stuck);
        }
        self.reload();
    }

    /// Permanently delete every item in every trash (the home one and each
    /// mounted drive's): the contents of `files/` and the records in
    /// `info/`. Asks first, naming what is in there. Then runs on the ops
    /// worker; root-owned leftovers go through the sudo flow when it ends,
    /// which asks again before it deletes anything as root.
    pub fn empty_trash(&mut self) {
        let mut items: Vec<PathBuf> = Vec::new();
        let mut records: Vec<PathBuf> = Vec::new();
        for td in crate::trash::all_trash_dirs() {
            items.extend(entries_of_real_dir(&td.files()));
            // Records and the spec's size cache are plain files. Anything
            // else found in their place is left alone: it is not ours, and
            // a folder there would be deleted with everything in it.
            let plain_file =
                |p: &PathBuf| std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir());
            // Only the records that have no item: an item takes its own
            // record with it when it is deleted (`trash::forget`), and one
            // that could not be deleted (root-owned, a mount still on it)
            // keeps its record, or it could never be restored.
            let files = td.files();
            let orphan = |p: &PathBuf| {
                p.file_stem()
                    .is_some_and(|item| std::fs::symlink_metadata(files.join(item)).is_err())
            };
            records.extend(
                entries_of_real_dir(&td.info())
                    .into_iter()
                    .filter(|p| p.extension().is_some_and(|e| e == "trashinfo"))
                    .filter(plain_file)
                    .filter(orphan),
            );
            records.extend(Some(td.dir.join("directorysizes")).filter(plain_file));
        }
        if items.is_empty() {
            // Only stray records (or nothing at all): no question to ask.
            self.delete_permanently(records, "Emptying Trash");
        } else {
            // Items before records: deleting an item drops its own record.
            let mut paths = items.clone();
            paths.extend(records);
            self.confirm_empty_trash(&items, paths);
        }
    }

    /// Permanent delete, on the ops worker (a big tree or a phone must not
    /// freeze the window). The caller has made sure this is what the user
    /// asked for. Failures are reported when it ends; permission failures
    /// go to the sudo prompt.
    pub fn delete_permanently(&mut self, paths: Vec<PathBuf>, label: &'static str) {
        self.ops.push(OpRequest {
            kind: OpKind::Delete,
            label,
            items: paths.into_iter().map(OpItem::delete).collect(),
            dest: self.current_dir.clone(),
            rearm_clipboard: None,
        });
    }
}

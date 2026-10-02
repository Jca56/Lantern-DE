//! Actions on rows that work from the paths they were asked for, not from
//! the folder being shown: Duplicate, Compress and Open as Root. A row
//! inside an expanded folder of the tree view is not in `App::entries`; its
//! path comes through `selected_paths()` (the row the context menu was
//! opened on), and everything here derives the working folder from that
//! path. (Trash, Restore and Extract follow the same rule in `trash_ops.rs`
//! and `file_ops.rs`.)

use std::path::{Path, PathBuf};

use crate::app::App;
use crate::ops::{OpItem, OpKind, OpRequest};

/// What goes into an archive made in `dir`: the names of the selected items
/// that live there. tar runs in `dir` and names its members relative to it,
/// so an item from somewhere else has no name to give and is left out.
fn archive_members(dir: &Path, selected: &[PathBuf]) -> Vec<std::ffi::OsString> {
    selected
        .iter()
        .filter(|p| p.parent() == Some(dir))
        .filter_map(|p| p.file_name().map(|n| n.to_os_string()))
        .collect()
}

/// The folder "Open as Root" goes into: the first target that is a folder.
/// A file has none: Fox cannot open a file with root privileges, so the
/// menu does not offer it there and this does nothing for one. `is_dir`
/// answers from the listing; `None` for a path it does not know.
fn root_folder(targets: &[PathBuf], is_dir: impl Fn(&Path) -> Option<bool>) -> Option<PathBuf> {
    targets.iter().find(|p| is_dir(p) == Some(true)).cloned()
}

/// The archive Compress makes in `dir`: `base.tar.gz`, or `base (2).tar.gz`,
/// `base (3).tar.gz`... while `taken` claims the name.
fn free_archive_name(dir: &Path, base: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let mut archive = dir.join(format!("{base}.tar.gz"));
    let mut counter = 2u32;
    while taken(&archive) && counter < 10_000 {
        archive = dir.join(format!("{base} ({counter}).tar.gz"));
        counter += 1;
    }
    archive
}

impl App {
    /// The Trash holds items and their restore records in pairs: anything
    /// made inside it by hand (a duplicate, an archive, an unpacked folder)
    /// would be an item nothing can restore. True (and a notice) when
    /// `paths` are in a trash and the action `what` must not run.
    pub(crate) fn refuse_in_trash(&mut self, paths: &[PathBuf], what: &str) -> bool {
        if !paths.iter().any(|p| crate::trash::locate(p).is_some()) {
            return false;
        }
        self.show_notice(
            format!("Items in the Trash can\u{2019}t be {what}"),
            vec!["Restore the item first, then try again.".into()],
        );
        true
    }

    /// Duplicate the selection (or the row the context menu was opened on)
    /// next to itself ("name (copy).ext"). Runs on the ops worker like a
    /// paste: progress, cancel, and an undo entry for exactly what was
    /// created, recorded when it has been created.
    pub fn duplicate_selected(&mut self) {
        let selected = self.selected_paths();
        if selected.is_empty() || self.refuse_in_trash(&selected, "duplicated") {
            return;
        }
        // Next to the first item: for a row inside an expanded folder that
        // is that folder, not the one being shown.
        let folder = selected[0]
            .parent()
            .unwrap_or(&self.current_dir)
            .to_path_buf();
        let mut items: Vec<OpItem> = Vec::new();
        for path in selected {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let parent = path.parent().unwrap_or(&self.current_dir).to_path_buf();
            let stem = std::path::Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let ext = std::path::Path::new(&name)
                .extension()
                .map(|s| format!(".{}", s.to_string_lossy()))
                .unwrap_or_default();

            // Taken: on disk, by an earlier item of this batch, or by an
            // operation that has not created it yet.
            let taken = |p: &Path| {
                std::fs::symlink_metadata(p).is_ok()
                    || items.iter().any(|i| i.target == p)
                    || self.ops.is_reserved(p)
            };
            let mut dest = parent.join(format!("{stem} (copy){ext}"));
            let mut counter = 2u32;
            while taken(&dest) {
                dest = parent.join(format!("{stem} (copy {counter}){ext}"));
                counter += 1;
            }
            items.push(OpItem {
                src: path,
                target: dest,
                replace: false,
            });
        }
        if items.is_empty() {
            return;
        }
        if self.root_covers(&folder) {
            self.run_privileged_items(true, &items);
            return;
        }
        self.ops.push(OpRequest {
            kind: OpKind::Copy,
            label: "Duplicating",
            items,
            dest: folder,
            rearm_clipboard: None,
        });
    }

    /// Pack the selection (or the row the context menu was opened on) into
    /// a .tar.gz next to it.
    pub fn compress_selected(&mut self) {
        let selected = self.selected_paths();
        if selected.is_empty() || self.refuse_in_trash(&selected, "compressed") {
            return;
        }
        // tar runs in the folder the items are in, and the archive lands
        // there. For a row inside an expanded folder that is that folder,
        // not the one being shown.
        let Some(dir) = selected[0].parent().map(Path::to_path_buf) else {
            return;
        };

        let base_name = selected[0]
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".into());
        // symlink_metadata: a dangling link still has the name.
        let taken = |p: &Path| p.symlink_metadata().is_ok();
        if self.root_covers(&dir) {
            // Through the sudo flow: tar is told its folder and gets the
            // archive as an absolute path, so nothing depends on a working
            // directory. Root mode is never on for a phone or a network
            // folder, so looking for a free name here is a local stat.
            let members = archive_members(&dir, &selected);
            if members.is_empty() {
                return;
            }
            let archive = free_archive_name(&dir, &base_name, taken);
            self.priv_run(crate::sudo::PendingPrivOp::Archive {
                dir,
                archive,
                members,
            });
            return;
        }
        let done = self.refresh_flag();
        std::thread::spawn(move || {
            // Looked up here, not on the UI thread: the folder may be on a
            // phone.
            let archive = free_archive_name(&dir, &base_name, taken);
            let file_args = archive_members(&dir, &selected);
            let _ = std::process::Command::new("tar")
                .arg("czf")
                .arg(&archive)
                // A name starting with '-' must not be read as an option.
                .arg("--")
                .args(&file_args)
                .current_dir(&dir)
                .status();
            // The watcher does not see inside expanded tree folders.
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// Whether a listed path is a folder, from the listing itself: no disk
    /// access, so it is safe on any mount. `None` when it is not listed.
    fn listed_is_dir(&self, path: &Path) -> Option<bool> {
        self.entries
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.is_dir)
            .or_else(|| {
                self.tree_entries
                    .iter()
                    .find(|te| te.entry.path == path)
                    .map(|te| te.entry.is_dir)
            })
    }

    /// "Open as Root" on a folder (the selection, or the row the context
    /// menu was opened on): go into it with root mode on for it. Root mode
    /// for the folder already shown is the empty-area menu's "Root Mode
    /// Here"; nothing here ever switches it on without going somewhere.
    pub fn open_as_root(&mut self) {
        let targets = self.selected_paths();
        let Some(dir) = root_folder(&targets, |p| self.listed_is_dir(p)) else {
            return;
        };
        if dir != self.current_dir {
            // Leaves root mode for the folder being left.
            self.navigate_to(dir);
        }
        self.enter_root_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_actions_work_from_the_row_not_from_the_shown_folder() {
        let p = |s: &str| PathBuf::from(s);
        // A row inside an expanded subfolder: the archive is made in that
        // subfolder, from names relative to it.
        let nested = vec![p("/home/a/Projects/sub/report.txt")];
        let dir = nested[0].parent().unwrap();
        assert_eq!(dir, Path::new("/home/a/Projects/sub"));
        assert_eq!(archive_members(dir, &nested), vec!["report.txt"]);
        // An item from another folder has no name relative to `dir`.
        let mixed = vec![p("/x/one"), p("/y/two"), p("/x/-three")];
        assert_eq!(archive_members(Path::new("/x"), &mixed), vec!["one", "-three"]);

        // Open as Root: a folder row opens that folder...
        let dirs = |path: &Path| Some(path.ends_with("sub") || path.ends_with("Projects"));
        assert_eq!(
            root_folder(&[p("/home/a/Projects/sub")], dirs),
            Some(p("/home/a/Projects/sub"))
        );
        // ...a file row opens nothing: there is no folder to go into, and
        // root mode is never switched on for "whatever is shown".
        assert_eq!(root_folder(&[p("/home/a/Projects/sub/report.txt")], dirs), None);
        // With files and a folder selected, the folder wins.
        assert_eq!(
            root_folder(&[p("/home/a/note.txt"), p("/home/a/Projects")], dirs),
            Some(p("/home/a/Projects"))
        );
        assert_eq!(root_folder(&[], dirs), None);
    }

    #[test]
    fn compress_never_reuses_an_archive_name() {
        let dir = Path::new("/opt/app");
        let have = [dir.join("logs.tar.gz"), dir.join("logs (2).tar.gz")];
        let taken = |p: &Path| have.iter().any(|h| h == p);
        assert_eq!(free_archive_name(dir, "logs", taken), dir.join("logs (3).tar.gz"));
        assert_eq!(free_archive_name(dir, "data", taken), dir.join("data.tar.gz"));
    }
}

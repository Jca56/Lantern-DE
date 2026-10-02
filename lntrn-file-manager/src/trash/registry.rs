//! Volume trashes the mount table cannot lead us back to.
//!
//! A drive's trash is found by looking at each mount point. A trash at the
//! top of a nested btrfs subvolume (a different `st_dev`, but no mount of
//! its own) is on no such list: without a note of where it is, what was
//! trashed there would vanish from the Trash view at the next start. Those
//! few directories are remembered in a small text file, one path per line.
//! A stale line costs nothing: every entry is checked before it is used.

use std::path::PathBuf;

use super::TrashDir;

static USED: std::sync::Mutex<Vec<TrashDir>> = std::sync::Mutex::new(Vec::new());

fn list_path() -> PathBuf {
    crate::app::dirs_home().join(".lantern/config/file-manager-trashes")
}

/// Note that `td` (a volume trash) now holds something.
pub(super) fn remember(td: &TrashDir) {
    let Some(top) = &td.top else { return };
    let mut used = USED.lock().unwrap_or_else(|e| e.into_inner());
    if used.contains(td) {
        return;
    }
    used.push(td.clone());
    // Tests trash into scratch folders; those must never reach the user's
    // real list.
    if cfg!(test) {
        return;
    }
    // Mounted drives are found again through the mount table.
    if crate::fs::mounts().iter().any(|(mount, _)| mount == top) {
        return;
    }
    let line = td.dir.to_string_lossy().into_owned();
    // A newline in the path would split it into two bogus entries.
    if line.contains('\n') {
        return;
    }
    let path = list_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l == line) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{existing}{line}\n"));
}

/// Remembered volume trashes: this session's and the saved ones. Unchecked;
/// the caller validates each before trusting it.
pub(super) fn remembered() -> Vec<TrashDir> {
    let mut out: Vec<TrashDir> = USED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let saved = std::fs::read_to_string(list_path()).unwrap_or_default();
    for line in saved.lines().filter(|l| !l.is_empty()) {
        let dir = PathBuf::from(line);
        // Only the two shapes a volume trash can have are accepted.
        let Some(found) = super::locate(&dir.join("files")) else {
            continue;
        };
        if found.trash.top.is_some() && !out.contains(&found.trash) {
            out.push(found.trash);
        }
    }
    out
}

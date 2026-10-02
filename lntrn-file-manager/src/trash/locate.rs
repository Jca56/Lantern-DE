//! Telling whether a path is inside a trash, from its shape alone.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::{home_trash, uid, TrashDir};

/// A path inside some trash's `files/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub trash: TrashDir,
    /// The path below `files/`; empty for `files/` itself.
    pub rel: PathBuf,
}

/// Is `path` inside a trash, and which one? Decided from the path's shape
/// alone (no I/O), so it is safe to ask every frame and on any mount.
pub fn locate(path: &Path) -> Option<Located> {
    let home = home_trash();
    if let Ok(rel) = path.strip_prefix(home.files()) {
        return Some(Located {
            trash: home,
            rel: rel.to_path_buf(),
        });
    }
    let uid = uid().to_string();
    let own = format!(".Trash-{uid}");
    for anc in path.ancestors() {
        if anc.file_name() != Some(OsStr::new("files")) {
            continue;
        }
        let Some(dir) = anc.parent() else { continue };
        let top = if dir.file_name() == Some(OsStr::new(&own)) {
            dir.parent()
        } else if dir.file_name() == Some(OsStr::new(&uid))
            && dir.parent().and_then(|p| p.file_name()) == Some(OsStr::new(".Trash"))
        {
            dir.parent().and_then(|p| p.parent())
        } else {
            None
        };
        if let (Some(top), Ok(rel)) = (top, path.strip_prefix(anc)) {
            return Some(Located {
                trash: TrashDir {
                    dir: dir.to_path_buf(),
                    top: Some(top.to_path_buf()),
                },
                rel: rel.to_path_buf(),
            });
        }
    }
    None
}

/// The real location of a trash's `files/` folder.
fn real_files_dir(trash: &TrashDir) -> Option<PathBuf> {
    trash.files().canonicalize().ok()
}

/// Is `path` an item that really lies in a trash? [`locate`] goes by the
/// shape of the path, and below a trashed link to a folder the listing shows
/// the folder the link points at: those rows have a trash-shaped path and
/// are live files. Only what is in a trash by shape *and* by where it is on
/// disk may be treated as trashed (and so deleted for good on request).
pub fn is_trashed(path: &Path) -> bool {
    let Some(loc) = locate(path) else {
        return false;
    };
    if loc.rel.as_os_str().is_empty() || crate::fs::is_slow_path(path) {
        return false;
    }
    // The item itself may be a link (a trashed link is an item like any
    // other), so it is its folder that gets resolved.
    let (Some(parent), Some(files)) = (path.parent(), real_files_dir(&loc.trash)) else {
        return false;
    };
    parent.canonicalize().is_ok_and(|real| real.starts_with(&files))
}

/// Does the folder `dir` show a trash, or a real folder inside one? Not when
/// it was reached through a trashed link: what is listed there is live.
pub fn shows_trash(dir: &Path) -> bool {
    let Some(loc) = locate(dir) else {
        return false;
    };
    if crate::fs::is_slow_path(dir) {
        return false;
    }
    match (dir.canonicalize(), real_files_dir(&loc.trash)) {
        (Ok(real), Some(files)) => real.starts_with(&files),
        // Not there (yet): the home trash before anything was trashed.
        _ => true,
    }
}


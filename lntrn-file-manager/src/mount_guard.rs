//! A folder with a drive, a phone or a share mounted on it, or anywhere
//! below it, is not an ordinary folder. Deleting it "with everything inside"
//! deletes what is on the mounted device, and moving it into the Trash
//! carries the mount along, where Empty Trash would then reach it.
//!
//! The Trash, the permanent delete and the root delete all ask here first
//! and leave such a folder alone.

use std::path::{Path, PathBuf};

/// Completes "can't be deleted: ...".
pub const REASON: &str = "a drive or device is mounted in it; eject it first";

/// True when something is mounted at `path` or below it. A symlink holds
/// nothing: it is removed as a link.
pub fn holds_mount(path: &Path) -> bool {
    let mounts = || crate::fs::mounts().into_iter().map(|(mount, _)| mount);
    // As written first: this needs no look at the disk, and it is the whole
    // answer for a phone or a share, where a look can hang.
    if holds_any(path, mounts()) {
        return true;
    }
    if crate::fs::is_slow_path(path) {
        return false;
    }
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return false;
    }
    // /proc/mounts lists resolved paths, so the folder is resolved as well.
    let Ok(real) = path.canonicalize() else {
        return false;
    };
    holds_any(&real, mounts())
}

fn holds_any(real: &Path, mounts: impl IntoIterator<Item = PathBuf>) -> bool {
    mounts.into_iter().any(|mount| mount.starts_with(real))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mount_point_and_every_folder_above_it_hold_a_mount() {
        let mounts = || {
            vec![
                PathBuf::from("/"),
                PathBuf::from("/home/a/mnt/server"),
                PathBuf::from("/run/media/a/STICK"),
            ]
        };
        assert!(holds_any(Path::new("/home/a/mnt/server"), mounts()));
        assert!(holds_any(Path::new("/home/a/mnt"), mounts()));
        assert!(holds_any(Path::new("/run/media"), mounts()));
        assert!(!holds_any(Path::new("/home/a/mnt/server/photos"), mounts()));
        assert!(!holds_any(Path::new("/home/a/Documents"), mounts()));
        // A name that merely starts the same is another folder.
        assert!(!holds_any(Path::new("/home/a/mnt/serv"), mounts()));
    }

    #[test]
    fn a_plain_folder_a_file_and_a_link_hold_nothing() {
        let dir = std::env::temp_dir().join(format!("fox-mount-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plain")).unwrap();
        std::fs::write(dir.join("file"), b"x").unwrap();
        // A link to the root of everything is still only a link.
        std::os::unix::fs::symlink("/", dir.join("link")).unwrap();
        assert!(!holds_mount(&dir.join("plain")));
        assert!(!holds_mount(&dir.join("file")));
        assert!(!holds_mount(&dir.join("link")));
        assert!(!holds_mount(&dir.join("missing")));
        assert!(holds_mount(Path::new("/")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

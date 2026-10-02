// What sync leaves alone inside ~/Cloud.
//
// An ignored path is inert on all three sides: it is not scanned, not
// uploaded, not downloaded and never read as "deleted". The scan, the action
// loop of a pass and the file watcher all ask here, so they cannot disagree.

/// Directories ignored wherever they sit in the tree, not just at the root.
const IGNORE_DIRS: &[&str] = &[".git", ".syncthing"];
const IGNORE_FILENAMES: &[&str] = &[".DS_Store"];
/// Suffix of the temp file a download is written to before the rename.
pub(super) const TMP_SUFFIX: &str = ".fox-tmp";
/// Suffix of the temp file the audio tag writer rewrites a song through.
const TAG_TMP_SUFFIX: &str = ".lntrn-tmp";
/// Start of the name a copy or move in progress carries until it is complete
/// (copy_tree.rs). For a folder the files inside have their ordinary names,
/// so it is the folder's name that has to be recognised.
const PART_PREFIX: &str = ".fox-part-";

/// Directory names that are never descended into.
pub(super) fn is_ignored_dir(name: &str) -> bool {
    IGNORE_DIRS.contains(&name) || name.starts_with(PART_PREFIX) || is_volume_trash(name)
}

/// `.Trash` or `.Trash-<uid>`: the trash of a ~/Cloud that is the top of
/// its own filesystem. What is trashed there must not be uploaded as new.
fn is_volume_trash(name: &str) -> bool {
    match name.strip_prefix(".Trash") {
        Some("") => true,
        Some(rest) => rest
            .strip_prefix('-')
            .is_some_and(|uid| !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit())),
        None => false,
    }
}

pub(super) fn should_ignore(rel: &str) -> bool {
    if let Some((dirs, _)) = rel.rsplit_once('/') {
        if dirs.split('/').any(is_ignored_dir) {
            return true;
        }
    }
    if let Some(name) = rel.rsplit('/').next() {
        if IGNORE_FILENAMES.contains(&name)
            || name.ends_with(TMP_SUFFIX)
            || name.ends_with(TAG_TMP_SUFFIX)
            || name.starts_with(PART_PREFIX)
        {
            return true;
        }
        // Skip in-flight conflict-rename intermediates and dotfiles starting with #
        if name.starts_with('#') || name.ends_with('~') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_vcs_dirs_at_any_depth() {
        assert!(should_ignore(".git/config"));
        assert!(should_ignore("projects/app/.git/objects/ab/cdef"));
        assert!(should_ignore("a/.syncthing/x"));
        // A file merely named like the directory is a normal file.
        assert!(!should_ignore("notes/.git"));
        assert!(!should_ignore("notes/git/readme.md"));
    }

    #[test]
    fn ignores_temp_and_editor_files() {
        assert!(should_ignore("photos/cat.jpg.fox-tmp"));
        assert!(should_ignore("music/.song.wav.4242.0.lntrn-tmp"));
        assert!(should_ignore("doc.txt~"));
        assert!(should_ignore("#doc.txt#"));
        assert!(should_ignore("sub/.DS_Store"));
        assert!(!should_ignore("photos/cat.jpg"));
    }

    #[test]
    fn ignores_a_copy_in_progress_and_everything_inside_it() {
        assert!(should_ignore(".fox-part-4242-0-film.mkv.fox-tmp"));
        assert!(is_ignored_dir(".fox-part-4242-1-Holiday.fox-tmp"));
        assert!(should_ignore(
            "photos/.fox-part-4242-1-Holiday.fox-tmp/day 1/cat.jpg"
        ));
        assert!(!should_ignore("photos/fox-part-notes.txt"));
    }

    #[test]
    fn ignores_a_volume_trash_inside_the_sync_root() {
        assert!(should_ignore(".Trash-1000/files/old.txt"));
        assert!(should_ignore(".Trash-1000/info/old.txt.trashinfo"));
        assert!(should_ignore(".Trash/1000/files/old.txt"));
        assert!(!should_ignore(".Trash-notes/a.txt"));
        assert!(!should_ignore("docs/.Trashcan"));
    }
}

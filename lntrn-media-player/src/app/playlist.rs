//! Which files make up the playlist: the opened file's folder, in the order
//! Fox lists it.

use std::path::{Path, PathBuf};

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "flv", "wmv", "m4v", "mp3", "flac", "wav", "ogg", "m4a",
    "aac", "opus", "wma",
];

/// The playlist for `file_path`'s folder — in the order Fox currently lists
/// it (same sort key, direction and hidden-file rule) — and where the file
/// sits in it.
pub(super) fn scan(file_path: &Path) -> (Vec<PathBuf>, usize) {
    let Some(dir) = file_path.parent() else {
        return (vec![file_path.to_path_buf()], 0);
    };
    let listing = crate::dir_sort::read_fox_listing();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_media(p))
        .filter(|p| {
            listing.show_hidden
                || !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    // Opened on purpose, so it belongs in the list even if the rules above
    // would have left it out (a hidden file, an odd extension).
    if !files.iter().any(|p| p == file_path) {
        files.push(file_path.to_path_buf());
    }
    crate::dir_sort::sort_like_fox(&mut files, listing);
    let index = files.iter().position(|p| p == file_path).unwrap_or(0);
    (files, index)
}

fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| MEDIA_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

/// Absolute path with the folder canonicalized but the file itself left
/// alone, so a symlinked track stays in the folder it was opened from and
/// Next/Previous walk that folder, not wherever the link points. None when
/// the file isn't there (a dangling link counts as missing).
pub(super) fn resolve(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    let name = abs.file_name()?.to_os_string();
    let dir = abs.parent()?.canonicalize().ok()?;
    let resolved = dir.join(name);
    resolved.exists().then_some(resolved)
}

/// Key a file's resume position is stored under. Deliberately not a real
/// (escaped) URI: this is the form positions.json has always used, and
/// changing it would orphan every remembered position.
pub(super) fn store_key(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lntrn-media-player-playlist-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn scan_lists_media_files_and_finds_the_opened_one() {
        let dir = scratch("scan");
        for name in ["b.mp3", "a.flac", "notes.txt", "c.MKV"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::create_dir(dir.join("folder.mp3")).unwrap();
        let (files, index) = scan(&dir.join("b.mp3"));
        let mut names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files[index], dir.join("b.mp3"));
        names.sort();
        // Media only: no text file, no directory that merely looks like one.
        assert_eq!(names, ["a.flac", "b.mp3", "c.MKV"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_opened_on_purpose_is_always_in_the_list() {
        let dir = scratch("explicit");
        std::fs::write(dir.join("song.mp3"), b"x").unwrap();
        std::fs::write(dir.join("clip.weird"), b"x").unwrap();
        let (files, index) = scan(&dir.join("clip.weird"));
        assert_eq!(files[index], dir.join("clip.weird"));
        assert_eq!(files.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_keeps_a_symlinked_file_as_the_link() {
        let dir = scratch("resolve");
        let (real, shown) = (dir.join("real"), dir.join("shown"));
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&shown).unwrap();
        std::fs::write(real.join("track.mp3"), b"x").unwrap();
        std::os::unix::fs::symlink(real.join("track.mp3"), shown.join("link.mp3")).unwrap();
        // Next/Previous walk `shown`, the folder it was opened from.
        assert_eq!(resolve(&shown.join("link.mp3")), Some(shown.join("link.mp3")));
        assert_eq!(resolve(&shown.join("missing.mp3")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_key_matches_what_older_builds_saved() {
        assert_eq!(
            store_key(Path::new("/home/alva/Music/Track #3.mp3")),
            "file:///home/alva/Music/Track #3.mp3"
        );
    }
}

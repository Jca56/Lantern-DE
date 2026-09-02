//! Sibling ordering for Left/Right navigation.
//!
//! Mirrors Fox's (lntrn-file-manager) sort so stepping through a folder in
//! the viewer follows the same order the user sees in the file manager.
//! The setting is read from `~/.lantern/config/file-manager.json` every time
//! a directory is scanned, so a viewer launched after a sort change picks
//! the new order up without a rebuild or restart of anything.

use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Size,
    Date,
    Type,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// What Fox is currently sorting by, plus whether it lists dotfiles.
#[derive(Clone, Copy, Debug)]
pub struct FoxListing {
    pub by: SortBy,
    pub dir: SortDir,
    pub show_hidden: bool,
}

impl Default for FoxListing {
    fn default() -> Self {
        Self {
            by: SortBy::Name,
            dir: SortDir::Asc,
            show_hidden: false,
        }
    }
}

/// Read Fox's persisted listing settings. Falls back to Name/Asc with hidden
/// files excluded when the config is missing or unparsable (fresh install, or
/// a machine where Fox has never been run).
pub fn read_fox_listing() -> FoxListing {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let path = PathBuf::from(home).join(".lantern/config/file-manager.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return FoxListing::default();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return FoxListing::default();
    };
    let by = match json.get("sort_by").and_then(|v| v.as_str()) {
        Some("size") => SortBy::Size,
        Some("date") => SortBy::Date,
        Some("type") => SortBy::Type,
        _ => SortBy::Name,
    };
    let dir = match json.get("sort_dir").and_then(|v| v.as_str()) {
        Some("desc") => SortDir::Desc,
        _ => SortDir::Asc,
    };
    let show_hidden = json
        .get("show_hidden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    FoxListing {
        by,
        dir,
        show_hidden,
    }
}

struct Entry {
    path: PathBuf,
    name_lc: String,
    size: u64,
    modified: SystemTime,
    ext: String,
}

/// Sort `files` in place exactly the way Fox's `fs::list_dir` would order
/// them: primary key per `SortBy`, flipped for `Desc`, with a case-insensitive
/// name tiebreak that always stays ascending.
pub fn sort_like_fox(files: &mut Vec<PathBuf>, listing: FoxListing) {
    let mut entries: Vec<Entry> = files
        .drain(..)
        .map(|path| {
            let meta = std::fs::metadata(&path).ok();
            Entry {
                name_lc: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default(),
                size: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH),
                ext: path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default(),
                path,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        let primary = match listing.by {
            SortBy::Name => a.name_lc.cmp(&b.name_lc),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Date => a.modified.cmp(&b.modified),
            SortBy::Type => a.ext.cmp(&b.ext),
        };
        let primary = match listing.dir {
            SortDir::Asc => primary,
            SortDir::Desc => primary.reverse(),
        };
        primary.then_with(|| a.name_lc.cmp(&b.name_lc))
    });

    files.extend(entries.into_iter().map(|e| e.path));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn names(files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect()
    }

    /// Scratch directory that removes itself when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "lntrn-image-viewer-dir_sort-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Fixture: mixed-case names, distinct sizes and mtimes, two extensions.
    fn fixture() -> (Scratch, Vec<PathBuf>) {
        let dir = Scratch::new();
        // (name, size bytes, mtime offset seconds)
        let spec = [
            ("Zebra.png", 10, 30),
            ("apple.jpg", 30, 10),
            ("Banana.png", 20, 40),
            ("cherry.jpg", 40, 20),
        ];
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut paths = Vec::new();
        for (name, size, secs) in spec {
            let p = dir.path().join(name);
            fs::write(&p, vec![b'x'; size]).unwrap();
            let f = fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(base + Duration::from_secs(secs)).unwrap();
            paths.push(p);
        }
        (dir, paths)
    }

    fn listing(by: SortBy, dir: SortDir) -> FoxListing {
        FoxListing {
            by,
            dir,
            show_hidden: false,
        }
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let (_d, mut files) = fixture();
        sort_like_fox(&mut files, listing(SortBy::Name, SortDir::Asc));
        assert_eq!(
            names(&files),
            ["apple.jpg", "Banana.png", "cherry.jpg", "Zebra.png"]
        );
        sort_like_fox(&mut files, listing(SortBy::Name, SortDir::Desc));
        assert_eq!(
            names(&files),
            ["Zebra.png", "cherry.jpg", "Banana.png", "apple.jpg"]
        );
    }

    #[test]
    fn date_sort_newest_first_when_desc() {
        let (_d, mut files) = fixture();
        sort_like_fox(&mut files, listing(SortBy::Date, SortDir::Desc));
        assert_eq!(
            names(&files),
            ["Banana.png", "Zebra.png", "cherry.jpg", "apple.jpg"]
        );
        sort_like_fox(&mut files, listing(SortBy::Date, SortDir::Asc));
        assert_eq!(
            names(&files),
            ["apple.jpg", "cherry.jpg", "Zebra.png", "Banana.png"]
        );
    }

    #[test]
    fn size_sort() {
        let (_d, mut files) = fixture();
        sort_like_fox(&mut files, listing(SortBy::Size, SortDir::Desc));
        assert_eq!(
            names(&files),
            ["cherry.jpg", "apple.jpg", "Banana.png", "Zebra.png"]
        );
    }

    #[test]
    fn type_sort_groups_by_extension_with_name_tiebreak() {
        let (_d, mut files) = fixture();
        sort_like_fox(&mut files, listing(SortBy::Type, SortDir::Asc));
        assert_eq!(
            names(&files),
            ["apple.jpg", "cherry.jpg", "Banana.png", "Zebra.png"]
        );
    }
}

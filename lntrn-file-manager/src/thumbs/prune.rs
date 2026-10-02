//! Keeps the on-disk thumbnail cache from growing without end.
//!
//! A cache file is named after a hash of its source's path, size and
//! modification time, so every edit of a picture leaves the previous
//! thumbnail behind, and thumbnails of deleted files, unplugged drives and
//! phone photos were never removed by anything. Once per run, on a thread
//! of its own (the folder holds thousands of files), the cache is brought
//! back within bounds:
//!  - files of an older cache generation go (their pixels are wrong now);
//!  - files not used for `MAX_AGE` go;
//!  - if what is left is still over `MAX_BYTES`, the ones used longest ago
//!    go until it is down to `KEEP_BYTES`.
//!
//! "Used" is the file's modification time: `mark_used` moves it forward
//! when a thumbnail is served from the cache (access times are not kept on
//! most mounts).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A thumbnail nobody has looked at for this long is dropped. Generous:
/// one for a photo on a phone costs a download of the whole photo to make
/// again.
const MAX_AGE: Duration = Duration::from_secs(90 * 24 * 60 * 60);
/// The cache may take this much disk (about 7,000 thumbnails at the size
/// they average)...
const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// ...and is cut back to this when it does, so the next few hundred new
/// thumbnails do not each bring it over again.
const KEEP_BYTES: u64 = MAX_BYTES / 4 * 3;
/// A cache hit moves the file's time forward at most this often: the order
/// only has to be right to the day, and it saves a write per thumbnail
/// shown.
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// One file of the cache folder.
#[derive(Clone, Debug, PartialEq)]
struct CacheFile {
    path: PathBuf,
    used: SystemTime,
    bytes: u64,
    /// Written by this version of the generator.
    current: bool,
}

struct Limits {
    max_age: Duration,
    max_bytes: u64,
    keep_bytes: u64,
}

const LIMITS: Limits = Limits {
    max_age: MAX_AGE,
    max_bytes: MAX_BYTES,
    keep_bytes: KEEP_BYTES,
};

/// The files to remove.
fn plan(mut files: Vec<CacheFile>, now: SystemTime, limits: &Limits) -> Vec<PathBuf> {
    let mut remove = Vec::new();
    // A time in the future (the clock was set back) counts as just used.
    let age = |f: &CacheFile| now.duration_since(f.used).unwrap_or_default();
    files.retain(|f| {
        let stale = !f.current || age(f) > limits.max_age;
        if stale {
            remove.push(f.path.clone());
        }
        !stale
    });
    let mut total: u64 = files.iter().map(|f| f.bytes).sum();
    if total > limits.max_bytes {
        files.sort_by_key(|f| f.used);
        for f in &files {
            if total <= limits.keep_bytes {
                break;
            }
            total -= f.bytes;
            remove.push(f.path.clone());
        }
    }
    remove
}

/// The cache folder's thumbnails. `generation` is the file name prefix of
/// the current one. Anything that is not a `.png` is not ours to judge.
fn scan(dir: &Path, generation: &str) -> Vec<CacheFile> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".png") {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(CacheFile {
                path: entry.path(),
                used: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                bytes: meta.len(),
                current: name.starts_with(generation),
            })
        })
        .collect()
}

fn run(dir: &Path, generation: &str, now: SystemTime, limits: &Limits) -> usize {
    let doomed = plan(scan(dir, generation), now, limits);
    for path in &doomed {
        // Another Fox may be pruning the same folder: gone is fine.
        let _ = std::fs::remove_file(path);
    }
    doomed.len()
}

/// Prune the cache folder once per run, off the calling thread. Called by
/// the generator the first time it is asked for a thumbnail, so a window
/// that never shows one never scans the folder.
pub(super) fn start_once(dir: PathBuf, generation: &'static str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Not started (out of threads): the cache is pruned by the next run.
        let _ = std::thread::Builder::new()
            .name("fox-thumb-prune".into())
            .spawn(move || {
                let removed = run(&dir, generation, SystemTime::now(), &LIMITS);
                if removed > 0 {
                    eprintln!("[fox] thumbnail cache: removed {removed} old files");
                }
            });
    });
}

/// A thumbnail was served from `cached`: note that it is still in use.
pub(super) fn mark_used(cached: &Path) {
    let now = SystemTime::now();
    let fresh = std::fs::metadata(cached)
        .and_then(|m| m.modified())
        .is_ok_and(|used| now.duration_since(used).unwrap_or_default() < TOUCH_AFTER);
    if fresh {
        return;
    }
    if let Ok(file) = std::fs::File::options().write(true).open(cached) {
        let _ = file.set_modified(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn file(name: &str, age_days: u32, bytes: u64, now: SystemTime) -> CacheFile {
        CacheFile {
            path: PathBuf::from(name),
            used: now - DAY * age_days,
            bytes,
            current: true,
        }
    }

    fn limits(max_age_days: u32, max_bytes: u64, keep_bytes: u64) -> Limits {
        Limits {
            max_age: DAY * max_age_days,
            max_bytes,
            keep_bytes,
        }
    }

    fn names(paths: Vec<PathBuf>) -> Vec<String> {
        let mut names: Vec<String> = paths
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_cache_within_its_bounds_is_left_alone() {
        let now = SystemTime::now();
        let files = vec![file("a", 1, 400, now), file("b", 80, 500, now)];
        assert!(plan(files, now, &limits(90, 1000, 750)).is_empty());
    }

    #[test]
    fn old_files_and_old_generations_go_whatever_the_size() {
        let now = SystemTime::now();
        let mut old_gen = file("old-gen", 0, 10, now);
        old_gen.current = false;
        let files = vec![file("fresh", 2, 10, now), file("stale", 91, 10, now), old_gen];
        assert_eq!(
            names(plan(files, now, &limits(90, 1000, 750))),
            ["old-gen", "stale"]
        );
    }

    #[test]
    fn over_the_size_limit_the_least_recently_used_go_first() {
        let now = SystemTime::now();
        // 1,200 bytes against a limit of 1,000: cut back to 750 or less.
        let files = vec![
            file("newest", 1, 300, now),
            file("oldest", 40, 300, now),
            file("middle", 10, 300, now),
            file("older", 20, 300, now),
        ];
        // Dropping "oldest" leaves 900, still over 750; "older" makes 600.
        assert_eq!(
            names(plan(files, now, &limits(90, 1000, 750))),
            ["older", "oldest"]
        );
    }

    #[test]
    fn stale_files_do_not_count_toward_the_size() {
        let now = SystemTime::now();
        // 1,100 bytes in all, but 400 of them go for their age anyway:
        // what is left is within the limit and stays.
        let files = vec![
            file("a", 1, 350, now),
            file("b", 5, 350, now),
            file("ancient", 200, 400, now),
        ];
        assert_eq!(names(plan(files, now, &limits(90, 1000, 750))), ["ancient"]);
    }

    #[test]
    fn a_time_in_the_future_counts_as_just_used() {
        let now = SystemTime::now();
        let mut ahead = file("ahead", 0, 10, now);
        ahead.used = now + DAY * 3;
        assert!(plan(vec![ahead], now, &limits(90, 1000, 750)).is_empty());
    }

    #[test]
    fn pruning_a_folder_removes_what_the_plan_names_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("fox-thumb-prune-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let write = |name: &str, age: Duration| {
            let path = dir.join(name);
            std::fs::write(&path, [0u8; 100]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - age).unwrap();
            path
        };
        let fresh = write("v2-0000000000000001.png", DAY);
        let stale = write("v2-0000000000000002.png", DAY * 120);
        let old_gen = write("0000000000000003.png", DAY);
        let not_ours = write("notes.txt", DAY * 400);

        assert_eq!(run(&dir, "v2-", now, &LIMITS), 2);
        assert!(fresh.exists() && not_ours.exists());
        assert!(!stale.exists() && !old_gen.exists());

        // A hit on a thumbnail last used long ago moves it to "now"; one
        // used today is left as it is.
        let dusty = write("v2-0000000000000004.png", DAY * 30);
        mark_used(&dusty);
        let used = std::fs::metadata(&dusty).unwrap().modified().unwrap();
        assert!(now.duration_since(used).unwrap_or_default() < DAY);
        let recent = write("v2-0000000000000005.png", Duration::from_secs(3600));
        let before = std::fs::metadata(&recent).unwrap().modified().unwrap();
        mark_used(&recent);
        assert_eq!(std::fs::metadata(&recent).unwrap().modified().unwrap(), before);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// Local scan of ~/Cloud.
//
// The scan can fail, and a failure is never a deletion. Three outcomes for a
// path that the manifest knows:
//   in `files`          → present, with its hash
//   covered by `unknown`→ could not be looked at; the pass leaves it alone
//                         (no upload, no download over it, no deletion)
//   neither             → the directory it lives in was listed completely and
//                         it is not there: really absent
// A missing or unreadable root is an error for the whole pass (RootError);
// the root is never created here.
//
// Two kinds of file are set aside without being read, and are "unknown" in
// the sense above (inert on every side, never a deletion):
//   - a file too big for the cloud (storage::MAX_BLOB_BYTES): hashing a
//     gigabyte on every pass to then have the upload refused helps nobody;
//   - a file written to within the last moments (`busy_window`): it is
//     still being copied or downloaded into the folder, and what would be
//     hashed is a part of it. It is looked at again when it has settled.
//
// Relative paths are the exact bytes of the file names joined with '/'. A
// name that is not valid UTF-8 cannot be a Firestore key: it is skipped (and
// reported once), never rewritten. A backslash is an ordinary character.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::hash::sha256_file;
use super::manifest::Manifest;
use super::ignore::{is_ignored_dir, should_ignore};
use super::storage::MAX_BLOB_BYTES;

/// Re-hash a file before overwriting or deleting it when it is at most this
/// big; above that, size + mtime (ns) + inode have to do.
const REHASH_MAX_BYTES: u64 = 8 * 1024 * 1024;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub(super) struct LocalFile {
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    /// Unix seconds: what the stat cache and the remote doc carry.
    pub mtime: u64,
    /// The full stamp and the inode, to notice a change made after the scan.
    pub mtime_ns: i128,
    pub ino: u64,
    pub sha: String,
    /// When (size, mtime) were read, unix seconds — taken just before the
    /// stat, so never later than the hash.
    pub stat_at: u64,
}

fn mtime_ns(md: &std::fs::Metadata) -> i128 {
    md.mtime() as i128 * 1_000_000_000 + md.mtime_nsec() as i128
}

/// Paths the scan could not vouch for.
#[derive(Debug, Default)]
pub(super) struct Unknown {
    paths: HashSet<String>,
    /// Directories, each with a trailing '/': everything below is unknown.
    prefixes: Vec<String>,
    /// Read errors (a directory that would not list, a file that would not
    /// stat or hash). Any of these makes the scan incomplete.
    pub failures: usize,
}

impl Unknown {
    pub fn covers(&self, rel: &str) -> bool {
        self.paths.contains(rel) || self.prefixes.iter().any(|p| rel.starts_with(p.as_str()))
    }

    fn file(&mut self, rel: String) {
        self.paths.insert(rel);
    }

    fn dir(&mut self, rel_dir: &str) {
        self.paths.insert(rel_dir.to_string());
        self.prefixes.push(format!("{rel_dir}/"));
    }
}

#[derive(Debug, Default)]
pub(super) struct Scan {
    pub files: HashMap<String, LocalFile>,
    pub unknown: Unknown,
    /// One line per thing that was skipped or failed. The same problem gives
    /// the same line on every pass, so the caller can log it once.
    pub notes: Vec<String>,
    /// The files and folders behind `unknown.failures`, each with its error.
    pub unreadable: Vec<(String, String)>,
    /// Files too big for the cloud. Not read; covered by `unknown`.
    pub too_big: Vec<String>,
    /// Files still being written. Not read this pass; covered by `unknown`.
    pub busy: usize,
}

/// What a walk carries besides the tree it is in.
struct Walk<'a> {
    manifest: &'a Manifest,
    busy_window: Duration,
    /// Old keys of names holding a backslash (see `scan_local`).
    legacy_keys: Vec<String>,
}

/// Was the file written to less than `window` ago? (A stamp in the future
/// says nothing about that.)
fn written_within(md: &std::fs::Metadata, window: Duration) -> bool {
    if window.is_zero() {
        return false;
    }
    md.modified()
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .is_some_and(|age| age < window)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RootError {
    /// No directory at the sync root.
    Missing,
    /// It is there but cannot be listed.
    Unreadable(String),
}

/// Is there a directory at the sync root? (Following a symlink: ~/Cloud may
/// point at another volume.)
pub(super) fn check_root(root: &Path) -> Result<(), RootError> {
    match std::fs::metadata(root) {
        Ok(md) if md.is_dir() => Ok(()),
        Ok(_) => Err(RootError::Missing),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(RootError::Missing),
        Err(e) => Err(RootError::Unreadable(e.to_string())),
    }
}

/// `busy_window`: a file with no usable stat-cache entry that was written to
/// less than this long ago is not hashed this pass (zero: hash everything).
pub(super) fn scan_local(
    root: &Path,
    manifest: &Manifest,
    busy_window: Duration,
) -> Result<Scan, RootError> {
    check_root(root)?;
    let mut scan = Scan::default();
    let mut walk_state = Walk {
        manifest,
        busy_window,
        legacy_keys: Vec::new(),
    };
    if let Err(e) = walk(root, "", &mut walk_state, &mut scan) {
        return Err(RootError::Unreadable(e.to_string()));
    }
    // An earlier build keyed "a\b.txt" as "a/b.txt". Whatever the manifest
    // or the cloud still holds under that key is not a file that was deleted
    // here: shield it, unless a real file lives at that path.
    for key in walk_state.legacy_keys {
        if !scan.files.contains_key(&key) {
            scan.unknown.file(key);
        }
    }
    Ok(scan)
}

fn join(rel_dir: &str, name: &str) -> String {
    if rel_dir.is_empty() {
        name.to_string()
    } else {
        format!("{rel_dir}/{name}")
    }
}

/// Lists one directory. An error is returned only for the directory the call
/// was made for; the caller decides what that means (the root: the pass
/// fails; a subdirectory: that subtree is unknown).
fn walk(dir: &Path, rel_dir: &str, state: &mut Walk, scan: &mut Scan) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        // An error in the middle of a listing: what was not listed is not
        // known to be absent.
        let entry = entry?;
        let os_name = entry.file_name();
        let Some(name) = os_name.to_str() else {
            let lossy = join(rel_dir, &os_name.to_string_lossy());
            scan.notes.push(format!(
                "not synced (name is not valid UTF-8): {}",
                entry.path().display()
            ));
            // The key an earlier build would have used for it, and for
            // everything below it if it is a directory.
            scan.unknown.dir(&lossy);
            continue;
        };
        let rel = join(rel_dir, name);
        let abs = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                scan.notes.push(format!("cannot stat {}: {e}", abs.display()));
                scan.unknown.dir(&rel);
                scan.unknown.failures += 1;
                scan.unreadable.push((rel, e.to_string()));
                continue;
            }
        };
        if ft.is_symlink() {
            // Not synced, and not "absent" either: a write to a path below
            // a linked folder would go through the link and land outside
            // ~/Cloud, and what is there would read as deleted. Unknown
            // makes the link and everything below it inert on all sides.
            // (Not a failure: nothing here keeps deletions elsewhere back.)
            if !should_ignore(&rel) {
                scan.notes.push(format!("not synced (symbolic link): {}", abs.display()));
                scan.unknown.dir(&rel);
            }
            continue;
        }
        if ft.is_dir() {
            if is_ignored_dir(name) {
                continue;
            }
            if let Err(e) = walk(&abs, &rel, state, scan) {
                scan.notes.push(format!("cannot read folder {}: {e}", abs.display()));
                scan.unknown.dir(&rel);
                scan.unknown.failures += 1;
                scan.unreadable.push((rel, e.to_string()));
            }
            continue;
        }
        if !ft.is_file() || should_ignore(&rel) {
            continue;
        }
        if rel.contains('\\') {
            state.legacy_keys.push(rel.replace('\\', "/"));
        }
        let stat_at = now_secs();
        let md = match entry.metadata() {
            Ok(md) => md,
            Err(e) => {
                scan.notes.push(format!("cannot stat {}: {e}", abs.display()));
                scan.unknown.file(rel.clone());
                scan.unknown.failures += 1;
                scan.unreadable.push((rel, e.to_string()));
                continue;
            }
        };
        let size = md.len();
        if size >= MAX_BLOB_BYTES {
            scan.notes
                .push(format!("not synced (1 GiB or larger): {}", abs.display()));
            scan.unknown.file(rel.clone());
            scan.too_big.push(rel);
            continue;
        }
        let mtime = u64::try_from(md.mtime()).unwrap_or(0);
        // Stat cache: an unchanged (size, mtime) means the last-synced sha
        // still describes this file — skip the sha256 read entirely.
        let sha = match state.manifest.cached_sha(&rel, size, mtime) {
            Some(cached) => cached.to_string(),
            None if written_within(&md, state.busy_window) => {
                scan.unknown.file(rel);
                scan.busy += 1;
                continue;
            }
            None => match sha256_file(&abs) {
                Ok(s) => s,
                Err(e) => {
                    scan.notes.push(format!("cannot read {}: {e}", abs.display()));
                    scan.unknown.file(rel.clone());
                    scan.unknown.failures += 1;
                    scan.unreadable.push((rel, e.to_string()));
                    continue;
                }
            },
        };
        scan.files.insert(
            rel.clone(),
            LocalFile {
                rel,
                abs,
                size,
                mtime,
                mtime_ns: mtime_ns(&md),
                ino: md.ino(),
                sha,
                stat_at,
            },
        );
    }
    Ok(())
}

/// Is the file at `abs` still what the scan saw? `snapshot` is the scan's
/// record of it, or `None` when the scan found nothing there. Called right
/// before a download replaces the file or a remote deletion removes it: a
/// pass can be minutes old by the time it reaches a path, and a save made in
/// between must win.
pub(super) fn unchanged_since_scan(snapshot: Option<&LocalFile>, abs: &Path) -> bool {
    let now = std::fs::symlink_metadata(abs);
    match (snapshot, now) {
        (None, Err(e)) => e.kind() == std::io::ErrorKind::NotFound,
        // Something appeared where the scan saw nothing.
        (None, Ok(_)) => false,
        (Some(_), Err(_)) => false,
        (Some(snap), Ok(md)) => {
            if !md.file_type().is_file()
                || md.len() != snap.size
                || mtime_ns(&md) != snap.mtime_ns
                || md.ino() != snap.ino
            {
                return false;
            }
            if snap.size > REHASH_MAX_BYTES {
                return true;
            }
            sha256_file(abs).is_ok_and(|sha| sha == snap.sha)
        }
    }
}

#[cfg(test)]
mod tests;

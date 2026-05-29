//! Live-indexed filesystem search provider.
//!
//! Walks `$HOME` once on a background thread, then keeps the index in
//! sync with the filesystem via inotify (see [`indexer`]). Search queries
//! fuzzy-match against the *file name* of every indexed path — fast
//! enough to run on every keystroke for a typical home directory.
//!
//! Self-contained: we build our own inotify wrapper on top of raw `libc`
//! rather than pulling in an external crate (`notify`, `walkdir`, …),
//! matching the project's minimal-dependency rule.

mod indexer;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::fuzzy;

/// Hard ceiling on indexed entries. A runaway walk (symlink loops, an
/// absurdly large home) can't blow up memory past this — we just stop
/// adding and log. ~300k paths at ~40 bytes of `String`/`PathBuf` each
/// is well under ~30 MB, comfortable for a desktop daemon.
pub const MAX_INDEXED: usize = 300_000;

/// One indexed filesystem entry. `name` is the final path component,
/// stored separately so the fuzzy matcher doesn't have to re-derive it
/// from `path` on every keystroke.
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// A ranked file hit. Owned (path cloned out from under the index lock)
/// so callers can use it after the lock drops.
pub struct FileHit {
    pub path: PathBuf,
    pub is_dir: bool,
    pub score: f32,
}

/// Shared handle to the background-built file index. Cheap to clone
/// (`Arc`). The indexer thread mutates `entries`; search queries read it.
#[derive(Clone)]
pub struct FileIndex {
    entries: Arc<Mutex<Vec<FileEntry>>>,
    ready: Arc<AtomicBool>,
}

impl FileIndex {
    /// Start the background indexer: an initial `$HOME` walk followed by
    /// an inotify watch loop. Returns immediately; results stream in as
    /// the walk progresses. If `$HOME` is unset we return an inert handle
    /// (`rank` always empty) rather than failing.
    pub fn spawn() -> Self {
        let entries = Arc::new(Mutex::new(Vec::new()));
        let ready = Arc::new(AtomicBool::new(false));

        let roots = search_roots();
        if roots.is_empty() {
            tracing::warn!("no search roots — file search disabled");
            ready.store(true, Ordering::Relaxed);
            return Self { entries, ready };
        }

        let entries_bg = Arc::clone(&entries);
        let ready_bg = Arc::clone(&ready);
        std::thread::Builder::new()
            .name("file-indexer".into())
            .spawn(move || indexer::run(entries_bg, ready_bg, roots))
            .ok();

        Self { entries, ready }
    }

    /// True once the initial walk has completed. Before this, `rank` may
    /// return partial results (whatever has been walked so far).
    #[allow(dead_code)]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Fuzzy-rank indexed files against `query` (file-name match only).
    /// Returns up to `limit` hits sorted by descending score.
    pub fn rank(&self, query: &str, limit: usize) -> Vec<FileHit> {
        if query.is_empty() {
            return Vec::new();
        }
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };

        let mut hits: Vec<FileHit> = entries
            .iter()
            .filter_map(|e| {
                let score = fuzzy::score(query, &e.name)?;
                Some(FileHit {
                    path: e.path.clone(),
                    is_dir: e.is_dir,
                    score,
                })
            })
            .collect();
        drop(entries);

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        hits
    }
}

/// Top-level locations the indexer walks + watches. `$HOME` is the main
/// one; we also add a few system config dirs worth searching directly
/// (e.g. `/etc/portage` so `make.conf` is a keystroke away). Each is
/// gated on `is_dir()`, so a path that doesn't exist on this machine —
/// `/etc/portage` on the Arch laptop, say — is simply skipped.
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home));
    }
    for p in ["/etc/portage"] {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            roots.push(pb);
        }
    }
    roots
}

/// Non-hidden directories we never descend into: huge, machine-generated,
/// never what a human searches for. Hidden build/cache dirs (`.cache`,
/// `.git`, `.gradle`, …) don't need listing here — they're already
/// excluded by the hidden-name rule below.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    "venv",
];

/// Hidden directories we index anyway — `~/.lantern` holds the user's
/// own config/binaries and is very much something they search for.
const HIDDEN_ALLOWLIST: &[&str] = &[".lantern"];

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Whether to recurse into the directory named `name`. Skips heavy
/// build dirs and hidden dirs (except the allowlist).
pub fn should_descend(name: &str) -> bool {
    if SKIP_DIRS.contains(&name) {
        return false;
    }
    if is_hidden(name) && !HIDDEN_ALLOWLIST.contains(&name) {
        return false;
    }
    true
}

/// Whether an entry named `name` should appear in search results at all.
/// Hidden files and hidden (non-allowlisted) directories are kept out —
/// they're noise for a launcher. Heavy non-hidden dirs still show as a
/// single navigable entry; we just don't index their contents.
pub fn should_index(name: &str) -> bool {
    !is_hidden(name) || HIDDEN_ALLOWLIST.contains(&name)
}

/// Render a path for display, collapsing the `$HOME` prefix to `~`.
/// Used as the secondary line under a file result.
pub fn display_path(path: &Path) -> String {
    let full = path.to_string_lossy();
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if let Some(rest) = full.strip_prefix(home.as_ref()) {
            return format!("~{rest}");
        }
    }
    full.into_owned()
}

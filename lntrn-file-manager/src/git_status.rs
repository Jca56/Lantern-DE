//! Git awareness for the file listing: per-entry modified/untracked badges
//! and the current branch name for the status bar.
//!
//! A scan runs on a worker thread (`bg::Task`), which wakes the main loop
//! when it is done. Only one scan runs at a time: a request that comes in
//! while one is out is remembered and started when it lands, so a folder
//! that changes eight times a second never has eight `git status` processes
//! racing each other. Non-repo directories resolve to an empty mark set
//! (no badges, no branch chip).
//!
//! Walking into a folder must never run a program that folder names. The
//! repository is found and its branch read from its files (repo.rs), and
//! `git status` is started only for a repository whose own configuration
//! cannot make git execute anything (config.rs). Any other shows its
//! branch and no badges.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bg::{Polled, Task};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitMark {
    /// Tracked file with staged or unstaged changes (or a dir containing one).
    Modified,
    /// Untracked file (or a dir containing only untracked files).
    Untracked,
}

/// What one scan found.
#[derive(Default, Clone, PartialEq, Debug)]
pub(crate) struct Scanned {
    /// The work tree's top folder (as git names it: the real path).
    root: Option<PathBuf>,
    branch: Option<String>,
    marks: HashMap<PathBuf, GitMark>,
}

type Scanner = Arc<dyn Fn(&Path) -> Scanned + Send + Sync>;

struct Flight {
    dir: PathBuf,
    started: Instant,
    task: Task<Scanned>,
}

/// After a scan, the next one of the same folder waits as long as that
/// scan took, up to this much: in a repository where `git status` needs
/// seconds, a folder under constant change must not keep git running back
/// to back.
const MAX_COOLDOWN: Duration = Duration::from_secs(5);

pub struct GitStatus {
    /// The folder the marks and the branch are for (empty: none).
    dir: PathBuf,
    found: Scanned,
    /// Whether the last completed scan found a repo — gates the periodic
    /// re-scan so non-repo folders are not looked at on a timer.
    in_repo: bool,
    flight: Option<Flight>,
    /// One more scan of `dir` is wanted once the one in flight has landed.
    wanted: bool,
    /// The wanted scan may start from here on.
    not_before: Option<Instant>,
    scanner: Scanner,
}

impl GitStatus {
    pub fn new() -> Self {
        Self::with_scanner(Arc::new(scan))
    }

    fn with_scanner(scanner: Scanner) -> Self {
        Self {
            dir: PathBuf::new(),
            found: Scanned::default(),
            in_repo: false,
            flight: None,
            wanted: false,
            not_before: None,
            scanner,
        }
    }

    /// The view now shows `dir`. What was known about the previous folder
    /// goes: its badges at once, and its branch too unless `dir` is in the
    /// same work tree (the chip then stays put instead of blinking on every
    /// step through a repository). A scan of `dir` starts as soon as no
    /// other is running. True when what is shown changed.
    pub fn enter(&mut self, dir: &Path) -> bool {
        let same_tree = self
            .found
            .root
            .as_ref()
            .is_some_and(|root| dir.starts_with(root));
        let had_marks = !self.found.marks.is_empty();
        let had_branch = self.found.branch.is_some();
        if same_tree {
            self.found.marks.clear();
        } else {
            self.found = Scanned::default();
            self.in_repo = false;
        }
        self.dir = dir.to_path_buf();
        // The user just went there: this scan does not wait out a cooldown.
        self.not_before = None;
        self.request();
        had_marks || (had_branch && !same_tree)
    }

    /// Scan the current folder again (its contents changed, or it is time).
    pub fn refresh(&mut self) {
        if !self.dir.as_os_str().is_empty() {
            self.request();
        }
    }

    fn request(&mut self) {
        self.wanted = true;
        self.start_if_free(Instant::now());
    }

    fn start_if_free(&mut self, now: Instant) {
        if !self.wanted || self.flight.is_some() {
            return;
        }
        if self.not_before.is_some_and(|t| now < t) {
            return;
        }
        self.wanted = false;
        self.not_before = None;
        let dir = self.dir.clone();
        let scanner = Arc::clone(&self.scanner);
        let scan_dir = dir.clone();
        self.flight = Some(Flight {
            dir,
            started: now,
            task: Task::spawn("fox-git", move || scanner(&scan_dir)),
        });
    }

    /// Forget the current repo state without scanning. Used on slow mounts,
    /// where even `git rev-parse` would stat its way up the tree over MTP.
    /// True when that changed what is shown.
    pub fn clear(&mut self) -> bool {
        let shown = self.found.branch.is_some() || !self.found.marks.is_empty();
        self.dir = PathBuf::new();
        self.found = Scanned::default();
        self.in_repo = false;
        self.wanted = false;
        self.not_before = None;
        shown
    }

    /// Take in a finished scan and start the next one if one is wanted.
    /// Called on every wake-up of the main loop; true when what is shown
    /// (the branch chip, the badges) changed and a frame is needed.
    pub fn poll(&mut self, now: Instant) -> bool {
        let mut changed = false;
        let landed = match self.flight.as_mut().map(|flight| flight.task.poll()) {
            None | Some(Polled::Pending) => None,
            Some(Polled::Ready(found)) => Some(Some(found)),
            // The worker died: nothing to show, and no point in sending
            // another one after it right away.
            Some(Polled::Lost) => Some(None),
        };
        if let Some(found) = landed {
            let (for_dir, started) = match self.flight.take() {
                Some(flight) => (flight.dir, flight.started),
                None => (PathBuf::new(), now),
            };
            let took = now.saturating_duration_since(started);
            self.not_before = Some(now + took.min(MAX_COOLDOWN));
            match found {
                // A result for a folder the view has left is not shown, and
                // the scan of the folder it is in now does not wait.
                Some(_) if for_dir != self.dir => self.not_before = None,
                Some(found) => {
                    self.in_repo = found.root.is_some();
                    if found != self.found {
                        self.found = found;
                        changed = true;
                    }
                }
                None => self.wanted = false,
            }
        }
        self.start_if_free(now);
        changed
    }

    /// When `poll` has to be called again for a waiting scan to start.
    pub fn wake_at(&self) -> Option<Instant> {
        if self.wanted && self.flight.is_none() {
            self.not_before
        } else {
            None
        }
    }

    pub fn mark(&self, path: &Path) -> Option<GitMark> {
        self.found.marks.get(path).copied()
    }

    pub fn branch(&self) -> Option<&str> {
        self.found.branch.as_deref()
    }

    pub fn in_repo(&self) -> bool {
        self.in_repo
    }
}

/// Run on a background thread: branch name + marks for entries directly in
/// `dir` (changes deeper in a subtree mark that subtree's top-level dir).
///
/// The repository is found and its branch read here, from its files; no
/// program is started for that. `git status` runs only for a repository
/// whose configuration cannot make git start anything (repo.rs,
/// config.rs). For any other the branch still shows and the badges are
/// simply absent.
fn scan(dir: &Path) -> Scanned {
    // The caller keeps git away from phones and network shares; this is
    // the last line of that rule, not the first.
    if crate::fs::is_slow_path(dir) {
        return Scanned::default();
    }
    // git works with the real path; `dir` may have been reached through a
    // symlink. Compare against the real path, key the marks by `dir` (what
    // the listing's entry paths are built from).
    let Ok(real_dir) = dir.canonicalize() else {
        return Scanned::default();
    };
    if crate::fs::is_slow_path(&real_dir) {
        return Scanned::default();
    }
    let Some(repo) = repo::discover(&real_dir) else {
        return Scanned::default();
    };
    let mut branch = repo.head().and_then(|head| head.label());
    let mut marks = HashMap::new();
    match repo.scan_allowed(&real_dir) {
        Ok(()) => {
            if let Some(out) = git_status(&repo, &real_dir) {
                let (header, found) = parse_status(&out, &repo.root, &real_dir, dir);
                marks = found;
                // Only a HEAD that could not be read from its file (the
                // reftable format) takes git's word for the branch.
                branch = branch.or(header);
            }
        }
        Err(why) => note_skipped(&repo.root, &why),
    }
    Scanned {
        root: Some(repo.root),
        branch,
        marks,
    }
}

/// Say once per repository why it gets no badges.
fn note_skipped(root: &Path, why: &str) {
    static SAID: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if said.iter().any(|r| r == root) {
        return;
    }
    // Bounded: a long session walks through many folders.
    if said.len() >= 256 {
        said.clear();
    }
    said.push(root.to_path_buf());
    eprintln!("[fox-git] no status badges in {}: {why}", root.display());
}

/// What `git status --porcelain=v1 -z -b` printed, as marks keyed by the
/// entries of `dir`, and the branch its header line names.
///
/// `root` and `real_dir` are real paths (git's view); `dir` is the same
/// folder as the listing spells it.
fn parse_status(
    out: &str,
    root: &Path,
    real_dir: &Path,
    dir: &Path,
) -> (Option<String>, HashMap<PathBuf, GitMark>) {
    let mut branch = None;
    let mut marks = HashMap::new();
    // -z: NUL separators, no quoting of unusual filenames. Pathspec "."
    // restricted the output to the viewed subtree; paths are relative to
    // the top of the work tree.
    for record in out.split('\0').filter(|r| r.len() > 3) {
        let (xy, rel) = record.split_at(2);
        if xy == "##" {
            branch = branch_from_header(rel.trim_start());
            continue;
        }
        if xy == "!!" {
            continue; // ignored files
        }
        let mark = if xy == "??" {
            GitMark::Untracked
        } else {
            GitMark::Modified
        };
        // Exactly one separator space after XY; a name may start with
        // spaces of its own.
        let rel = rel.strip_prefix(' ').unwrap_or(rel);
        let abs = root.join(rel.trim_end_matches('/'));
        // Badge the entry the user can actually see: the path itself if
        // it sits directly in `dir`, else the top-level subdir of `dir`
        // that contains it. Modified outranks Untracked when both occur.
        let Ok(below) = abs.strip_prefix(real_dir) else {
            continue;
        };
        let Some(first) = below.components().next() else {
            continue;
        };
        let entry = dir.join(first.as_os_str());
        if marks.get(&entry) != Some(&GitMark::Modified) {
            marks.insert(entry, mark);
        }
    }
    (branch, marks)
}

/// The branch in a `## ...` header: `main`, `main...origin/main [ahead 1]`,
/// `No commits yet on main`, `HEAD (no branch)`.
fn branch_from_header(header: &str) -> Option<String> {
    let name = match header.strip_prefix("No commits yet on ") {
        Some(name) => name,
        None => header.split("...").next()?.split(' ').next()?,
    };
    (!name.is_empty() && name != "HEAD").then(|| repo::display_name(name))
}

/// How long one `git status` may take. git opens files Fox never looks at
/// (the index, .gitignore) with a plain open(); a FIFO among them in an
/// unpacked archive would hold the one scan there is for the whole session.
const GIT_DEADLINE: Duration = Duration::from_secs(10);

/// `git status` for `real_dir`, in exactly the repository `repo` names.
fn git_status(repo: &repo::Repo, real_dir: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("--no-pager")
        // Belt and braces: `scan_allowed` already refused a config that
        // sets either. On the command line they outrank the repository's.
        .args(["-c", "core.fsmonitor=false"])
        .args(["-c", "core.hooksPath=/dev/null"])
        // The repository found by our own walk, and no other: git does not
        // search, so nothing planted in the folder can stand in for it.
        .arg("--git-dir")
        .arg(&repo.git_dir)
        .arg("--work-tree")
        .arg(&repo.root)
        .args(["status", "--porcelain=v1", "-z", "-b", "--no-renames"])
        // A submodule has a config of its own, which nobody has read.
        .arg("--ignore-submodules=all")
        .arg(".")
        .current_dir(real_dir)
        // Never take the index lock: status would rewrite .git/index,
        // which the watcher sees, which re-runs status. And it can make the
        // user's own git command fail with "index.lock exists".
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Never fetch a missing object from a remote for a badge.
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        // The common directory whose config `scan_allowed` read, and no
        // other. Without this git works it out again from the `commondir`
        // file by rules of its own (symlinks resolved first, any size, any
        // bytes), and a crafted one sent git to a config Fox never saw.
        .env("GIT_COMMON_DIR", &repo.common_dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Fox may have been started from a shell inside some git command.
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
        "GIT_EXTERNAL_DIFF",
    ] {
        cmd.env_remove(var);
    }
    let (status, bytes) = crate::bg::output_with_deadline(&mut cmd, GIT_DEADLINE)?;
    if !status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

mod config;
mod repo;

#[cfg(test)]
mod tests;

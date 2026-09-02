//! Git awareness for the file listing: per-entry modified/untracked badges
//! and the current branch name for the status bar.
//!
//! All git invocations run on background threads; results come back through
//! a channel tagged with a generation counter so a stale result from a
//! previous directory can never overwrite a newer one. Non-repo directories
//! resolve to an empty mark set (no badges, no branch chip).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GitMark {
    /// Tracked file with staged or unstaged changes (or a dir containing one).
    Modified,
    /// Untracked file (or a dir containing only untracked files).
    Untracked,
}

struct GitResult {
    generation: u64,
    branch: Option<String>,
    marks: HashMap<PathBuf, GitMark>,
}

pub struct GitStatus {
    generation: u64,
    marks: HashMap<PathBuf, GitMark>,
    branch: Option<String>,
    /// Whether the last completed scan found a repo — gates the periodic
    /// re-scan so non-repo folders never run git on a timer.
    in_repo: bool,
    rx: mpsc::Receiver<GitResult>,
    tx: mpsc::Sender<GitResult>,
}

impl GitStatus {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            generation: 0,
            marks: HashMap::new(),
            branch: None,
            in_repo: false,
            rx,
            tx,
        }
    }

    /// Kick off a background scan of `dir`. Cheap to call; the heavy work
    /// happens off-thread and lands via `poll()`.
    pub fn refresh(&mut self, dir: &Path) {
        self.generation += 1;
        let generation = self.generation;
        let dir = dir.to_path_buf();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let (branch, marks) = scan(&dir);
            let _ = tx.send(GitResult {
                generation,
                branch,
                marks,
            });
        });
    }

    /// Forget the current repo state without scanning. Used on slow mounts,
    /// where even `git rev-parse` would stat its way up the tree over MTP.
    pub fn clear(&mut self) {
        self.generation += 1;
        self.in_repo = false;
        self.branch = None;
        self.marks.clear();
    }

    /// Drain finished scans. Call once per frame.
    pub fn poll(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            if result.generation == self.generation {
                self.in_repo = result.branch.is_some();
                self.branch = result.branch;
                self.marks = result.marks;
            }
        }
    }

    pub fn mark(&self, path: &Path) -> Option<GitMark> {
        self.marks.get(path).copied()
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn in_repo(&self) -> bool {
        self.in_repo
    }
}

/// Run on a background thread: branch name + marks for entries directly in
/// `dir` (changes deeper in a subtree mark that subtree's top-level dir).
fn scan(dir: &Path) -> (Option<String>, HashMap<PathBuf, GitMark>) {
    let Some(root) = git_stdout(dir, &["rev-parse", "--show-toplevel"]) else {
        return (None, HashMap::new());
    };
    let root = PathBuf::from(root.trim_end());
    let branch = git_stdout(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|b| b.trim_end().to_string())
        .filter(|b| !b.is_empty());

    // -z: NUL separators, no quoting of unusual filenames. Pathspec "."
    // restricts output to the viewed subtree; paths are repo-root-relative.
    let mut marks = HashMap::new();
    if let Some(out) = git_stdout(
        dir,
        &["status", "--porcelain=v1", "-z", "--no-renames", "."],
    ) {
        for record in out.split('\0').filter(|r| r.len() > 3) {
            let (xy, rel) = record.split_at(2);
            if xy == "!!" {
                continue; // ignored files
            }
            let mark = if xy == "??" {
                GitMark::Untracked
            } else {
                GitMark::Modified
            };
            let abs = root.join(rel.trim_start().trim_end_matches('/'));
            // Badge the entry the user can actually see: the path itself if
            // it sits directly in `dir`, else the top-level subdir of `dir`
            // that contains it. Modified outranks Untracked when both occur.
            let Ok(below) = abs.strip_prefix(dir) else {
                continue;
            };
            let Some(first) = below.components().next() else {
                continue;
            };
            let entry = dir.join(first.as_os_str());
            match marks.get(&entry) {
                Some(GitMark::Modified) => {}
                _ => {
                    let dominant = if marks.get(&entry) == Some(&GitMark::Untracked)
                        && mark == GitMark::Modified
                    {
                        GitMark::Modified
                    } else {
                        mark
                    };
                    marks.insert(entry, dominant);
                }
            }
        }
    }
    (branch, marks)
}

fn git_stdout(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

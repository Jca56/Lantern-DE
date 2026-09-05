//! Git for the project: the branch, what `git status` says about every
//! file, the HEAD copy of a file for the gutter ([`gutter`]), and staging
//! and committing from the Git editor ([`view`]). Everything is asked of
//! the git binary on a thread, so the editor never waits on it.

pub mod glue;
pub mod gutter;
pub mod view;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};

use lntrn_app::Waker;

/// A file's two status letters from `git status --porcelain`: the index
/// (staged) side and the work-tree side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub index: char,
    pub work: char,
}

impl FileStatus {
    pub fn untracked(self) -> bool {
        self.index == '?'
    }

    pub fn staged(self) -> bool {
        !self.untracked() && self.index != ' ' && self.index != '!'
    }

    pub fn unstaged(self) -> bool {
        self.untracked() || self.work != ' '
    }

    /// The letter to show: the work-tree side when it changed, else the
    /// staged side.
    pub fn letter(self) -> char {
        if self.untracked() {
            '?'
        } else if self.work != ' ' {
            self.work
        } else {
            self.index
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub rel: String,
    pub status: FileStatus,
}

/// The HEAD copy of a file, once asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blob {
    /// Not in HEAD: every line is new.
    Missing,
    Text(String),
}

enum Req {
    Status,
    Blob(PathBuf, String),
    /// Run git with these arguments, then report status again.
    Run(Vec<String>),
}

enum Reply {
    Status { branch: String, head: String, changes: Vec<(String, FileStatus)>, error: Option<String> },
    Blob { path: PathBuf, head: String, blob: Blob },
    Ran { ok: bool, output: String },
}

/// Status is asked again this long after the last change on disk.
const REFRESH_DELAY: f64 = 0.5;

pub struct Git {
    pub root: PathBuf,
    pub branch: String,
    /// The HEAD commit, or empty in a repository with none.
    pub head: String,
    pub changes: Vec<Change>,
    by_path: HashMap<PathBuf, FileStatus>,
    /// Folders with a change somewhere inside.
    pub dirty_dirs: HashSet<PathBuf>,
    blobs: HashMap<PathBuf, (String, Blob)>,
    asked: HashSet<PathBuf>,
    status_pending: bool,
    refresh_at: Option<f64>,
    pub busy: bool,
    pub last_error: Option<String>,
    /// What the last stage/commit printed, for a toast.
    pub last_output: Option<(bool, String)>,
    /// Bumped whenever status or a blob changes.
    pub version: u64,
    pub commit_message: String,
    /// A commit was asked for: the message clears when it succeeds.
    pub commit_pending: bool,
    tx: Sender<Req>,
    rx: Receiver<Reply>,
}

impl Git {
    /// The repository `dir` is in, if any: `.git` here or in a parent.
    pub fn find(dir: &Path, waker: Option<Waker>) -> Option<Self> {
        let mut d = Some(dir);
        while let Some(cur) = d {
            if cur.join(".git").exists() {
                return Some(Self::new(cur.to_path_buf(), waker));
            }
            d = cur.parent();
        }
        None
    }

    fn new(root: PathBuf, waker: Option<Waker>) -> Self {
        let (tx, worker_rx) = channel::<Req>();
        let (worker_tx, rx) = channel::<Reply>();
        let wroot = root.clone();
        let _ = std::thread::Builder::new().name("git".into()).spawn(move || worker(&wroot, &worker_rx, &worker_tx, waker.as_ref()));
        let mut g = Self { root, branch: String::new(), head: String::new(), changes: Vec::new(), by_path: HashMap::new(), dirty_dirs: HashSet::new(), blobs: HashMap::new(), asked: HashSet::new(), status_pending: false, refresh_at: None, busy: false, last_error: None, last_output: None, version: 0, commit_message: String::new(), commit_pending: false, tx, rx };
        g.request_status();
        g
    }

    pub fn request_status(&mut self) {
        if self.status_pending {
            return;
        }
        self.status_pending = true;
        self.busy = true;
        self.refresh_at = None;
        let _ = self.tx.send(Req::Status);
    }

    /// Something changed on disk: status is asked again once it settles.
    pub fn mark_dirty(&mut self, now: f64) {
        self.refresh_at = Some(now + REFRESH_DELAY);
    }

    /// Run a due refresh. Returns the delay to check again, if one is due.
    pub fn tick(&mut self, now: f64) -> Option<f64> {
        let at = self.refresh_at?;
        if now >= at {
            self.request_status();
            None
        } else {
            Some(at - now)
        }
    }

    /// Stage, unstage or commit: `args` after `git`.
    pub fn run(&mut self, args: Vec<String>) {
        self.busy = true;
        self.status_pending = true;
        let _ = self.tx.send(Req::Run(args));
    }

    pub fn status_of(&self, path: &Path) -> Option<FileStatus> {
        self.by_path.get(path).copied()
    }

    pub fn rel(&self, path: &Path) -> Option<String> {
        path.strip_prefix(&self.root).ok().map(|r| r.to_string_lossy().into_owned())
    }

    /// The HEAD copy of `path`, asked for on first call; `None` while it
    /// is on its way.
    pub fn blob(&mut self, path: &Path) -> Option<&Blob> {
        let fresh = self.blobs.get(path).is_some_and(|(h, _)| *h == self.head);
        if !fresh {
            if !self.head.is_empty()
                && !self.asked.contains(path)
                && let Some(rel) = self.rel(path)
            {
                self.asked.insert(path.to_path_buf());
                let _ = self.tx.send(Req::Blob(path.to_path_buf(), rel));
            }
            return None;
        }
        self.blobs.get(path).map(|(_, b)| b)
    }

    /// Take in what the worker answered. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(r) = self.rx.try_recv() {
            changed = true;
            match r {
                Reply::Status { branch, head, changes, error } => {
                    self.status_pending = false;
                    self.busy = false;
                    self.last_error = error;
                    if head != self.head {
                        self.asked.clear();
                    }
                    self.branch = branch;
                    self.head = head;
                    self.changes = changes.into_iter().map(|(rel, status)| Change { path: self.root.join(&rel), rel, status }).collect();
                    self.by_path = self.changes.iter().map(|c| (c.path.clone(), c.status)).collect();
                    self.dirty_dirs.clear();
                    for c in &self.changes {
                        let mut d = c.path.parent();
                        while let Some(p) = d {
                            if !self.dirty_dirs.insert(p.to_path_buf()) || p == self.root {
                                break;
                            }
                            d = p.parent();
                        }
                    }
                    self.version += 1;
                }
                Reply::Blob { path, head, blob } => {
                    self.blobs.insert(path, (head, blob));
                    self.version += 1;
                }
                Reply::Ran { ok, output } => {
                    if std::mem::take(&mut self.commit_pending) && ok {
                        self.commit_message.clear();
                    }
                    self.last_output = Some((ok, output));
                }
            }
        }
        changed
    }

    pub fn staged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.status.staged())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.status.unstaged())
    }
}

/// A git command in `root`. Optional locks are off so a status never
/// rewrites the index: the app watches `.git`, and its own status
/// touching the index would ask for another status, forever.
fn command(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).env("GIT_OPTIONAL_LOCKS", "0");
    c
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = command(root).args(args).output().map_err(|e| format!("git: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() { Ok(text) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

/// `git status --porcelain=v1 -z`: `XY path\0`, renames adding `\0old`.
fn parse_status(raw: &[u8]) -> Vec<(String, FileStatus)> {
    let mut out = Vec::new();
    let mut parts = raw.split(|b| *b == 0).peekable();
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let index = entry[0] as char;
        let work = entry[1] as char;
        let rel = String::from_utf8_lossy(&entry[3..]).into_owned();
        if index == 'R' || index == 'C' || work == 'R' || work == 'C' {
            parts.next();
        }
        out.push((rel, FileStatus { index, work }));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn status_reply(root: &Path) -> Reply {
    let branch = match git(root, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        Ok(b) if !b.trim().is_empty() => b.trim().to_owned(),
        _ => git(root, &["rev-parse", "--short", "HEAD"]).map(|h| format!("detached {}", h.trim())).unwrap_or_else(|_| "no commits".to_owned()),
    };
    let head = git(root, &["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_default();
    let out = command(root).args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]).output();
    let (changes, error) = match out {
        Ok(o) if o.status.success() => (parse_status(&o.stdout), None),
        Ok(o) => (Vec::new(), Some(String::from_utf8_lossy(&o.stderr).trim().to_owned())),
        Err(e) => (Vec::new(), Some(format!("git: {e}"))),
    };
    Reply::Status { branch, head, changes, error }
}

fn worker(root: &Path, rx: &Receiver<Req>, tx: &Sender<Reply>, waker: Option<&Waker>) {
    while let Ok(req) = rx.recv() {
        let replies = match req {
            Req::Status => vec![status_reply(root)],
            Req::Blob(path, rel) => {
                let head = git(root, &["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_default();
                let blob = match git(root, &["show", &format!("HEAD:{rel}")]) {
                    Ok(t) => Blob::Text(t),
                    Err(_) => Blob::Missing,
                };
                vec![Reply::Blob { path, head, blob }]
            }
            Req::Run(args) => {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let ran = match git(root, &refs) {
                    Ok(o) => Reply::Ran { ok: true, output: o.trim().to_owned() },
                    Err(e) => Reply::Ran { ok: false, output: e },
                };
                vec![ran, status_reply(root)]
            }
        };
        for r in replies {
            if tx.send(r).is_err() {
                return;
            }
        }
        if let Some(w) = waker {
            w.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let raw = b" M src/a.rs\0A  src/b.rs\0?? new.txt\0R  new/name.rs\0old/name.rs\0MM both.rs\0";
        let s = parse_status(raw);
        let names: Vec<&str> = s.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(names, vec!["both.rs", "new.txt", "new/name.rs", "src/a.rs", "src/b.rs"]);
        let of = |n: &str| s.iter().find(|(r, _)| r == n).unwrap().1;
        assert!(of("src/a.rs").unstaged() && !of("src/a.rs").staged());
        assert!(of("src/b.rs").staged() && !of("src/b.rs").unstaged());
        assert!(of("new.txt").untracked() && of("new.txt").letter() == '?');
        assert!(of("both.rs").staged() && of("both.rs").unstaged() && of("both.rs").letter() == 'M');
        assert_eq!(of("new/name.rs").letter(), 'R');
    }

    #[test]
    fn talks_to_a_repository() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let sh = |args: &[&str]| {
            let o = Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        sh(&["init", "-q", "-b", "main"]);
        sh(&["config", "user.email", "t@t"]);
        sh(&["config", "user.name", "t"]);
        std::fs::write(dir.join("src/a.rs"), "one\ntwo\n").unwrap();
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("src/a.rs"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x").unwrap();
        let mut g = Git::find(&dir.join("src"), None).expect("found the repo");
        assert_eq!(g.root, dir);
        let wait = |g: &mut Git, until: &dyn Fn(&Git) -> bool| {
            for _ in 0..400 {
                g.poll();
                if until(g) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("git did not answer in time");
        };
        wait(&mut g, &|g| !g.status_pending);
        assert_eq!(g.branch, "main");
        assert_eq!(g.head.len(), 40);
        let rels: Vec<&str> = g.changes.iter().map(|c| c.rel.as_str()).collect();
        assert_eq!(rels, vec!["new.txt", "src/a.rs"]);
        assert!(g.dirty_dirs.contains(&dir.join("src")));
        assert!(g.status_of(&dir.join("src/a.rs")).unwrap().unstaged());
        assert!(g.blob(&dir.join("src/a.rs")).is_none(), "asked for, not there yet");
        wait(&mut g, &|g| g.blobs.contains_key(&dir.join("src/a.rs")));
        assert_eq!(g.blob(&dir.join("src/a.rs")), Some(&Blob::Text("one\ntwo\n".into())));
        g.run(vec!["add".into(), "new.txt".into()]);
        wait(&mut g, &|g| !g.status_pending && g.last_output.is_some());
        assert!(g.status_of(&dir.join("new.txt")).unwrap().staged());
        assert_eq!(g.staged().count(), 1);
        assert_eq!(g.unstaged().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

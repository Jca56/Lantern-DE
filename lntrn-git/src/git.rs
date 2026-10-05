//! Git through its command line. Everything here blocks: call it from
//! the worker thread. Diffs are in `diff.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// `git` in `repo`, told never to stop and ask for a password: a push
/// that needs one fails with a message instead of hanging the worker.
pub fn git(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo).env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

/// What a finished command said: its output when it worked (`stderr`
/// first, where git reports progress, else `stdout`, else `fallback`),
/// its error text when it didn't.
fn said(output: std::io::Result<Output>, fallback: &str) -> Result<String, String> {
    let output = output.map_err(|e| format!("git could not be run: {e}"))?;
    let (out, err) = (text(&output.stdout), text(&output.stderr));
    if !output.status.success() {
        return Err(if err.is_empty() { out } else { err });
    }
    Ok([err, out].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| fallback.to_owned()))
}

fn ran(output: std::io::Result<Output>) -> bool {
    output.is_ok_and(|o| o.status.success())
}

/// A changed file in the working tree. One file can be here twice: once
/// for what is staged and once for what isn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub status: FileState,
    pub staged: bool,
    pub is_submodule: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    /// Both sides of a merge changed it and git could not put them
    /// together; staging it says it has been sorted out.
    Conflict,
}

impl FileState {
    /// The letter git shows for it.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
            Self::Conflict => "!",
        }
    }
}

/// Summary of a repo's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStatus {
    pub branch: String,
    pub files: Vec<FileStatus>,
    pub ahead: u32,
    pub behind: u32,
}

/// Git repos under `~/Projects`, two folders deep.
pub fn find_repos() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut repos = Vec::new();
    scan_repos(&Path::new(&home).join("Projects"), &mut repos, 2);
    repos.sort();
    repos.dedup();
    repos
}

fn scan_repos(dir: &Path, repos: &mut Vec<PathBuf>, depth: u32) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let hidden = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
        if !p.is_dir() || hidden {
            continue;
        }
        if is_repo(&p) {
            repos.push(p);
        } else {
            scan_repos(&p, repos, depth - 1);
        }
    }
}

/// Whether `path` is the top of a work tree (a submodule's `.git` is a
/// file, not a folder).
pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// The folder's name, which is what a repo is called.
pub fn repo_name(repo: &Path) -> String {
    repo.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_owned()
}

pub fn current_branch(repo: &Path) -> String {
    git(repo).args(["branch", "--show-current"]).output().map(|o| text(&o.stdout)).unwrap_or_default()
}

/// Commits `a` has that `b` doesn't, and the other way round.
fn left_right(repo: &Path, range: &str) -> (u32, u32) {
    let Ok(output) = git(repo).args(["rev-list", "--left-right", "--count", range]).output() else { return (0, 0) };
    if !output.status.success() {
        return (0, 0);
    }
    let s = text(&output.stdout);
    let mut parts = s.split_whitespace().map(|p| p.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// The paths git tracks as submodules (mode 160000 in the index): more
/// robust than `git submodule status`, which fails on a stale
/// `.gitmodules`.
fn submodule_paths(repo: &Path) -> Vec<String> {
    let Ok(output) = git(repo).args(["ls-files", "--stage", "-z"]).output() else { return Vec::new() };
    String::from_utf8_lossy(&output.stdout).split('\0').filter(|e| e.starts_with("160000 ")).filter_map(|e| e.split('\t').nth(1).map(str::to_owned)).collect()
}

/// `git status --porcelain=v1 -z` as files. Entries are `XY path`,
/// separated by NULs and never quoted; a rename's old path follows it as
/// an entry of its own, which is skipped.
pub fn parse_status(porcelain: &str, submodules: &[String]) -> Vec<FileStatus> {
    let mut files = Vec::new();
    let mut entries = porcelain.split('\0');
    while let Some(entry) = entries.next() {
        let bytes = entry.as_bytes();
        if bytes.len() < 4 {
            continue;
        }
        let (index, worktree) = (bytes[0], bytes[1]);
        let path = entry[3..].trim_end_matches('/').to_owned();
        if index == b'R' || index == b'C' || worktree == b'R' {
            entries.next();
        }
        let is_submodule = submodules.contains(&path);
        let mut push = |status, staged| files.push(FileStatus { path: path.clone(), status, staged, is_submodule });
        if index == b'?' {
            push(FileState::Untracked, false);
            continue;
        }
        if index == b'U' || worktree == b'U' || (index == worktree && matches!(index, b'A' | b'D')) {
            push(FileState::Conflict, false);
            continue;
        }
        match index {
            b' ' | b'!' => {}
            b'A' => push(FileState::Added, true),
            b'D' => push(FileState::Deleted, true),
            b'R' | b'C' => push(FileState::Renamed, true),
            _ => push(FileState::Modified, true),
        }
        match worktree {
            b'M' | b'T' => push(FileState::Modified, false),
            b'D' => push(FileState::Deleted, false),
            _ => {}
        }
    }
    files
}

pub fn status(repo: &Path) -> RepoStatus {
    let branch = current_branch(repo);
    let (ahead, behind) = left_right(repo, "HEAD...@{upstream}");
    let submodules = submodule_paths(repo);
    let files = git(repo).args(["status", "--porcelain=v1", "-z"]).output().map(|o| parse_status(&String::from_utf8_lossy(&o.stdout), &submodules)).unwrap_or_default();
    RepoStatus { branch, files, ahead, behind }
}

pub fn stage(repo: &Path, path: &str) -> bool {
    ran(git(repo).args(["add", "--", path]).output())
}

/// Take a file out of the next commit. A repo with no commit yet has no
/// `HEAD` to restore from, so there the file is dropped from the index.
pub fn unstage(repo: &Path, path: &str) -> bool {
    ran(git(repo).args(["restore", "--staged", "--", path]).output()) || ran(git(repo).args(["rm", "--cached", "-r", "-q", "--", path]).output())
}

pub fn stage_all(repo: &Path) -> bool {
    ran(git(repo).args(["add", "-A"]).output())
}

pub fn unstage_all(repo: &Path) -> bool {
    ran(git(repo).args(["reset", "-q"]).output())
}

/// Throw away what isn't staged of a file. A tracked file goes back to
/// what is staged (or committed); an untracked one is deleted, since git
/// has no copy to go back to. Nothing brings either back.
pub fn discard(repo: &Path, file: &FileStatus) -> Result<String, String> {
    if file.staged {
        return Err("Unstage it first: only changes that aren't staged can be discarded".into());
    }
    let done = if file.status == FileState::Untracked {
        said(git(repo).args(["clean", "-f", "-d", "-q", "--", &file.path]).output(), "")
    } else {
        said(git(repo).args(["restore", "--worktree", "--", &file.path]).output(), "")
    };
    done.map(|_| format!("Discarded {}", file.path))
}

pub fn commit(repo: &Path, message: &str) -> Result<String, String> {
    let output = git(repo).args(["commit", "-m", message]).output().map_err(|e| format!("git could not be run: {e}"))?;
    // A commit that has nothing to do says so on stdout.
    if output.status.success() { Ok(text(&output.stdout)) } else { Err([text(&output.stderr), text(&output.stdout)].into_iter().find(|s| !s.is_empty()).unwrap_or_default()) }
}

/// Push the current branch. One with no upstream yet is pushed to
/// `origin` and set to track it.
pub fn push(repo: &Path) -> Result<String, String> {
    match said(git(repo).args(["push"]).output(), "Pushed") {
        Err(e) if e.contains("no upstream") || e.contains("set the remote as upstream") => push_new_branch(repo, &current_branch(repo)),
        other => other,
    }
}

pub fn push_new_branch(repo: &Path, name: &str) -> Result<String, String> {
    said(git(repo).args(["push", "-u", "origin", name]).output(), &format!("Pushed '{name}' to origin"))
}

pub fn pull(repo: &Path) -> Result<String, String> {
    said(git(repo).args(["pull"]).output(), "Pulled")
}

/// Clone `url` into a folder of its own inside `dest`.
pub fn clone_repo(url: &str, dest: &Path) -> Result<String, String> {
    if !dest.is_dir() {
        return Err(format!("There is no folder {}", dest.display()));
    }
    said(git(dest).args(["clone", "--quiet", url]).output(), "Cloned")
}

/// A new empty repo at `parent/name`, on a `main` branch.
pub fn init_repo(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err("That is not a name a repository can have".into());
    }
    if !parent.is_dir() {
        return Err(format!("There is no folder {}", parent.display()));
    }
    let path = parent.join(name);
    if path.exists() {
        return Err(format!("{} is already there", path.display()));
    }
    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    said(git(&path).args(["init", "-q", "-b", "main"]).output(), "").map(|_| path)
}

// ---- branches -------------------------------------------------------------

/// A local branch as the branches tab shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    /// Commits it has that the base branch doesn't, and the reverse.
    pub ahead: u32,
    pub behind: u32,
    /// Its newest commit's subject.
    pub last_commit: String,
    pub has_upstream: bool,
}

/// The branch the others are measured against: `main`, else `master`,
/// else the first.
pub fn base_branch(names: &[String]) -> Option<&str> {
    ["main", "master"].into_iter().find(|b| names.iter().any(|n| n == b)).or(names.first().map(String::as_str))
}

/// Every local branch with what the branches tab says about it.
pub fn branches(repo: &Path) -> Vec<Branch> {
    let format = "--format=%(HEAD)%00%(refname:short)%00%(upstream:short)%00%(subject)";
    let Ok(output) = git(repo).args(["branch", "--list", format]).output() else { return Vec::new() };
    let mut out: Vec<Branch> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\0');
            let (head, name, upstream, subject) = (parts.next()?, parts.next()?, parts.next()?, parts.next().unwrap_or(""));
            if name.starts_with('(') {
                return None;
            }
            Some(Branch { name: name.to_owned(), is_current: head == "*", has_upstream: !upstream.is_empty(), last_commit: subject.to_owned(), ..Branch::default() })
        })
        .collect();
    let names: Vec<String> = out.iter().map(|b| b.name.clone()).collect();
    if let Some(base) = base_branch(&names).map(str::to_owned) {
        for b in out.iter_mut().filter(|b| b.name != base) {
            (b.ahead, b.behind) = left_right(repo, &format!("{}...{base}", b.name));
        }
    }
    out
}

/// A name as git will take it for a branch: spaces become dashes, and
/// anything else a ref can't hold is dropped.
pub fn branch_name(typed: &str) -> String {
    let name: String = typed.trim().chars().map(|c| if c.is_whitespace() { '-' } else { c }).filter(|c| c.is_alphanumeric() || "-_./".contains(*c)).collect();
    name.trim_matches(|c| c == '/' || c == '.').to_owned()
}

/// Make a branch from where `HEAD` is and switch to it.
pub fn create_branch(repo: &Path, name: &str) -> Result<String, String> {
    said(git(repo).args(["checkout", "-q", "-b", name]).output(), "").map(|_| format!("Created '{name}' and switched to it"))
}

pub fn switch_branch(repo: &Path, name: &str) -> Result<String, String> {
    said(git(repo).args(["checkout", "-q", name]).output(), "").map(|_| format!("Switched to '{name}'"))
}

/// Merge `source` into the branch that is checked out.
pub fn merge_branch(repo: &Path, source: &str) -> Result<String, String> {
    let output = git(repo).args(["merge", source, "--no-edit"]).output().map_err(|e| format!("git could not be run: {e}"))?;
    // A merge that stops on conflicts explains itself on stdout.
    if output.status.success() { Ok(text(&output.stdout)) } else { Err([text(&output.stdout), text(&output.stderr)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")) }
}

// ---- history --------------------------------------------------------------

/// A commit for the history graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub subject: String,
    /// The refs on it, as `git log` decorates them (`HEAD -> main`,
    /// `origin/main`, `tag: v1`).
    pub decorations: Vec<String>,
}

/// One `%H%x00%h%x00%P%x00%s%x00%D` line.
pub fn parse_commit(line: &str) -> Option<Commit> {
    let parts: Vec<&str> = line.splitn(5, '\0').collect();
    let [hash, short, parents, subject, refs] = parts[..] else { return None };
    Some(Commit {
        hash: hash.to_owned(),
        short_hash: short.to_owned(),
        parents: parents.split_whitespace().map(str::to_owned).collect(),
        subject: subject.to_owned(),
        decorations: refs.split(", ").map(str::trim).filter(|d| !d.is_empty()).map(str::to_owned).collect(),
    })
}

/// The newest `count` commits of every branch, remote branch and tag
/// (and of `HEAD`, when it is on none), children before parents. Not
/// `--all`: that would list every stash as three commits of its own.
pub fn log(repo: &Path, count: usize) -> Vec<Commit> {
    let Ok(output) = git(repo).args(["log", "--topo-order", &format!("-n{count}"), "--format=%H%x00%h%x00%P%x00%s%x00%D", "--branches", "--remotes", "--tags", "HEAD"]).output() else { return Vec::new() };
    String::from_utf8_lossy(&output.stdout).lines().filter_map(parse_commit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_entries_become_files() {
        // Staged and unstaged at once, a rename (its old path is the next
        // entry), a deletion, an untracked folder, a submodule.
        let porcelain = "MM src/app.rs\0R  src/new name.rs\0src/old.rs\0 D gone.rs\0?? notes/\0A  added.rs\0UU both.rs\0 M vendor/lib\0";
        let files = parse_status(porcelain, &["vendor/lib".to_owned()]);
        let got: Vec<(&str, &str, bool)> = files.iter().map(|f| (f.path.as_str(), f.status.letter(), f.staged)).collect();
        assert_eq!(got, [("src/app.rs", "M", true), ("src/app.rs", "M", false), ("src/new name.rs", "R", true), ("gone.rs", "D", false), ("notes", "?", false), ("added.rs", "A", true), ("both.rs", "!", false), ("vendor/lib", "M", false),]);
        assert!(files.last().unwrap().is_submodule && !files[0].is_submodule);
        assert!(parse_status("", &[]).is_empty());
    }

    #[test]
    fn typed_names_become_branch_names() {
        assert_eq!(branch_name("  fix the  thing "), "fix-the--thing");
        assert_eq!(branch_name("feature/new look!"), "feature/new-look");
        assert_eq!(branch_name("/.hidden."), "hidden");
        assert_eq!(branch_name("~^:?*"), "");
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(base_branch(&names(&["dev", "master", "x"])), Some("master"));
        assert_eq!(base_branch(&names(&["dev", "main", "master"])), Some("main"));
        assert_eq!(base_branch(&names(&["dev"])), Some("dev"));
        assert_eq!(base_branch(&[]), None);
    }

    #[test]
    fn log_lines_become_commits() {
        let c = parse_commit("abc123\0abc\0p1 p2\0Merge things\0HEAD -> main, origin/main, tag: v1").unwrap();
        assert_eq!((c.hash.as_str(), c.short_hash.as_str(), c.subject.as_str()), ("abc123", "abc", "Merge things"));
        assert_eq!(c.parents, ["p1", "p2"]);
        assert_eq!(c.decorations, ["HEAD -> main", "origin/main", "tag: v1"]);
        let root = parse_commit("abc\0a\0\0First\0").unwrap();
        assert!(root.parents.is_empty() && root.decorations.is_empty());
        assert!(parse_commit("not a log line").is_none());
    }
}

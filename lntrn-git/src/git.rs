//! Git operations via CLI commands. All blocking — call from background thread.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A changed file in the working tree.
#[derive(Debug, Clone)]
pub struct FileStatus {
    pub path: String,
    pub status: FileState,
    pub staged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

impl FileState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
        }
    }
}

/// Summary of repo state.
#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub branch: String,
    pub files: Vec<FileStatus>,
    pub ahead: u32,
    pub behind: u32,
}

/// Find git repos in common locations (scans 2 levels deep).
pub fn find_repos() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let search_dirs = [
        format!("{home}/Documents/Projects"),
        format!("{home}/Projects"),
        format!("{home}/src"),
        format!("{home}/dev"),
        format!("{home}/.config"),
    ];

    let mut repos = Vec::new();
    for dir in &search_dirs {
        let path = Path::new(dir);
        if !path.is_dir() { continue; }
        scan_repos(path, &mut repos, 2);
    }
    repos.sort();
    repos.dedup();
    repos
}

fn scan_repos(dir: &Path, repos: &mut Vec<PathBuf>, depth: u32) {
    if depth == 0 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() { continue; }
        // Skip hidden dirs (except .config which we explicitly search)
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') { continue; }
        }
        if p.join(".git").exists() {
            repos.push(p);
        } else {
            scan_repos(&p, repos, depth - 1);
        }
    }
}

/// Get the current branch name.
pub fn current_branch(repo: &Path) -> String {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo)
        .output();
    output.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Get ahead/behind counts relative to upstream.
pub fn ahead_behind(repo: &Path) -> (u32, u32) {
    let output = Command::new("git")
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .current_dir(repo)
        .output();
    let Ok(output) = output else { return (0, 0) };
    if !output.status.success() { return (0, 0); }
    let s = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = s.trim().split_whitespace().collect();
    if parts.len() == 2 {
        let ahead = parts[0].parse().unwrap_or(0);
        let behind = parts[1].parse().unwrap_or(0);
        (ahead, behind)
    } else {
        (0, 0)
    }
}

/// Get full repo status.
pub fn status(repo: &Path) -> RepoStatus {
    let branch = current_branch(repo);
    let (ahead, behind) = ahead_behind(repo);

    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--ignore-submodules=dirty"])
        .current_dir(repo)
        .output();

    let mut files = Vec::new();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.len() < 4 { continue; }
            let index = line.as_bytes()[0];
            let worktree = line.as_bytes()[1];
            let path = line[3..].to_string();

            // Staged changes (index column)
            if index != b' ' && index != b'?' {
                let state = match index {
                    b'M' => FileState::Modified,
                    b'A' => FileState::Added,
                    b'D' => FileState::Deleted,
                    b'R' => FileState::Renamed,
                    _ => FileState::Modified,
                };
                files.push(FileStatus { path: path.clone(), status: state, staged: true });
            }

            // Unstaged changes (worktree column)
            if worktree == b'M' || worktree == b'D' {
                let state = if worktree == b'D' { FileState::Deleted } else { FileState::Modified };
                // Don't duplicate if already added as staged
                if index == b' ' || index == b'?' {
                    files.push(FileStatus { path: path.clone(), status: state, staged: false });
                } else {
                    files.push(FileStatus { path: path.clone(), status: state, staged: false });
                }
            }

            // Untracked
            if index == b'?' {
                files.push(FileStatus { path, status: FileState::Untracked, staged: false });
            }
        }
    }

    RepoStatus { branch, files, ahead, behind }
}

/// Stage a file.
pub fn stage(repo: &Path, path: &str) -> bool {
    Command::new("git").args(["add", path]).current_dir(repo).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

/// Unstage a file.
pub fn unstage(repo: &Path, path: &str) -> bool {
    Command::new("git").args(["restore", "--staged", path]).current_dir(repo).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

/// Commit staged changes.
pub fn commit(repo: &Path, message: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Push to remote.
pub fn push(repo: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["push"])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        // git push outputs to stderr even on success
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(if msg.is_empty() { "Pushed successfully".into() } else { msg })
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Pull from remote.
pub fn pull(repo: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["pull"])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Get the repo name from the path.
pub fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

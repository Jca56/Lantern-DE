//! Git operations via CLI commands. All blocking — call from the background worker thread.

use std::path::{Path, PathBuf};
use std::process::Command;

// ── Types ───────────────────────────────────────────────────────────────────

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

#[derive(Debug, Clone)]
pub struct FileStatus {
    pub path: String,
    pub status: FileState,
    pub staged: bool,
}

#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub branch: String,
    pub files: Vec<FileStatus>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct GraphCommit {
    pub short_hash: String,
    pub subject: String,
    pub decorations: Vec<String>,
}

// ── Repo discovery ──────────────────────────────────────────────────────────

/// Walk up from `start` to find the nearest `.git` directory.
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ── Status ──────────────────────────────────────────────────────────────────

pub fn current_branch(repo: &Path) -> String {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo)
        .output();
    output
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

pub fn ahead_behind(repo: &Path) -> (u32, u32) {
    let output = Command::new("git")
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .current_dir(repo)
        .output();
    let Ok(output) = output else { return (0, 0) };
    if !output.status.success() {
        return (0, 0);
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = s.trim().split_whitespace().collect();
    if parts.len() == 2 {
        (
            parts[0].parse().unwrap_or(0),
            parts[1].parse().unwrap_or(0),
        )
    } else {
        (0, 0)
    }
}

pub fn status(repo: &Path) -> RepoStatus {
    let branch = current_branch(repo);
    let (ahead, behind) = ahead_behind(repo);

    let output = Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(repo)
        .output();

    let mut files = Vec::new();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.len() < 4 {
                continue;
            }
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
                files.push(FileStatus {
                    path: path.clone(),
                    status: state,
                    staged: true,
                });
            }

            // Unstaged changes (worktree column)
            if worktree == b'M' || worktree == b'D' {
                let state = if worktree == b'D' {
                    FileState::Deleted
                } else {
                    FileState::Modified
                };
                files.push(FileStatus {
                    path: path.clone(),
                    status: state,
                    staged: false,
                });
            }

            // Untracked
            if index == b'?' {
                files.push(FileStatus {
                    path,
                    status: FileState::Untracked,
                    staged: false,
                });
            }
        }
    }

    RepoStatus {
        branch,
        files,
        ahead,
        behind,
    }
}

// ── Staging ─────────────────────────────────────────────────────────────────

pub fn stage(repo: &Path, path: &str) -> bool {
    Command::new("git")
        .args(["add", path])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn unstage(repo: &Path, path: &str) -> bool {
    Command::new("git")
        .args(["restore", "--staged", path])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Commit / Push / Pull ────────────────────────────────────────────────────

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

pub fn push(repo: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["push"])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Ok(if msg.is_empty() {
            "Pushed successfully".into()
        } else {
            msg
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("no upstream")
        || stderr.contains("has no upstream")
        || stderr.contains("set the remote as upstream")
    {
        let branch = current_branch(repo);
        return push_new_branch(repo, &branch);
    }
    Err(stderr.trim().to_string())
}

pub fn push_new_branch(repo: &Path, name: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["push", "-u", "origin", name])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(if msg.is_empty() {
            format!("Pushed '{name}' to origin")
        } else {
            msg
        })
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

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

// ── Branches ────────────────────────────────────────────────────────────────

pub fn list_branches(repo: &Path) -> Vec<BranchInfo> {
    let output = Command::new("git")
        .args(["branch", "--list"])
        .current_dir(repo)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            let is_current = line.starts_with('*');
            let name = line.trim_start_matches('*').trim().to_string();
            BranchInfo { name, is_current }
        })
        .filter(|b| !b.name.is_empty())
        .collect()
}

pub fn switch_branch(repo: &Path, name: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["checkout", name])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(format!("Switched to '{name}'"))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

// ── Graph ───────────────────────────────────────────────────────────────────

pub fn log_structured(repo: &Path, count: usize) -> Vec<GraphCommit> {
    let output = Command::new("git")
        .args([
            "log",
            "--all",
            "--topo-order",
            &format!("-n{count}"),
            "--format=%h%x00%P%x00%s%x00%D",
        ])
        .current_dir(repo)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\0').collect();
            if parts.len() < 4 {
                return None;
            }
            let decorations = if parts[3].is_empty() {
                Vec::new()
            } else {
                parts[3]
                    .split(", ")
                    .map(|s| s.trim().to_string())
                    .collect()
            };
            Some(GraphCommit {
                short_hash: parts[0].to_string(),
                subject: parts[2].to_string(),
                decorations,
            })
        })
        .collect()
}

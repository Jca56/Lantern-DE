//! Git operations via CLI commands. All blocking — call from background thread.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A changed file in the working tree.
#[derive(Debug, Clone)]
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
        format!("{home}/Projects"),
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

/// Detect submodule paths by checking which entries in the repo are
/// themselves git repos (have .git file or directory). More robust than
/// `git submodule status` which can fail on stale .gitmodules entries.
pub fn submodule_paths(repo: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["ls-files", "--stage"])
        .current_dir(repo)
        .output();
    let Ok(output) = output else { return Vec::new() };
    // Submodules show as mode 160000 in the index
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            if line.starts_with("160000 ") {
                // Format: "160000 <hash> <stage>\t<path>"
                line.split('\t').nth(1).map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Get full repo status.
pub fn status(repo: &Path) -> RepoStatus {
    let branch = current_branch(repo);
    let (ahead, behind) = ahead_behind(repo);

    let submodules = submodule_paths(repo);

    let output = Command::new("git")
        .args(["status", "--porcelain=v1"])
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
            let is_sub = submodules.iter().any(|s| s == &path);

            // Staged changes (index column)
            if index != b' ' && index != b'?' {
                let state = match index {
                    b'M' => FileState::Modified,
                    b'A' => FileState::Added,
                    b'D' => FileState::Deleted,
                    b'R' => FileState::Renamed,
                    _ => FileState::Modified,
                };
                files.push(FileStatus { path: path.clone(), status: state, staged: true, is_submodule: is_sub });
            }

            // Unstaged changes (worktree column)
            if worktree == b'M' || worktree == b'D' {
                let state = if worktree == b'D' { FileState::Deleted } else { FileState::Modified };
                files.push(FileStatus { path: path.clone(), status: state, staged: false, is_submodule: is_sub });
            }

            // Untracked
            if index == b'?' {
                files.push(FileStatus { path, status: FileState::Untracked, staged: false, is_submodule: is_sub });
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

// ── GitHub remote repos ─────────────────────────────────────────────────────

/// A remote GitHub repository.
#[derive(Debug, Clone)]
pub struct RemoteRepo {
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub clone_url: String,
    pub is_private: bool,
    pub is_fork: bool,
}

/// Fetch the authenticated user's GitHub repos via `gh` CLI.
pub fn fetch_github_repos() -> Result<Vec<RemoteRepo>, String> {
    let output = Command::new("gh")
        .args([
            "repo", "list", "--limit", "200",
            "--json", "name,nameWithOwner,description,url,isPrivate,isFork",
        ])
        .output()
        .map_err(|e| format!("gh not found: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    // Minimal JSON parsing — avoid pulling in serde/serde_json
    let json = String::from_utf8_lossy(&output.stdout);
    parse_gh_repo_list(&json)
}

/// Clone a repo into the given directory.
pub fn clone_repo(url: &str, dest: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["clone", url])
        .current_dir(dest)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok("Cloned successfully".into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Tiny JSON parser for gh repo list output.
/// Format: [{"name":"...","nameWithOwner":"...","description":"...","url":"...","isPrivate":bool,"isFork":bool}, ...]
fn parse_gh_repo_list(json: &str) -> Result<Vec<RemoteRepo>, String> {
    let json = json.trim();
    if !json.starts_with('[') {
        return Err("unexpected gh output".into());
    }

    let mut repos = Vec::new();
    // Split by "},{" to get individual objects
    let inner = &json[1..json.len() - 1]; // strip [ ]
    if inner.trim().is_empty() {
        return Ok(repos);
    }

    for chunk in split_json_objects(inner) {
        let name = extract_json_str(&chunk, "name").unwrap_or_default();
        let full_name = extract_json_str(&chunk, "nameWithOwner").unwrap_or_default();
        let description = extract_json_str(&chunk, "description").unwrap_or_default();
        let clone_url = extract_json_str(&chunk, "url").unwrap_or_default();
        let is_private = extract_json_bool(&chunk, "isPrivate");
        let is_fork = extract_json_bool(&chunk, "isFork");
        repos.push(RemoteRepo { name, full_name, description, clone_url, is_private, is_fork });
    }

    Ok(repos)
}

fn split_json_objects(s: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => {
                if depth == 0 { start = i; }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    objects.push(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    objects
}

fn extract_json_str(obj: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\":", key);
    let pos = obj.find(&needle)? + needle.len();
    let rest = &obj[pos..].trim_start();
    if rest.starts_with("null") {
        return Some(String::new());
    }
    if !rest.starts_with('"') { return None; }
    let inner = &rest[1..];
    let mut result = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    match escaped {
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        'n' => result.push(' '),
                        _ => { result.push('\\'); result.push(escaped); }
                    }
                }
            }
            '"' => break,
            _ => result.push(c),
        }
    }
    Some(result)
}

fn extract_json_bool(obj: &str, key: &str) -> bool {
    let needle = format!("\"{}\":", key);
    let Some(pos) = obj.find(&needle) else { return false };
    let rest = &obj[pos + needle.len()..].trim_start();
    rest.starts_with("true")
}

/// Get the repo name from the path.
pub fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

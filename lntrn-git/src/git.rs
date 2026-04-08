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

/// Push to remote. If no upstream is set, automatically pushes with `-u origin <branch>`.
pub fn push(repo: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["push"])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Ok(if msg.is_empty() { "Pushed successfully".into() } else { msg });
    }
    // If push failed due to no upstream, auto-set it
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("no upstream") || stderr.contains("has no upstream") || stderr.contains("set the remote as upstream") {
        let branch = current_branch(repo);
        return push_new_branch(repo, &branch);
    }
    Err(stderr.trim().to_string())
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

// ── Branch operations ───────────────────────────────────────────────────────

/// A local branch.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
}

/// List all local branches.
pub fn list_branches(repo: &Path) -> Vec<BranchInfo> {
    let output = Command::new("git")
        .args(["branch", "--list"])
        .current_dir(repo)
        .output();
    let Ok(output) = output else { return Vec::new() };
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

/// Create a new branch and switch to it.
pub fn create_branch(repo: &Path, name: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["checkout", "-b", name])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(format!("Created and switched to '{name}'"))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Push a new branch to origin with upstream tracking.
pub fn push_new_branch(repo: &Path, name: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["push", "-u", "origin", name])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(if msg.is_empty() { format!("Pushed '{name}' to origin") } else { msg })
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Switch to an existing branch.
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

// ── Detailed branch info ────────────────────────────────────────────────────

/// Detailed branch info for the branch panel.
#[derive(Debug, Clone)]
pub struct BranchDetail {
    pub name: String,
    pub is_current: bool,
    pub ahead: u32,
    pub behind: u32,
    pub last_commit: String,
    pub has_upstream: bool,
}

/// Get ahead/behind counts between two branches.
pub fn ahead_behind_branches(repo: &Path, a: &str, b: &str) -> (u32, u32) {
    let output = Command::new("git")
        .args(["rev-list", "--left-right", "--count", &format!("{a}...{b}")])
        .current_dir(repo)
        .output();
    let Ok(output) = output else { return (0, 0) };
    if !output.status.success() { return (0, 0); }
    let s = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = s.trim().split_whitespace().collect();
    if parts.len() == 2 {
        (parts[0].parse().unwrap_or(0), parts[1].parse().unwrap_or(0))
    } else {
        (0, 0)
    }
}

/// List all branches with ahead/behind relative to main (or master).
pub fn list_branches_detailed(repo: &Path) -> Vec<BranchDetail> {
    let branches = list_branches(repo);
    if branches.is_empty() { return Vec::new(); }

    // Find the base branch name (main or master)
    let base = branches.iter()
        .find(|b| b.name == "main" || b.name == "master")
        .map(|b| b.name.clone())
        .unwrap_or_else(|| branches[0].name.clone());

    // Get last commit subject per branch
    let output = Command::new("git")
        .args(["branch", "--format=%(refname:short)\t%(subject)", "--list"])
        .current_dir(repo)
        .output();
    let subjects: Vec<(String, String)> = output.map(|o| {
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(2, '\t');
                let name = parts.next()?.to_string();
                let subject = parts.next().unwrap_or("").to_string();
                Some((name, subject))
            })
            .collect()
    }).unwrap_or_default();

    branches.iter().map(|b| {
        let (ahead, behind) = if b.name == base {
            (0, 0)
        } else {
            ahead_behind_branches(repo, &b.name, &base)
        };
        let last_commit = subjects.iter()
            .find(|(n, _)| n == &b.name)
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        let has_upstream = Command::new("git")
            .args(["config", &format!("branch.{}.remote", b.name)])
            .current_dir(repo)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        BranchDetail {
            name: b.name.clone(),
            is_current: b.is_current,
            ahead, behind, last_commit, has_upstream,
        }
    }).collect()
}

/// Merge another branch into the current branch.
pub fn merge_branch(repo: &Path, source: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["merge", source, "--no-edit"])
        .current_dir(repo)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

// ── Commit graph data ───────────────────────────────────────────────────────

/// A commit for graph rendering.
#[derive(Debug, Clone)]
pub struct GraphCommit {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub decorations: Vec<String>,
}

/// Get structured commit data for graph rendering.
pub fn log_structured(repo: &Path, count: usize) -> Vec<GraphCommit> {
    // NUL-separated fields, record separator between commits
    let output = Command::new("git")
        .args([
            "log", "--all", "--topo-order",
            &format!("-n{count}"),
            "--format=%H%x00%h%x00%P%x00%s%x00%D",
        ])
        .current_dir(repo)
        .output();
    let Ok(output) = output else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().filter_map(|line| {
        let parts: Vec<&str> = line.splitn(5, '\0').collect();
        if parts.len() < 5 { return None; }
        let parents = if parts[2].is_empty() {
            Vec::new()
        } else {
            parts[2].split(' ').map(|s| s.to_string()).collect()
        };
        let decorations = if parts[4].is_empty() {
            Vec::new()
        } else {
            parts[4].split(", ").map(|s| s.trim().to_string()).collect()
        };
        Some(GraphCommit {
            hash: parts[0].to_string(),
            short_hash: parts[1].to_string(),
            parents,
            subject: parts[3].to_string(),
            decorations,
        })
    }).collect()
}

/// Get the repo name from the path.
pub fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

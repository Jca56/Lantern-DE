//! GitHub integration via the `gh` CLI — repo listing and creation.
//! All blocking — call from the background worker thread.

use std::path::Path;
use std::process::Command;

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
            "repo",
            "list",
            "--limit",
            "200",
            "--json",
            "name,nameWithOwner,description,url,isPrivate,isFork",
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

/// Create the repo on GitHub via `gh` and link it as the `origin` remote.
/// Works on an empty repo — no push is attempted, just the remote link.
pub fn create_github_repo(repo: &Path, name: &str, private: bool) -> Result<String, String> {
    let vis = if private { "--private" } else { "--public" };
    let output = Command::new("gh")
        .args([
            "repo", "create", name, vis, "--source", ".", "--remote", "origin",
        ])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("gh not found: {e}"))?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(if url.is_empty() {
            "Created on GitHub".into()
        } else {
            format!("Created {url}")
        })
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
        repos.push(RemoteRepo {
            name,
            full_name,
            description,
            clone_url,
            is_private,
            is_fork,
        });
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
                if depth == 0 {
                    start = i;
                }
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
    if !rest.starts_with('"') {
        return None;
    }
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
                        _ => {
                            result.push('\\');
                            result.push(escaped);
                        }
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
    let Some(pos) = obj.find(&needle) else {
        return false;
    };
    let rest = &obj[pos + needle.len()..].trim_start();
    rest.starts_with("true")
}

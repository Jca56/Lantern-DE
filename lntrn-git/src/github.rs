//! GitHub through the `gh` command: listing the user's repos and making
//! new ones. Blocking: call from the worker thread.

use std::path::Path;
use std::process::Command;

use lntrn_data::{Doc, json};

/// A repository on GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteRepo {
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub clone_url: String,
    pub is_private: bool,
    pub is_fork: bool,
}

fn gh_missing(e: std::io::Error) -> String {
    format!("The gh command could not be run ({e}). Install it and sign in with `gh auth login`.")
}

/// The signed-in user's repos, newest first as `gh` lists them.
pub fn list_repos() -> Result<Vec<RemoteRepo>, String> {
    let output = Command::new("gh").args(["repo", "list", "--limit", "200", "--json", "name,nameWithOwner,description,url,isPrivate,isFork"]).output().map_err(gh_missing)?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    parse_repo_list(&String::from_utf8_lossy(&output.stdout))
}

/// `gh repo list --json ...` as repos.
pub fn parse_repo_list(text: &str) -> Result<Vec<RemoteRepo>, String> {
    let doc = json::parse(text).map_err(|e| format!("gh said something unexpected: {e}"))?;
    let list = doc.as_list().ok_or("gh said something unexpected: not a list")?;
    let word = |d: &Doc, key: &str| d.get(key).and_then(Doc::as_str).unwrap_or("").to_owned();
    let flag = |d: &Doc, key: &str| d.get(key).and_then(Doc::as_bool).unwrap_or(false);
    Ok(list
        .iter()
        .map(|d| RemoteRepo {
            name: word(d, "name"),
            full_name: word(d, "nameWithOwner"),
            // A description is one line here, whatever it holds.
            description: word(d, "description").split_whitespace().collect::<Vec<_>>().join(" "),
            clone_url: word(d, "url"),
            is_private: flag(d, "isPrivate"),
            is_fork: flag(d, "isFork"),
        })
        .filter(|r| !r.name.is_empty() && !r.clone_url.is_empty())
        .collect())
}

/// Make the repo on GitHub and set it as `origin`. Nothing is pushed: it
/// works on a repo with no commit yet.
pub fn create_repo(repo: &Path, name: &str, private: bool) -> Result<String, String> {
    let visibility = if private { "--private" } else { "--public" };
    let output = Command::new("gh").args(["repo", "create", name, visibility, "--source", ".", "--remote", "origin"]).current_dir(repo).output().map_err(gh_missing)?;
    if output.status.success() { Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_output_becomes_repos() {
        let text = r#"[{"description":"A \"desktop\"\nfor fun","isFork":false,"isPrivate":true,"name":"Lantern-DE","nameWithOwner":"Jca56/Lantern-DE","url":"https://github.com/Jca56/Lantern-DE"},
            {"description":null,"isFork":true,"isPrivate":false,"name":"llama.cpp","nameWithOwner":"Jca56/llama.cpp","url":"https://github.com/Jca56/llama.cpp"}]"#;
        let repos = parse_repo_list(text).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0], RemoteRepo { name: "Lantern-DE".into(), full_name: "Jca56/Lantern-DE".into(), description: "A \"desktop\" for fun".into(), clone_url: "https://github.com/Jca56/Lantern-DE".into(), is_private: true, is_fork: false });
        assert!(repos[1].description.is_empty() && repos[1].is_fork && !repos[1].is_private);
        assert_eq!(parse_repo_list("[]").unwrap(), []);
        assert!(parse_repo_list("gh: not logged in").is_err());
    }
}

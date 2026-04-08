//! Background git worker thread — runs blocking git operations off the UI thread.

use std::path::PathBuf;
use std::sync::mpsc;

use crate::git;

/// Events from the background git thread.
pub enum GitEvent {
    Repos(Vec<PathBuf>),
    Status(git::RepoStatus),
    Branches(Vec<git::BranchInfo>),
    BranchDetails(Vec<git::BranchDetail>),
    GraphData(Vec<git::GraphCommit>),
    Message(String),
    Error(String),
    RemoteRepos(Result<Vec<git::RemoteRepo>, String>),
}

/// Commands to the background git thread.
pub enum GitCmd {
    FindRepos,
    OpenRepo(PathBuf),
    Refresh,
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit(String),
    Push,
    Pull,
    FetchGitHubRepos,
    ListBranches,
    ListBranchesDetailed,
    FetchGraph(usize),
    CreateBranch(String, bool),
    SwitchBranch(String),
    Merge { source: String, target: String },
}

/// Spawn the worker thread — returns the command sender and event receiver.
pub fn spawn() -> (mpsc::Sender<GitCmd>, mpsc::Receiver<GitEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("git-worker".into())
        .spawn(move || run(event_tx, cmd_rx))
        .expect("spawn git worker");

    (cmd_tx, event_rx)
}

fn run(tx: mpsc::Sender<GitEvent>, rx: mpsc::Receiver<GitCmd>) {
    let mut repo_path: Option<PathBuf> = None;

    loop {
        let cmd = match rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => return,
        };

        match cmd {
            GitCmd::FindRepos => {
                let repos = git::find_repos();
                let _ = tx.send(GitEvent::Repos(repos));
            }
            GitCmd::OpenRepo(path) => {
                repo_path = Some(path.clone());
                let status = git::status(&path);
                let branches = git::list_branches(&path);
                let _ = tx.send(GitEvent::Status(status));
                let _ = tx.send(GitEvent::Branches(branches));
            }
            GitCmd::Refresh => {
                if let Some(ref path) = repo_path {
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::Stage(file) => {
                if let Some(ref path) = repo_path {
                    git::stage(path, &file);
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::Unstage(file) => {
                if let Some(ref path) = repo_path {
                    git::unstage(path, &file);
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::StageAll => {
                if let Some(ref path) = repo_path {
                    let _ = std::process::Command::new("git")
                        .args(["add", "-A"])
                        .current_dir(path)
                        .output();
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::UnstageAll => {
                if let Some(ref path) = repo_path {
                    let _ = std::process::Command::new("git")
                        .args(["reset", "HEAD"])
                        .current_dir(path)
                        .output();
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::Commit(msg) => {
                if let Some(ref path) = repo_path {
                    match git::commit(path, &msg) {
                        Ok(out) => { let _ = tx.send(GitEvent::Message(out)); }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::Push => {
                if let Some(ref path) = repo_path {
                    match git::push(path) {
                        Ok(out) => { let _ = tx.send(GitEvent::Message(out)); }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::Pull => {
                if let Some(ref path) = repo_path {
                    match git::pull(path) {
                        Ok(out) => { let _ = tx.send(GitEvent::Message(out)); }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::FetchGitHubRepos => {
                let result = git::fetch_github_repos();
                let _ = tx.send(GitEvent::RemoteRepos(result));
            }
            GitCmd::ListBranches => {
                if let Some(ref path) = repo_path {
                    let branches = git::list_branches(path);
                    let _ = tx.send(GitEvent::Branches(branches));
                }
            }
            GitCmd::ListBranchesDetailed => {
                if let Some(ref path) = repo_path {
                    let details = git::list_branches_detailed(path);
                    let _ = tx.send(GitEvent::BranchDetails(details));
                }
            }
            GitCmd::FetchGraph(count) => {
                if let Some(ref path) = repo_path {
                    let commits = git::log_structured(path, count);
                    let _ = tx.send(GitEvent::GraphData(commits));
                }
            }
            GitCmd::CreateBranch(name, push) => {
                if let Some(ref path) = repo_path {
                    match git::create_branch(path, &name) {
                        Ok(mut msg) => {
                            if push {
                                match git::push_new_branch(path, &name) {
                                    Ok(push_msg) => { msg = format!("{msg} — {push_msg}"); }
                                    Err(push_err) => {
                                        let _ = tx.send(GitEvent::Error(push_err));
                                        let branches = git::list_branches(path);
                                        let _ = tx.send(GitEvent::Branches(branches));
                                        continue;
                                    }
                                }
                            }
                            let _ = tx.send(GitEvent::Message(msg));
                            let branches = git::list_branches(path);
                            let _ = tx.send(GitEvent::Branches(branches));
                        }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::SwitchBranch(name) => {
                if let Some(ref path) = repo_path {
                    match git::switch_branch(path, &name) {
                        Ok(msg) => {
                            let _ = tx.send(GitEvent::Message(msg));
                            let status = git::status(path);
                            let branches = git::list_branches(path);
                            let _ = tx.send(GitEvent::Status(status));
                            let _ = tx.send(GitEvent::Branches(branches));
                        }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::Merge { source, target } => {
                if let Some(ref path) = repo_path {
                    // Switch to target branch first (if not already on it)
                    let current = git::current_branch(path);
                    if current != target {
                        if let Err(err) = git::switch_branch(path, &target) {
                            let _ = tx.send(GitEvent::Error(err));
                            continue;
                        }
                    }
                    match git::merge_branch(path, &source) {
                        Ok(msg) => {
                            let _ = tx.send(GitEvent::Message(msg));
                            let status = git::status(path);
                            let _ = tx.send(GitEvent::Status(status));
                        }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
        }
    }
}

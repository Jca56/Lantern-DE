//! Background git worker thread — runs blocking git operations off the UI thread.

use std::path::PathBuf;
use std::sync::mpsc;

use crate::git;

/// Events from the background git thread.
pub enum GitEvent {
    Repos(Vec<PathBuf>),
    Status(git::RepoStatus),
    Branches(Vec<git::BranchInfo>),
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
    CreateBranch(String),
    SwitchBranch(String),
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
            GitCmd::CreateBranch(name) => {
                if let Some(ref path) = repo_path {
                    match git::create_branch(path, &name) {
                        Ok(msg) => {
                            let _ = tx.send(GitEvent::Message(msg));
                            // Refresh branches + status after create
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
        }
    }
}

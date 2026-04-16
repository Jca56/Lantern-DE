//! Background git worker thread — runs blocking git operations off the UI thread.

use std::path::PathBuf;
use std::sync::mpsc;

use super::ops;

/// Events sent from the worker back to the UI thread.
pub enum GitEvent {
    Status(ops::RepoStatus),
    Branches(Vec<ops::BranchInfo>),
    GraphData(Vec<ops::GraphCommit>),
    Message(String),
    Error(String),
}

/// Commands sent from the UI thread to the worker.
pub enum GitCmd {
    OpenRepo(PathBuf),
    Refresh,
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit(String),
    Push,
    Pull,
    FetchGraph(usize),
    SwitchBranch(String),
}

/// Spawn the worker thread. `wake` is called after every event to prod the UI.
pub fn spawn(
    wake: impl Fn() + Send + 'static,
) -> (mpsc::Sender<GitCmd>, mpsc::Receiver<GitEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("git-worker".into())
        .spawn(move || run(event_tx, cmd_rx, wake))
        .expect("spawn git worker");

    (cmd_tx, event_rx)
}

fn send(tx: &mpsc::Sender<GitEvent>, wake: &dyn Fn(), event: GitEvent) {
    let _ = tx.send(event);
    wake();
}

fn run(tx: mpsc::Sender<GitEvent>, rx: mpsc::Receiver<GitCmd>, wake: impl Fn()) {
    let mut repo_path: Option<PathBuf> = None;

    loop {
        let cmd = match rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => return,
        };

        match cmd {
            GitCmd::OpenRepo(path) => {
                repo_path = Some(path.clone());
                let status = ops::status(&path);
                let branches = ops::list_branches(&path);
                send(&tx, &wake, GitEvent::Status(status));
                send(&tx, &wake, GitEvent::Branches(branches));
            }
            GitCmd::Refresh => {
                if let Some(ref path) = repo_path {
                    let status = ops::status(path);
                    let branches = ops::list_branches(path);
                    send(&tx, &wake, GitEvent::Status(status));
                    send(&tx, &wake, GitEvent::Branches(branches));
                }
            }
            GitCmd::Stage(file) => {
                if let Some(ref path) = repo_path {
                    ops::stage(path, &file);
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::Unstage(file) => {
                if let Some(ref path) = repo_path {
                    ops::unstage(path, &file);
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::StageAll => {
                if let Some(ref path) = repo_path {
                    let _ = std::process::Command::new("git")
                        .args(["add", "-A"])
                        .current_dir(path)
                        .output();
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::UnstageAll => {
                if let Some(ref path) = repo_path {
                    let _ = std::process::Command::new("git")
                        .args(["reset", "HEAD"])
                        .current_dir(path)
                        .output();
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::Commit(msg) => {
                if let Some(ref path) = repo_path {
                    match ops::commit(path, &msg) {
                        Ok(out) => send(&tx, &wake, GitEvent::Message(out)),
                        Err(err) => send(&tx, &wake, GitEvent::Error(err)),
                    }
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::Push => {
                if let Some(ref path) = repo_path {
                    match ops::push(path) {
                        Ok(out) => send(&tx, &wake, GitEvent::Message(out)),
                        Err(err) => send(&tx, &wake, GitEvent::Error(err)),
                    }
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::Pull => {
                if let Some(ref path) = repo_path {
                    match ops::pull(path) {
                        Ok(out) => send(&tx, &wake, GitEvent::Message(out)),
                        Err(err) => send(&tx, &wake, GitEvent::Error(err)),
                    }
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                }
            }
            GitCmd::FetchGraph(count) => {
                if let Some(ref path) = repo_path {
                    let commits = ops::log_structured(path, count);
                    send(&tx, &wake, GitEvent::GraphData(commits));
                }
            }
            GitCmd::SwitchBranch(name) => {
                if let Some(ref path) = repo_path {
                    match ops::switch_branch(path, &name) {
                        Ok(msg) => send(&tx, &wake, GitEvent::Message(msg)),
                        Err(err) => send(&tx, &wake, GitEvent::Error(err)),
                    }
                    send(&tx, &wake, GitEvent::Status(ops::status(path)));
                    send(&tx, &wake, GitEvent::Branches(ops::list_branches(path)));
                }
            }
        }
    }
}

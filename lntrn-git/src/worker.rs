//! The worker thread: every git and GitHub command runs here, off the UI
//! thread, one at a time in the order asked. What comes of each goes back
//! as [`Event`]s, and the window is woken to show them.

use std::path::PathBuf;
use std::cell::Cell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, OnceLock};

use lntrn_app::Waker;

use crate::diff::{self, Diff};
use crate::git::{self, FileStatus};
use crate::github;

/// What the UI asks for. Commands about "the repo" mean the one last
/// opened with [`Cmd::Open`].
pub enum Cmd {
    FindRepos,
    Open(PathBuf),
    Refresh,
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Discard(FileStatus),
    /// Commit what is staged; `push` it too when that worked.
    Commit { message: String, push: bool },
    Push,
    Pull,
    /// The diff of one file. `tag` comes back with it, so an answer to a
    /// question since overtaken can be told from a current one.
    Diff { file: FileStatus, tag: u64 },
    History(usize),
    /// Make a branch and switch to it; `push` sets it up on `origin` too.
    CreateBranch { name: String, push: bool },
    SwitchBranch(String),
    /// Merge `source` into `target`, switching to `target` first.
    Merge { source: String, target: String },
    GitHubRepos,
    Clone { url: String, name: String, dest: PathBuf },
    CreateRepo { name: String, parent: PathBuf, github: bool, private: bool },
}

/// What came of a command.
pub enum Event {
    Repos(Vec<PathBuf>),
    Status(git::RepoStatus),
    Branches(Vec<git::Branch>),
    History(Vec<git::Commit>),
    Diff { file: FileStatus, tag: u64, diff: Diff },
    /// Something worked and git had this to say about it.
    Done(String),
    Failed(String),
    GitHubRepos(Result<Vec<github::RemoteRepo>, String>),
    /// A clone finished: where the new repo is, or why not.
    Cloned(Result<PathBuf, String>),
    /// A repo was made: where, and what GitHub said if it was asked to
    /// make one too and couldn't (the local one is there either way).
    Created(Result<(PathBuf, Option<String>), String>),
    /// Nothing is waiting: this many commands have been run in all.
    Idle(u64),
}

/// The UI's end of the worker.
pub struct Link {
    tx: Sender<Cmd>,
    rx: Receiver<(u64, Event)>,
    waker: Arc<OnceLock<Waker>>,
    /// How many commands have been sent, and how many of them the worker
    /// has said are done.
    sent: Cell<u64>,
    done: Cell<u64>,
}

impl Link {
    /// Start the worker thread.
    pub fn spawn() -> Link {
        let (tx, cmd_rx) = channel();
        let (event_tx, rx) = channel();
        let waker = Arc::new(OnceLock::new());
        let wake = waker.clone();
        let spawned = std::thread::Builder::new().name("git-worker".into()).spawn(move || run(cmd_rx, event_tx, wake));
        if let Err(e) = spawned {
            lntrn_core::log_error!("the git worker could not start: {e}");
        }
        Link { tx, rx, waker, sent: Cell::new(0), done: Cell::new(0) }
    }

    /// The loop's waker, so an event shows without waiting for input.
    pub fn set_waker(&self, waker: Waker) {
        let _ = self.waker.set(waker);
    }

    pub fn send(&self, cmd: Cmd) {
        self.sent.set(self.sent.get() + 1);
        let _ = self.tx.send(cmd);
    }

    /// Whether anything sent is still to be done, as of the last
    /// [`Self::take`].
    pub fn busy(&self) -> bool {
        self.done.get() < self.sent.get()
    }

    /// The events that have arrived since the last call, each with how
    /// many [`Cmd::Open`]s the worker had run when it was made: what is
    /// said about a repo since left can be told from what is said about
    /// the one that is open.
    pub fn take(&self) -> Vec<(u64, Event)> {
        let events: Vec<(u64, Event)> = self.rx.try_iter().collect();
        for (_, event) in &events {
            if let Event::Idle(done) = event {
                self.done.set(*done);
            }
        }
        events
    }
}

fn run(rx: Receiver<Cmd>, tx: Sender<(u64, Event)>, waker: Arc<OnceLock<Waker>>) {
    let mut repo: Option<PathBuf> = None;
    let (mut opens, mut done) = (0u64, 0u64);
    let mut next = rx.recv().ok();
    while let Some(cmd) = next {
        let mut events = Vec::new();
        if matches!(cmd, Cmd::Open(_)) {
            opens += 1;
        }
        handle(cmd, &mut repo, &mut events);
        done += 1;
        // The next command, if one is waiting; else say all is done and
        // sleep until one comes.
        next = match rx.try_recv() {
            Ok(cmd) => Some(cmd),
            Err(TryRecvError::Empty) => {
                events.push(Event::Idle(done));
                None
            }
            Err(TryRecvError::Disconnected) => return,
        };
        for event in events {
            if tx.send((opens, event)).is_err() {
                return;
            }
        }
        if let Some(w) = waker.get() {
            w.wake();
        }
        if next.is_none() {
            next = rx.recv().ok();
        }
    }
}

/// Run one command, pushing what came of it onto `out`.
fn handle(cmd: Cmd, repo: &mut Option<PathBuf>, out: &mut Vec<Event>) {
    // Commands that need no repo open.
    let cmd = match cmd {
        Cmd::FindRepos => return out.push(Event::Repos(git::find_repos())),
        Cmd::GitHubRepos => return out.push(Event::GitHubRepos(github::list_repos())),
        Cmd::Clone { url, name, dest } => return out.push(Event::Cloned(git::clone_repo(&url, &dest).map(|_| dest.join(name)))),
        Cmd::CreateRepo { name, parent, github, private } => {
            let made = git::init_repo(&parent, &name).map(|path| {
                let complaint = if github { github::create_repo(&path, &name, private).err() } else { None };
                (path, complaint)
            });
            return out.push(Event::Created(made));
        }
        Cmd::Open(path) => {
            out.push(Event::Status(git::status(&path)));
            out.push(Event::Branches(git::branches(&path)));
            *repo = Some(path);
            return;
        }
        other => other,
    };
    let Some(path) = repo.as_deref() else { return };
    // What a command that changes things reports: its outcome, then the
    // repo as it now is.
    let mut report = |result: Result<String, String>, branches: bool| {
        out.push(match result {
            Ok(said) => Event::Done(said),
            Err(why) => Event::Failed(why),
        });
        out.push(Event::Status(git::status(path)));
        if branches {
            out.push(Event::Branches(git::branches(path)));
        }
    };
    match cmd {
        Cmd::Refresh => {
            out.push(Event::Status(git::status(path)));
            out.push(Event::Branches(git::branches(path)));
        }
        Cmd::Stage(file) => {
            git::stage(path, &file);
            out.push(Event::Status(git::status(path)));
        }
        Cmd::Unstage(file) => {
            git::unstage(path, &file);
            out.push(Event::Status(git::status(path)));
        }
        Cmd::StageAll => {
            git::stage_all(path);
            out.push(Event::Status(git::status(path)));
        }
        Cmd::UnstageAll => {
            git::unstage_all(path);
            out.push(Event::Status(git::status(path)));
        }
        Cmd::Discard(file) => report(git::discard(path, &file), false),
        Cmd::Commit { message, push } => {
            let done = git::commit(path, &message).and_then(|said| if push { git::push(path).map(|_| format!("{} Pushed.", first_line(&said))) } else { Ok(said) });
            report(done, true);
        }
        Cmd::Push => report(git::push(path), true),
        Cmd::Pull => report(git::pull(path), true),
        Cmd::Diff { file, tag } => {
            let diff = diff::of(path, &file);
            out.push(Event::Diff { file, tag, diff });
        }
        Cmd::History(count) => out.push(Event::History(git::log(path, count))),
        Cmd::CreateBranch { name, push } => {
            let done = git::create_branch(path, &name).and_then(|said| if push { git::push_new_branch(path, &name).map(|_| format!("{said}. Pushed to origin.")) } else { Ok(said) });
            report(done, true);
        }
        Cmd::SwitchBranch(name) => report(git::switch_branch(path, &name), true),
        Cmd::Merge { source, target } => {
            let on_target = if git::current_branch(path) == target { Ok(String::new()) } else { git::switch_branch(path, &target) };
            report(on_target.and_then(|_| git::merge_branch(path, &source)), true);
        }
        Cmd::FindRepos | Cmd::GitHubRepos | Cmd::Clone { .. } | Cmd::CreateRepo { .. } | Cmd::Open(_) => {}
    }
}

/// The first line of what git said: enough for a toast.
pub fn first_line(said: &str) -> &str {
    said.lines().next().unwrap_or("").trim()
}

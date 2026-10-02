//! Background file operations: copy, move and permanent delete.
//!
//! Everything that can take time runs on a worker thread (`ops/worker.rs`)
//! so the window stays responsive; the UI thread only resolves conflicts and
//! queues an [`OpRequest`]. Copies run one at a time and the rest wait their
//! turn in the [`OpQueue`]: two copies hammering the same stick or phone
//! only slow each other down. A move that is a plain rename, and a delete
//! on a local disk, do not wait behind a long copy: they run beside it.
//! Each operation is finalised on its own when it ends (undo entries, sudo
//! fallback, failure report, reload). Undo and redo run here as well
//! (`push_history`): what they do is in `undo/exec.rs`.
//!
//! What the worker guarantees:
//!  - It never overwrites. A target that exists is only replaced when the
//!    item says `replace`, and then the old item goes to the Trash.
//!  - A copy is built under a hidden staging name next to its target and
//!    renamed into place once complete, so a real name never holds a partial
//!    item: not after a failure, not after a cancel, not if Fox dies.
//!    (Slow mounts are written in place instead, exclusively created, and
//!    the partial is removed on failure: staging would cost an MTP rename
//!    per item that not every device honours.)
//!  - Replace: the old item is trashed only after the new one is complete,
//!    and comes back if the last step fails.
//!  - A cross-device move is copy, land, then delete from the source only
//!    what verifiably arrived (`copy_tree::remove_moved_source`).
//!  - Cancel is honoured between entries and between chunks of a file; the
//!    item in flight is rolled back.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

pub use crate::copy_tree::EntryFailure as OpFailure;
use crate::trash::Trashed;

mod history;
mod worker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    /// Permanent delete. Only `OpItem::src` is used.
    Delete,
    /// An undo or a redo (`OpQueue::push_history`). It has no items of its
    /// own; its result is `OpOutcome::history`.
    History,
}

/// One source and the exact place the user chose for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpItem {
    pub src: PathBuf,
    pub target: PathBuf,
    /// The user answered a name conflict with Replace: whatever is at
    /// `target` goes to the Trash once the new item is complete.
    pub replace: bool,
}

impl OpItem {
    pub fn delete(path: PathBuf) -> Self {
        Self {
            src: path,
            target: PathBuf::new(),
            replace: false,
        }
    }
}

pub struct OpRequest {
    pub kind: OpKind,
    /// Shown in the status strip: "Copying", "Moving", "Deleting"...
    pub label: &'static str,
    pub items: Vec<OpItem>,
    /// The folder the items go to (or, for a delete, were in).
    pub dest: PathBuf,
    /// A clipboard paste took the clipboard; it gets these back when the
    /// copy ends, unless something else was copied in the meantime.
    pub rearm_clipboard: Option<Vec<PathBuf>>,
}

/// Everything the UI needs to finalise an operation.
#[derive(Debug, Default)]
pub struct OpOutcome {
    /// Items that landed by copying: (source, target). For a move these
    /// crossed a filesystem boundary.
    pub created: Vec<(PathBuf, PathBuf)>,
    /// Moves done by a rename on one filesystem: (source, target).
    pub renamed: Vec<(PathBuf, PathBuf)>,
    /// Old items a Replace moved to the Trash.
    pub replaced: Vec<Trashed>,
    /// Items stopped by a permission error, with the target the user chose,
    /// for the sudo fallback.
    pub perm_fails: Vec<OpItem>,
    /// Everything else that did not happen, with the reason.
    pub failures: Vec<OpFailure>,
    pub cancelled: bool,
    /// The result of an undo or redo (`OpKind::History` only).
    pub history: Option<crate::undo::Outcome>,
}

impl OpOutcome {
    fn fail(&mut self, path: &Path, reason: &str) {
        self.failures.push(OpFailure {
            path: path.to_path_buf(),
            reason: reason.to_string(),
        });
    }
}

/// Events the worker sends to the main loop.
#[derive(Debug)]
enum OpProgress {
    StartedItem {
        index: usize,
        total: usize,
        name: String,
    },
    Done(OpOutcome),
}

/// The operation that is running right now.
pub struct OpHandle {
    rx: Receiver<OpProgress>,
    cancel: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    /// Live snapshot of the latest StartedItem, for the status strip.
    pub current_name: String,
    pub index: usize,
    pub total: usize,
    pub label: &'static str,
    kind: OpKind,
    dest: PathBuf,
    rearm_clipboard: Option<Vec<PathBuf>>,
    /// Names this operation will create, so a later paste does not claim
    /// one of them while it is still on its way.
    targets: Vec<PathBuf>,
    outcome: Option<OpOutcome>,
    disconnected: bool,
}

impl OpHandle {
    fn drain(&mut self) -> bool {
        let mut dirty = false;
        loop {
            match self.rx.try_recv() {
                Ok(OpProgress::StartedItem { index, total, name }) => {
                    self.index = index;
                    self.total = total;
                    self.current_name = name;
                    dirty = true;
                }
                Ok(OpProgress::Done(outcome)) => {
                    self.outcome = Some(outcome);
                    dirty = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.disconnected = true;
                    break;
                }
            }
        }
        dirty
    }

    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.index as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn cancelling(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn ended(&self) -> bool {
        self.outcome.is_some() || self.disconnected
    }

    fn finish(mut self) -> FinishedOp {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        // A worker that went away without its Done (a panic) must not
        // wedge the queue or vanish without a word.
        let outcome = self.outcome.take().unwrap_or_else(|| {
            let mut out = OpOutcome::default();
            out.fail(&self.dest, "the operation stopped unexpectedly");
            out
        });
        FinishedOp {
            kind: self.kind,
            dest: self.dest,
            rearm_clipboard: self.rearm_clipboard,
            outcome,
        }
    }

    fn stop_and_wait(mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// An operation that has ended, handed to `App::finish_op`.
pub struct FinishedOp {
    pub kind: OpKind,
    pub dest: PathBuf,
    pub rearm_clipboard: Option<Vec<PathBuf>>,
    pub outcome: OpOutcome,
}

/// The running operations plus the ones waiting their turn.
#[derive(Default)]
pub struct OpQueue {
    /// The one heavy operation (a copy, a cross-device move) running now.
    active: Option<OpHandle>,
    /// Light operations running beside it; see `runs_beside`.
    beside: Vec<OpHandle>,
    waiting: VecDeque<OpRequest>,
}

/// A move that is nothing but renames on one local filesystem, or a delete
/// on a local disk: these must not sit behind a twenty-minute copy before
/// anything visibly happens. Everything that streams data, and everything
/// on a slow mount (one transfer at a time is all a phone can do), queues.
fn runs_beside(req: &OpRequest) -> bool {
    let slow = crate::fs::is_slow_path;
    match req.kind {
        OpKind::Copy => false,
        OpKind::Move => req.items.iter().all(|i| {
            !slow(&i.src) && !slow(&i.target) && worker::same_filesystem(&i.src, &i.target)
        }),
        OpKind::Delete => req.items.iter().all(|i| !slow(&i.src)),
        OpKind::History => true,
    }
}

impl OpQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `req`, or line it up behind the running copy.
    pub fn push(&mut self, req: OpRequest) {
        if req.items.is_empty() {
            return;
        }
        if runs_beside(&req) {
            self.beside.push(spawn(req));
        } else if self.active.is_none() {
            self.active = Some(spawn(req));
        } else {
            self.waiting.push_back(req);
        }
    }

    /// Start an undo or a redo. It runs beside whatever else is going on:
    /// its steps are renames (the one exception, copying an item again
    /// because its trashed copy is gone, is rare), and a Ctrl+Z that waited
    /// behind a long copy would look dead. The caller starts them one at a
    /// time (`UndoStack::next`).
    pub fn push_history(&mut self, job: crate::undo::Job) {
        let (label, total, dest) = (job.label(), job.len(), job.folder());
        self.beside.push(start(
            OpKind::History,
            label,
            total,
            dest,
            None,
            Vec::new(),
            move |tx, cancel| history::run(job, tx, cancel, &crate::trash::trash),
        ));
    }

    pub fn is_busy(&self) -> bool {
        self.active.is_some() || !self.beside.is_empty()
    }

    /// The operation the status strip shows (and its cancel button stops).
    pub fn shown(&self) -> Option<&OpHandle> {
        self.active.as_ref().or(self.beside.first())
    }

    /// Operations other than the shown one: running beside it or queued.
    pub fn others(&self) -> usize {
        let running = self.active.iter().count() + self.beside.len();
        (running + self.waiting.len()).saturating_sub(1)
    }

    /// True when a running or queued operation is going to create `target`.
    /// The disk cannot say so yet.
    pub fn is_reserved(&self, target: &Path) -> bool {
        self.active
            .iter()
            .chain(&self.beside)
            .any(|h| h.targets.iter().any(|t| t == target))
            || self
                .waiting
                .iter()
                .any(|r| r.kind != OpKind::Delete && r.items.iter().any(|i| i.target == target))
    }

    /// Drain worker events. Returns whether anything changed on screen and
    /// the operations that ended; the next queued one is started here.
    pub fn poll(&mut self) -> (bool, Vec<FinishedOp>) {
        let mut dirty = false;
        let mut finished = Vec::new();
        let mut i = 0;
        while i < self.beside.len() {
            dirty |= self.beside[i].drain();
            if self.beside[i].ended() {
                finished.push(self.beside.remove(i).finish());
                dirty = true;
            } else {
                i += 1;
            }
        }
        while let Some(handle) = self.active.as_mut() {
            dirty |= handle.drain();
            if !handle.ended() {
                break;
            }
            if let Some(handle) = self.active.take() {
                finished.push(handle.finish());
            }
            // A queued request may be a light one by now (its copy-shaped
            // neighbours are what made it wait); the lanes sort that out.
            while self.active.is_none() {
                match self.waiting.pop_front() {
                    Some(next) => self.push(next),
                    None => break,
                }
            }
            dirty = true;
        }
        (dirty, finished)
    }

    /// Stop the operation the strip shows: its item in flight is rolled
    /// back, the ones already done stay. Everything else carries on.
    pub fn cancel_shown(&self) {
        if let Some(h) = self.shown() {
            h.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Stop every running operation and drop the queued ones, which have
    /// not touched anything yet.
    pub fn cancel_all(&mut self) {
        for h in self.active.iter().chain(&self.beside) {
            h.cancel.store(true, Ordering::SeqCst);
        }
        self.waiting.clear();
    }

    /// Last thing before the process exits: stop, and wait until every
    /// worker has rolled back its current item. Without the wait the
    /// threads would be killed mid-write.
    pub fn shutdown(&mut self) {
        self.cancel_all();
        for handle in self.active.take().into_iter().chain(self.beside.drain(..)) {
            handle.stop_and_wait();
        }
    }
}

fn spawn(req: OpRequest) -> OpHandle {
    let total = req.items.len();
    let targets = match req.kind {
        OpKind::Delete => Vec::new(),
        _ => req.items.iter().map(|i| i.target.clone()).collect(),
    };
    let (kind, items) = (req.kind, req.items);
    start(
        kind,
        req.label,
        total,
        req.dest,
        req.rearm_clipboard,
        targets,
        move |tx, cancel| worker::run(kind, items, tx, cancel, &crate::trash::trash),
    )
}

/// Put `body` on a worker thread of its own and return the handle the
/// queue polls. `body` reports through the sender and ends with `Done`.
fn start(
    kind: OpKind,
    label: &'static str,
    total: usize,
    dest: PathBuf,
    rearm_clipboard: Option<Vec<PathBuf>>,
    targets: Vec<PathBuf>,
    body: impl FnOnce(mpsc::Sender<OpProgress>, &AtomicBool) + Send + 'static,
) -> OpHandle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let join = thread::Builder::new()
        .name("fox-ops".into())
        // Folder copies recurse once per directory level.
        .stack_size(8 * 1024 * 1024)
        .spawn(move || body(tx, &flag))
        .expect("spawn fox-ops thread");

    OpHandle {
        rx,
        cancel,
        join: Some(join),
        current_name: String::new(),
        index: 0,
        total,
        label,
        kind,
        dest,
        rearm_clipboard,
        targets,
        outcome: None,
        disconnected: false,
    }
}

#[cfg(test)]
mod tests;

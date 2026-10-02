//! Carrying out one undo or redo. Runs on the ops worker.
//!
//! Each item of the action is reversed on its own and ends in one of four
//! ways: done (it moves to the other stack), nothing left to do (the item
//! is gone), refused (with the reason, shown to the user), or waiting for
//! root (a rename the user may not make; offered to sudo afterwards). A
//! cancel leaves the items not reached yet on the stack they came from.
//!
//! The rules, for every step in both directions:
//!  - a move is a no-replace rename: a name that is taken is never written
//!    over, the step is refused instead;
//!  - an item is only moved or trashed when it is still the file the action
//!    made or moved (`Stamp`); a stranger under the same name is left alone;
//!  - nothing is deleted: undoing a copy or a New File/Folder moves the item
//!    to the Trash, and redo takes it out again. The one removal is `rmdir`
//!    of a created folder where there is no Trash, which the system refuses
//!    unless the folder is empty.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::steps::{recopy, recreate, retrash, shift, uncopy, uncreate, untrash};
use super::words::{describe, name_of};
use super::{Direction, Entry, Job, Note, RootWait, UndoAction};
use crate::ops::OpFailure;
use crate::trash::{TrashError, Trashed};

/// What the worker lends to a job. The Trash and the copier are parameters
/// so the tests can run against a scratch trash.
pub struct Env<'a> {
    pub cancel: &'a AtomicBool,
    /// Move an item to the Trash of its drive.
    pub trash: &'a dyn Fn(&Path) -> Result<Trashed, TrashError>,
    /// Copy `src` to the free name `dst` (redo of a copy whose trashed copy
    /// is gone). True when the copy is in place; the failures are what did
    /// not make it.
    pub copy: &'a dyn Fn(&Path, &Path) -> (bool, Vec<OpFailure>),
    /// (index, total, name) before each item.
    pub progress: &'a mut dyn FnMut(usize, usize, &str),
}

/// The result of a job, for `UndoStack::finish`.
#[derive(Debug)]
pub struct Outcome {
    pub dir: Direction,
    pub(super) generation: u64,
    /// The items that were reversed, as the entry for the other stack.
    pub done: Option<Entry>,
    /// The items not attempted (cancelled): they stay where they were.
    pub rest: Option<Entry>,
    /// Renames that need root.
    pub root: Option<RootWait>,
    /// Items that could not be reversed, and why.
    pub failures: Vec<OpFailure>,
    /// Items with nothing left to reverse (already gone).
    pub gone: usize,
    pub cancelled: bool,
}

impl Outcome {
    /// One line for the status bar, or `None` when the failure notice or
    /// the password prompt already tells the story.
    pub fn summary(&self) -> Option<String> {
        if let Some(done) = &self.done {
            return Some(describe(self.dir, done));
        }
        if self.root.is_some() || !self.failures.is_empty() {
            return None;
        }
        let word = match self.dir {
            Direction::Undo => "undo",
            Direction::Redo => "redo",
        };
        Some(if self.cancelled {
            match self.dir {
                Direction::Undo => "Undo stopped".to_string(),
                Direction::Redo => "Redo stopped".to_string(),
            }
        } else if self.gone == 1 {
            format!("Nothing to {word}: the item is no longer there")
        } else {
            format!("Nothing to {word}: the items are no longer there")
        })
    }
}

/// How one item ended.
pub(super) enum Step<T> {
    /// Reversed: the item as the other stack records it.
    Done(T, Note),
    /// A rename (from, to) refused for lack of permission, and what the
    /// item becomes once sudo has made it.
    Root(PathBuf, PathBuf, T, Note),
    /// Nothing was there to reverse.
    Gone,
    /// Refused: the path the reason is about, and the reason.
    Failed(PathBuf, String),
    /// Cancelled in the middle; nothing was changed.
    Stopped,
}

struct Walked<T> {
    done: Vec<(T, Note)>,
    rest: Vec<(T, Note)>,
    root: Vec<(PathBuf, PathBuf, T, Note)>,
}

/// Reverse `items` one by one until done or cancelled.
fn walk<T: Clone>(
    items: &[T],
    notes: &[Note],
    env: &mut Env,
    out: &mut Outcome,
    shown: impl Fn(&T) -> &Path,
    mut step: impl FnMut(&T, &Note, &Env, &mut Vec<OpFailure>) -> Step<T>,
) -> Walked<T> {
    let mut walked = Walked {
        done: Vec::new(),
        rest: Vec::new(),
        root: Vec::new(),
    };
    let total = items.len();
    for (i, item) in items.iter().enumerate() {
        let note = notes.get(i).cloned().unwrap_or_default();
        if out.cancelled || env.cancel.load(Ordering::SeqCst) {
            out.cancelled = true;
            walked.rest.push((item.clone(), note));
            continue;
        }
        (env.progress)(i, total, &name_of(shown(item)));
        match step(item, &note, env, &mut out.failures) {
            Step::Done(item, note) => walked.done.push((item, note)),
            Step::Root(from, to, item, note) => walked.root.push((from, to, item, note)),
            Step::Gone => out.gone += 1,
            Step::Failed(path, reason) => out.failures.push(OpFailure { path, reason }),
            Step::Stopped => {
                out.cancelled = true;
                walked.rest.push((item.clone(), note));
            }
        }
    }
    walked
}

fn entry_of<T>(
    items: Vec<(T, Note)>,
    make: &impl Fn(Vec<T>) -> Option<UndoAction>,
) -> Option<Entry> {
    if items.is_empty() {
        return None;
    }
    let (items, notes): (Vec<T>, Vec<Note>) = items.into_iter().unzip();
    make(items).map(|action| Entry { action, notes })
}

impl Outcome {
    fn take<T>(&mut self, walked: Walked<T>, make: impl Fn(Vec<T>) -> Option<UndoAction>) {
        self.done = entry_of(walked.done, &make);
        self.rest = entry_of(walked.rest, &make);
        let mut moves = Vec::new();
        let mut items = Vec::new();
        for (from, to, item, note) in walked.root {
            moves.push((from, to));
            items.push((item, note));
        }
        self.root = entry_of(items, &make).map(|entry| RootWait {
            dir: self.dir,
            generation: self.generation,
            moves,
            entry,
        });
    }
}

/// Run one undo or redo to the end (or to a cancel).
pub fn run(job: Job, env: &mut Env) -> Outcome {
    use Direction::{Redo, Undo};
    let Job {
        dir,
        entry,
        generation,
    } = job;
    let Entry { action, notes } = entry;
    let mut out = Outcome {
        dir,
        generation,
        done: None,
        rest: None,
        root: None,
        failures: Vec::new(),
        gone: 0,
        cancelled: false,
    };
    match action {
        UndoAction::Rename { from, to } => {
            let items = [(from, to)];
            let walked = walk(
                &items,
                &notes,
                env,
                &mut out,
                |(from, to)| if dir == Undo { to } else { from },
                |item, note, _, _| match dir {
                    Undo => shift(&item.1, &item.0, note, item.clone()),
                    Redo => shift(&item.0, &item.1, note, item.clone()),
                },
            );
            out.take(walked, |mut v| {
                v.pop().map(|(from, to)| UndoAction::Rename { from, to })
            });
        }
        UndoAction::Move(pairs) => {
            let walked = walk(
                &pairs,
                &notes,
                env,
                &mut out,
                |(_, dst)| dst,
                |item, note, _, _| match dir {
                    Undo => shift(&item.1, &item.0, note, item.clone()),
                    Redo => shift(&item.0, &item.1, note, item.clone()),
                },
            );
            out.take(walked, |v| Some(UndoAction::Move(v)));
        }
        UndoAction::Trash(items) => {
            let walked = walk(
                &items,
                &notes,
                env,
                &mut out,
                |(original, _, _)| original,
                |item, note, env, _| match dir {
                    Undo => untrash(item, note),
                    Redo => retrash(&item.0, note, env),
                },
            );
            out.take(walked, |v| Some(UndoAction::Trash(v)));
        }
        UndoAction::Create(items) => {
            let walked = walk(
                &items,
                &notes,
                env,
                &mut out,
                |(path, _)| path,
                |item, note, env, _| match dir {
                    Undo => uncreate(&item.0, item.1, note, env),
                    Redo => recreate(&item.0, item.1, note),
                },
            );
            out.take(walked, |v| Some(UndoAction::Create(v)));
        }
        UndoAction::Copy(pairs) => {
            let walked = walk(
                &pairs,
                &notes,
                env,
                &mut out,
                |(_, created)| created,
                |item, note, env, failures| match dir {
                    Undo => uncopy(item, note, env),
                    Redo => recopy(item, note, env, failures),
                },
            );
            out.take(walked, |v| Some(UndoAction::Copy(v)));
        }
    }
    out
}

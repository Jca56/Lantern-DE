//! Undo and redo of file operations.
//!
//! The stack records what was done ([`UndoAction`]) and, per item, what it
//! takes to reverse it safely ([`Note`]): which file on disk the item is,
//! and, once a created item has been undone, where in the Trash it went.
//! The reversing itself (`exec`) runs on the ops worker, never on the UI
//! thread; the stack only hands out one job at a time and takes its result.
//!
//! What undo and redo keep to:
//!  - Nothing is deleted. Undoing a copy or a "New Folder" moves what was
//!    made to the Trash; redo takes it out again.
//!  - Nothing is overwritten. Every move back is a no-replace rename: when a
//!    newer item has the name, the step is refused and reported.
//!  - An item that is no longer the one the action made or moved (same
//!    name, another file) is left alone.
//!  - An action changes stacks only for the items that really were reversed.

use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::ops::OpFailure;
use crate::trash::Trashed;

pub mod exec;
mod steps;
mod words;

pub use exec::Outcome;

/// A recorded file operation that can be undone/redone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoAction {
    /// File or folder was renamed.
    Rename { from: PathBuf, to: PathBuf },
    /// Files were moved to trash. Each entry: (original_path, trash_file_path, trash_info_path).
    Trash(Vec<(PathBuf, PathBuf, PathBuf)>),
    /// Empty files or folders were created (New File, New Folder). Each
    /// entry: (path, is_dir) — the kind is recorded, not guessed from the name.
    Create(Vec<(PathBuf, bool)>),
    /// Files were moved (cut+paste). Each entry: (source, destination).
    Move(Vec<(PathBuf, PathBuf)>),
    /// Files were copied (copy+paste or duplicate). Each entry: (source,
    /// created destination), only for the items that actually landed.
    Copy(Vec<(PathBuf, PathBuf)>),
}

impl UndoAction {
    pub fn len(&self) -> usize {
        match self {
            UndoAction::Rename { .. } => 1,
            UndoAction::Trash(v) => v.len(),
            UndoAction::Create(v) => v.len(),
            UndoAction::Move(v) | UndoAction::Copy(v) => v.len(),
        }
    }

    /// Where each item is once the action has been done, in item order.
    fn places(&self) -> Vec<&Path> {
        match self {
            UndoAction::Rename { to, .. } => vec![to.as_path()],
            UndoAction::Trash(v) => v.iter().map(|(_, file, _)| file.as_path()).collect(),
            UndoAction::Create(v) => v.iter().map(|(path, _)| path.as_path()).collect(),
            UndoAction::Move(v) | UndoAction::Copy(v) => {
                v.iter().map(|(_, dst)| dst.as_path()).collect()
            }
        }
    }

    /// Every path the action names.
    fn paths(&self) -> Vec<&Path> {
        match self {
            UndoAction::Rename { from, to } => vec![from.as_path(), to.as_path()],
            UndoAction::Trash(v) => v
                .iter()
                .flat_map(|(original, file, _)| [original.as_path(), file.as_path()])
                .collect(),
            UndoAction::Create(v) => v.iter().map(|(path, _)| path.as_path()).collect(),
            UndoAction::Move(v) | UndoAction::Copy(v) => v
                .iter()
                .flat_map(|(a, b)| [a.as_path(), b.as_path()])
                .collect(),
        }
    }

    /// The action with only the items `keep` says yes to (by index).
    fn filtered(self, keep: impl Fn(usize) -> bool) -> Option<UndoAction> {
        fn pick<T>(v: Vec<T>, keep: impl Fn(usize) -> bool) -> Vec<T> {
            v.into_iter()
                .enumerate()
                .filter(|(i, _)| keep(*i))
                .map(|(_, item)| item)
                .collect()
        }
        let action = match self {
            UndoAction::Rename { from, to } => {
                return keep(0).then_some(UndoAction::Rename { from, to })
            }
            UndoAction::Trash(v) => UndoAction::Trash(pick(v, keep)),
            UndoAction::Create(v) => UndoAction::Create(pick(v, keep)),
            UndoAction::Move(v) => UndoAction::Move(pick(v, keep)),
            UndoAction::Copy(v) => UndoAction::Copy(pick(v, keep)),
        };
        (action.len() > 0).then_some(action)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

impl Direction {
    /// For the failure notice: "2 items could not be undone".
    pub fn verb(self) -> &'static str {
        match self {
            Direction::Undo => "undone",
            Direction::Redo => "redone",
        }
    }
}

/// Which file on disk an item is, so a later undo can tell it from another
/// file that has taken its name since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    dev: u64,
    ino: u64,
    dir: bool,
    len: u64,
    mtime: (i64, i64),
    /// When the file was made, where the filesystem records it.
    born: Option<std::time::SystemTime>,
}

impl Stamp {
    fn from_meta(meta: &std::fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            dir: meta.is_dir(),
            len: meta.len(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            born: meta.created().ok(),
        }
    }

    /// The stamp of what is at `path` now. `None` when nothing is there, and
    /// on a phone or a network folder: their inode numbers are made up per
    /// look-up, and this may run on the UI thread, where they are never
    /// touched.
    pub fn of(path: &Path) -> Option<Self> {
        if crate::fs::is_slow_path(path) {
            return None;
        }
        std::fs::symlink_metadata(path)
            .ok()
            .map(|m| Self::from_meta(&m))
    }

    /// Is `now` the item this stamp was taken of? The inode says so, however
    /// much the item was edited in place. Two corrections:
    ///  - a file deleted and made anew can be given the old inode number
    ///    again (ext4 does that readily); its time of birth then differs;
    ///  - FAT drives hand out a new inode number when an entry falls out of
    ///    the kernel's cache; there an untouched file is still recognised
    ///    by its size and timestamp.
    fn same_item(&self, now: &Stamp) -> bool {
        if self.dev != now.dev || self.dir != now.dir {
            return false;
        }
        if matches!((self.born, now.born), (Some(then), Some(born)) if then != born) {
            return false;
        }
        self.ino == now.ino || (!self.dir && self.len == now.len && self.mtime == now.mtime)
    }
}

/// What the stack knows about one item of an action.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    /// The item's identity, when it could be taken.
    pub stamp: Option<Stamp>,
    /// Undo moved a created or copied item to the Trash: where it is, so
    /// redo can take it out again.
    pub parked: Option<Trashed>,
}

/// One action on a stack, with a note per item (same order as the items).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub action: UndoAction,
    pub notes: Vec<Note>,
}

impl Entry {
    /// A freshly done action: note which files its items are right now.
    fn record(action: UndoAction) -> Self {
        let notes = action
            .places()
            .into_iter()
            .map(|place| Note {
                stamp: Stamp::of(place),
                parked: None,
            })
            .collect();
        Self { action, notes }
    }

    /// The entry with only the items `keep` says yes to.
    fn filtered(self, keep: impl Fn(usize) -> bool) -> Option<Entry> {
        let notes = self
            .notes
            .into_iter()
            .enumerate()
            .filter(|(i, _)| keep(*i))
            .map(|(_, note)| note)
            .collect();
        self.action
            .filtered(keep)
            .map(|action| Entry { action, notes })
    }

    /// The folders this entry's items live in, for refreshing the views.
    fn folders(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let parked = self
            .notes
            .iter()
            .filter_map(|n| n.parked.as_ref())
            .map(|t| t.trashed.as_path());
        for path in self.action.paths().into_iter().chain(parked) {
            if let Some(parent) = path.parent() {
                if !out.iter().any(|p| p == parent) {
                    out.push(parent.to_path_buf());
                }
            }
        }
        out
    }
}

/// One undo or redo, handed to the worker.
#[derive(Debug)]
pub struct Job {
    pub dir: Direction,
    pub entry: Entry,
    generation: u64,
}

impl Job {
    /// Shown in the status strip while it runs.
    pub fn label(&self) -> &'static str {
        match self.dir {
            Direction::Undo => "Undoing",
            Direction::Redo => "Redoing",
        }
    }

    pub fn len(&self) -> usize {
        self.entry.action.len()
    }

    /// A folder the job works in, for the queue's bookkeeping.
    pub fn folder(&self) -> PathBuf {
        self.entry.folders().into_iter().next().unwrap_or_default()
    }
}

/// Renames an undo or redo could not make for lack of permission. They are
/// offered to sudo; what they amount to is only recorded once they are seen
/// to have happened.
#[derive(Debug)]
pub struct RootWait {
    dir: Direction,
    generation: u64,
    /// (from, to), one per item of `entry`. Never onto an existing name.
    pub moves: Vec<(PathBuf, PathBuf)>,
    entry: Entry,
}

/// What `UndoStack::next` found to do.
#[derive(Debug)]
pub enum Next {
    Job(Job),
    /// The stack for this direction is empty.
    Nothing(Direction),
}

/// What the UI shows and does once an undo or redo has ended.
#[derive(Debug, Default)]
pub struct Report {
    /// One line for the status bar. `None` when a notice or the password
    /// prompt says it all.
    pub summary: Option<String>,
    pub failures: Vec<OpFailure>,
    /// Renames to offer to sudo (see [`RootWait`]).
    pub root_moves: Vec<(PathBuf, PathBuf)>,
    /// Folders whose listing changed.
    pub folders: Vec<PathBuf>,
}

const MAX_UNDO: usize = 50;

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Bumped by every newly recorded action. An undo that was already on
    /// its way when the user did something new must not land on the redo
    /// stack: redo would replay it on top of the newer action.
    generation: u64,
    /// An undo or redo is on the worker. They run strictly one at a time,
    /// in the order asked: a later entry often needs the earlier one gone
    /// (a Replace is undone as "take the copy away", then "put the old item
    /// back").
    running: bool,
    /// Ctrl+Z / Ctrl+Shift+Z presses not started yet.
    queued: VecDeque<Direction>,
    root_wait: Option<RootWait>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an action that has just been done.
    pub fn push(&mut self, action: UndoAction) {
        if action.len() == 0 {
            return;
        }
        self.generation += 1;
        self.redo.clear();
        self.push_undo(Entry::record(action));
    }

    fn push_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The user asked for an undo or a redo. It starts when `next` hands it
    /// out, after the ones asked before it.
    pub fn request(&mut self, dir: Direction) {
        // A held key must not pile up more than the stack can hold.
        if self.queued.len() < MAX_UNDO {
            self.queued.push_back(dir);
        }
    }

    /// Renames of an earlier undo are still waiting for the password prompt.
    pub fn waiting_for_root(&self) -> bool {
        self.root_wait.is_some()
    }

    /// Whether a privileged move of exactly these (from, to) pairs is the
    /// one an undo is waiting for.
    pub fn root_wait_is(&self, moves: &[(&Path, &Path)]) -> bool {
        self.root_wait.as_ref().is_some_and(|w| {
            w.moves.len() == moves.len()
                && w.moves
                    .iter()
                    .zip(moves)
                    .all(|((from, to), (f, t))| from == f && to == t)
        })
    }

    /// The password prompt for the waiting renames is gone without an
    /// answer for them: they did not happen and are not recorded.
    pub fn forget_root_wait(&mut self) {
        self.root_wait = None;
    }

    /// The next undo or redo to run, if one was asked for and none is
    /// running. The caller hands a `Job` to the worker and gives the result
    /// to [`finish`](Self::finish).
    pub fn next(&mut self) -> Option<Next> {
        if self.running || self.root_wait.is_some() {
            return None;
        }
        let dir = self.queued.pop_front()?;
        let entry = match dir {
            Direction::Undo => self.undo.pop(),
            Direction::Redo => self.redo.pop(),
        };
        Some(match entry {
            Some(entry) => {
                self.running = true;
                Next::Job(Job {
                    dir,
                    entry,
                    generation: self.generation,
                })
            }
            None => Next::Nothing(dir),
        })
    }

    /// The worker went away without a result. What it had in hand is not
    /// put back on either stack: nobody knows how far it got.
    pub fn abandon(&mut self) {
        self.running = false;
    }

    /// Take the result of a job: what was reversed goes to the other stack,
    /// what was not attempted returns to its own, the rest is dropped (the
    /// report says why each of those could not be done).
    pub fn finish(&mut self, out: Outcome) -> Report {
        self.running = false;
        let summary = out.summary();
        let Outcome {
            dir,
            generation,
            done,
            rest,
            root,
            failures,
            ..
        } = out;
        let mut folders = Vec::new();
        for entry in done.iter().chain(root.iter().map(|w| &w.entry)) {
            for folder in entry.folders() {
                if !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
        }
        let fresh = generation == self.generation;
        if let Some(rest) = rest {
            match dir {
                Direction::Undo => self.push_undo(rest),
                Direction::Redo if fresh => self.redo.push(rest),
                Direction::Redo => {}
            }
        }
        if let Some(done) = done {
            self.land(dir, fresh, done);
        }
        let root_moves = root.as_ref().map(|w| w.moves.clone()).unwrap_or_default();
        self.root_wait = root;
        Report {
            summary,
            failures,
            root_moves,
            folders,
        }
    }

    /// A reversed entry arrives on the other stack.
    fn land(&mut self, dir: Direction, fresh: bool, entry: Entry) {
        match dir {
            Direction::Undo if fresh => self.redo.push(entry),
            Direction::Undo => {}
            // A redo is an action done (again): it can always be undone.
            Direction::Redo => self.push_undo(entry),
        }
    }

    /// The privileged renames have been tried (or given up on). `made` says
    /// for each (from, to) whether it happened; those that did are recorded
    /// like any other reversed item. Returns the line for the status bar.
    pub fn settle_root(&mut self, made: impl Fn(&Path, &Path) -> bool) -> Option<String> {
        let wait = self.root_wait.take()?;
        let hits: Vec<bool> = wait.moves.iter().map(|(from, to)| made(from, to)).collect();
        let entry = wait.entry.filtered(|i| hits.get(i).copied().unwrap_or(false))?;
        let summary = words::describe(wait.dir, &entry);
        self.land(wait.dir, wait.generation == self.generation, entry);
        Some(summary)
    }
}

#[cfg(test)]
mod stack_tests;
#[cfg(test)]
mod tests;

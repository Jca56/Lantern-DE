//! Paste/move conflict resolution state machine.
//!
//! When the user pastes (or drag-drops) multiple files into a folder that
//! already contains some of those names, we don't want to silently fail.
//! Instead: pop a per-file Replace / Keep Both / Skip dialog with an
//! "Apply to all remaining" checkbox.
//!
//! The flow is stateful because the dialog is asynchronous from the user's
//! perspective: `start_paste` walks the source list, pausing the first time
//! it hits a collision and stashing the unresolved sources in
//! `App.pending_paste`. The dialog choice handler resumes that walk.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug)]
pub enum PasteMode {
    Copy,
    Cut,
}

#[derive(Clone, Copy, Debug)]
pub enum ConflictAction {
    Replace,
    KeepBoth,
    Skip,
}

/// In-progress paste state. Stays on App while a conflict dialog is open.
///
/// Resolving conflicts changes nothing on disk: each source only gets its
/// final target and whether it replaces what is there. The work happens
/// afterwards, on the ops worker (or through sudo), so cancelling the paste
/// at any dialog leaves everything as it was.
#[derive(Clone, Debug)]
pub struct PendingPaste {
    pub mode: PasteMode,
    pub dest: PathBuf,
    /// Sources still to process. Drained as we go.
    pub remaining: Vec<PathBuf>,
    /// If set, all remaining conflicts are auto-resolved with this action.
    pub apply_to_all: Option<ConflictAction>,
    /// Original full source list — kept so the clipboard can be re-armed
    /// after the paste completes or is cancelled.
    pub originals: Vec<PathBuf>,
    /// True when this paste took the clipboard (Ctrl+V), false for a
    /// drag-drop. Only a clipboard paste hands the clipboard back afterwards.
    pub from_clipboard: bool,
    /// Sources with their conflicts resolved, ready to run.
    pub resolved: Vec<crate::ops::OpItem>,
    /// Root mode: the resolved items run through sudo instead of the worker.
    pub privileged: bool,
    /// If the paste originated from a drag-drop onto a non-current tab,
    /// reload that tab too after the operation completes.
    pub reload_tab: Option<usize>,
}

impl PendingPaste {
    pub fn new(mode: PasteMode, dest: PathBuf, sources: Vec<PathBuf>) -> Self {
        Self {
            mode,
            dest,
            remaining: sources.clone(),
            apply_to_all: None,
            originals: sources,
            from_clipboard: false,
            resolved: Vec::new(),
            privileged: false,
            reload_tab: None,
        }
    }

    /// True if an earlier item of this paste was already assigned `target`.
    /// Its copy has not run yet, so the name is taken even though nothing
    /// exists on disk.
    pub fn is_reserved(&self, target: &Path) -> bool {
        self.resolved.iter().any(|item| item.target == target)
    }
}

/// In-progress rename waiting on the conflict dialog's choice.
#[derive(Clone, Debug)]
pub struct PendingRename {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// State the dialog renders from. Built when a conflict is hit.
#[derive(Clone, Debug)]
pub struct ConflictDialog {
    pub target: PathBuf,
    pub source_meta: ConflictMeta,
    pub target_meta: ConflictMeta,
    pub apply_to_all: bool,
    pub remaining_count: usize,
    /// The existing item sits where there is no Trash (a phone, a network
    /// folder). Replace would have to delete it, so it is not offered.
    pub no_trash: bool,
}

impl ConflictDialog {
    pub fn new(src: &Path, target: &Path, remaining_count: usize) -> Self {
        Self {
            target: target.to_path_buf(),
            source_meta: ConflictMeta::read(src),
            target_meta: ConflictMeta::read(target),
            apply_to_all: false,
            remaining_count,
            no_trash: crate::fs::is_slow_path(target),
        }
    }

    pub fn replace_allowed(&self) -> bool {
        !self.no_trash
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConflictMeta {
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub is_dir: bool,
    /// For a folder: how many entries it holds directly. `None` when that
    /// was not counted (a slow mount, an unreadable folder).
    pub items: Option<usize>,
}

/// Counting stops here; the dialog then says "N+".
pub const ITEM_COUNT_CAP: usize = 10_000;

impl ConflictMeta {
    pub fn read(path: &Path) -> Self {
        // lstat: a link is described as itself. Replace acts on the link.
        let m = std::fs::symlink_metadata(path).ok();
        let is_dir = m.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let items = if is_dir && !crate::fs::is_slow_path(path) {
            std::fs::read_dir(path)
                .ok()
                .map(|rd| rd.take(ITEM_COUNT_CAP).count())
        } else {
            None
        };
        Self {
            size: m.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: m.as_ref().and_then(|m| m.modified().ok()),
            is_dir,
            items,
        }
    }
}

/// Generate a "Keep Both" target by suffixing the basename: foo.txt → foo (2).txt.
/// Counts up until a free slot is found, capped at 1000 to avoid loops.
/// `reserved` reports names that are spoken for but not on disk yet.
pub fn unique_keep_both_path(target: &Path, reserved: impl Fn(&Path) -> bool) -> PathBuf {
    // lstat: a dangling link holds its name too.
    let taken = |p: &Path| std::fs::symlink_metadata(p).is_ok() || reserved(p);
    if !taken(target) {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or(Path::new("."));
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = target
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for n in 2u32..1000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !taken(&candidate) {
            return candidate;
        }
    }
    parent.join(format!("{stem} (copy){ext}"))
}

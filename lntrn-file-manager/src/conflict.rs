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
#[derive(Clone, Debug)]
pub struct PendingPaste {
    pub mode: PasteMode,
    pub dest: PathBuf,
    /// Sources still to process. Drained as we go.
    pub remaining: Vec<PathBuf>,
    /// If set, all remaining conflicts are auto-resolved with this action.
    pub apply_to_all: Option<ConflictAction>,
    /// Original full source list — kept so the clipboard can be re-armed
    /// on Copy mode after the paste completes.
    pub originals: Vec<PathBuf>,
    /// Successfully created targets (for Copy undo). Populated by the
    /// background copy worker on completion.
    pub created: Vec<PathBuf>,
    /// Successful (src, dst) pairs (for Cut undo). Cut runs inline since
    /// fs::rename is atomic; no worker needed.
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// Sources that hit PermissionDenied — sent to the sudo flow at the end.
    pub perm_fails: Vec<PathBuf>,
    /// Resolved (src, target) pairs ready for the copy worker. Only used
    /// in Copy mode — Cut applies inline.
    pub resolved_pairs: Vec<(PathBuf, PathBuf)>,
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
            created: Vec::new(),
            moves: Vec::new(),
            perm_fails: Vec::new(),
            resolved_pairs: Vec::new(),
            reload_tab: None,
        }
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
    pub source: PathBuf,
    pub target: PathBuf,
    pub source_meta: ConflictMeta,
    pub target_meta: ConflictMeta,
    pub apply_to_all: bool,
    pub remaining_count: usize,
    pub mode: PasteMode,
}

#[derive(Clone, Debug, Default)]
pub struct ConflictMeta {
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub is_dir: bool,
}

impl ConflictMeta {
    pub fn read(path: &Path) -> Self {
        let m = std::fs::metadata(path).ok();
        Self {
            size: m.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: m.as_ref().and_then(|m| m.modified().ok()),
            is_dir: m.as_ref().map(|m| m.is_dir()).unwrap_or(false),
        }
    }
}

/// Generate a "Keep Both" target by suffixing the basename: foo.txt → foo (2).txt.
/// Counts up until a free slot is found, capped at 1000 to avoid loops.
pub fn unique_keep_both_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or(Path::new("."));
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = target.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    for n in 2u32..1000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} (copy){ext}"))
}

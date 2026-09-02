//! Async copy worker + progress reporting.
//!
//! Pastes/drops that copy data run on a background thread so the UI stays
//! responsive on multi-GB folder copies. The worker sends `OpProgress`
//! events back over an mpsc channel; the main loop polls those each frame
//! and surfaces them in the status bar.
//!
//! Cancellation: the worker checks `cancel` between files. Mid-file cancel
//! isn't supported (we don't want partially-written destinations sitting
//! around) — the smallest unit of cancellation is one file/folder.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

/// Events the worker sends to the main loop.
#[derive(Debug)]
pub enum OpProgress {
    /// Starting work on a new file/folder.
    StartedItem {
        index: usize,
        total: usize,
        name: String,
    },
    /// All items processed. Carries undo + sudo-fallback data.
    Done {
        created: Vec<PathBuf>,
        perm_fails: Vec<PathBuf>,
        cancelled: bool,
    },
}

/// State the main thread holds while a worker is running.
pub struct OpHandle {
    pub rx: Receiver<OpProgress>,
    pub cancel: Arc<AtomicBool>,
    /// Live snapshot of the latest StartedItem — cheaper than asking the
    /// renderer to drain the channel.
    pub current_name: String,
    pub index: usize,
    pub total: usize,
    /// What kind of operation, for the status bar label.
    pub label: &'static str,
    /// True once Done has been received; the main loop uses this to finalize.
    pub finished: bool,
    /// For Copy ops: the original source list, used to re-arm the clipboard.
    pub originals: Vec<PathBuf>,
    pub dest: PathBuf,
    pub mode: crate::conflict::PasteMode,
    /// Buffered Done payload — captured when the Done event arrives.
    pub done_payload: Option<(Vec<PathBuf>, Vec<PathBuf>, bool)>,
    /// If this op started from a drag-drop onto a non-current tab, reload
    /// that tab too on completion.
    pub reload_tab: Option<usize>,
}

impl OpHandle {
    pub fn poll(&mut self) -> bool {
        let mut dirty = false;
        while let Ok(evt) = self.rx.try_recv() {
            match evt {
                OpProgress::StartedItem { index, total, name } => {
                    self.index = index;
                    self.total = total;
                    self.current_name = name;
                    dirty = true;
                }
                OpProgress::Done {
                    created,
                    perm_fails,
                    cancelled,
                } => {
                    self.done_payload = Some((created, perm_fails, cancelled));
                    self.finished = true;
                    dirty = true;
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

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// Spawn the copy worker. Pairs (src, target) are pre-resolved — the worker
/// only does the file IO. Returns the handle that the main thread polls.
pub fn spawn_copy_worker(
    pairs: Vec<(PathBuf, PathBuf)>,
    originals: Vec<PathBuf>,
    dest: PathBuf,
    label: &'static str,
) -> OpHandle {
    spawn_worker(pairs, originals, dest, label, crate::conflict::PasteMode::Copy)
}

/// Cross-filesystem move (phone → disk, USB stick → home): `rename` fails
/// with EXDEV there, so each pair is copied and the source deleted once the
/// copy verifiably landed. Same progress strip and cancel as a copy.
pub fn spawn_move_worker(pairs: Vec<(PathBuf, PathBuf)>, dest: PathBuf) -> OpHandle {
    spawn_worker(pairs, Vec::new(), dest, "Moving", crate::conflict::PasteMode::Cut)
}

fn spawn_worker(
    pairs: Vec<(PathBuf, PathBuf)>,
    originals: Vec<PathBuf>,
    dest: PathBuf,
    label: &'static str,
    mode: crate::conflict::PasteMode,
) -> OpHandle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let total = pairs.len();
    let delete_sources = matches!(mode, crate::conflict::PasteMode::Cut);

    thread::Builder::new()
        .name("fox-copy".into())
        .spawn(move || {
            run_copy(pairs, tx, cancel_clone, delete_sources);
        })
        .expect("spawn fox-copy thread");

    OpHandle {
        rx,
        cancel,
        current_name: String::new(),
        index: 0,
        total,
        label,
        finished: false,
        originals,
        dest,
        mode,
        done_payload: None,
        reload_tab: None,
    }
}

fn run_copy(
    pairs: Vec<(PathBuf, PathBuf)>,
    tx: Sender<OpProgress>,
    cancel: Arc<AtomicBool>,
    delete_sources: bool,
) {
    let total = pairs.len();
    let mut created = Vec::new();
    let mut perm_fails = Vec::new();
    let mut cancelled = false;

    for (i, (src, target)) in pairs.into_iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| target.display().to_string());
        let _ = tx.send(OpProgress::StartedItem {
            index: i,
            total,
            name,
        });

        let res = if src.is_dir() {
            crate::file_ops::copy_dir_recursive(&src, &target)
        } else {
            std::fs::copy(&src, &target).map(|_| ())
        };
        match res {
            Ok(()) => {
                if delete_sources && copy_landed(&src, &target) {
                    let _ = if src.is_dir() {
                        std::fs::remove_dir_all(&src)
                    } else {
                        std::fs::remove_file(&src)
                    };
                }
                created.push(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                perm_fails.push(src);
            }
            Err(_) => {}
        }
    }
    // Final tick so the bar reaches 100% before the Done event.
    let _ = tx.send(OpProgress::StartedItem {
        index: total,
        total,
        name: String::new(),
    });
    let _ = tx.send(OpProgress::Done {
        created,
        perm_fails,
        cancelled,
    });
}

/// Before a move deletes its source: `fs::copy` reports Ok once the bytes
/// are written, but a FUSE destination only really commits on close, whose
/// error `File`'s Drop swallows. A size match is the cheap sanity check
/// that the data actually arrived. Directories trust the recursive copy's
/// own error propagation.
fn copy_landed(src: &std::path::Path, target: &std::path::Path) -> bool {
    if src.is_dir() {
        return true;
    }
    match (std::fs::metadata(src), std::fs::metadata(target)) {
        (Ok(s), Ok(t)) => s.len() == t.len(),
        _ => false,
    }
}

// The file watcher of the sync engine: inotify on ~/Cloud, reduced to one
// question per event — "does the folder have to be scanned again?".
//
// The answer is no for paths sync ignores anyway. That matters: a download
// writes a temp file inside the watched tree, and if that write counted as a
// change, a failing download would schedule its own retry every two seconds,
// forever.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

use super::ignore::should_ignore;
use super::log_line;

/// Does a change at `path` mean ~/Cloud has to be looked at again? Not when
/// sync ignores the path anyway: a failed download's temp file must not be
/// what schedules the next attempt.
fn path_matters(path: &Path, root: &Path) -> bool {
    match path.strip_prefix(root) {
        // Outside the root, or the root itself: look.
        Err(_) => true,
        Ok(rel) if rel.as_os_str().is_empty() => true,
        // A name that is not UTF-8 is not synced.
        Ok(rel) => rel.to_str().is_some_and(|r| !should_ignore(r)),
    }
}

fn event_matters(ev: &notify::Result<notify::Event>, root: &Path) -> bool {
    use notify::event::{AccessKind, AccessMode, EventKind};
    let Ok(ev) = ev else {
        // Watcher trouble (queue overflow): something may have been missed.
        return true;
    };
    if let EventKind::Access(kind) = ev.kind {
        // Reads change nothing; only "closed after writing" does.
        if kind != AccessKind::Close(AccessMode::Write) {
            return false;
        }
    }
    ev.paths.is_empty() || ev.paths.iter().any(|p| path_matters(p, root))
}

pub(super) struct Watch {
    root: PathBuf,
    watcher: Option<notify::RecommendedWatcher>,
    pub rx: Receiver<notify::Result<notify::Event>>,
    /// Keeps the channel open when there is no watcher, so waiting on `rx`
    /// still works as the loop's sleep.
    _tx: Sender<notify::Result<notify::Event>>,
    /// (device, inode) of the directory being watched.
    attached: Option<(u64, u64)>,
    /// A failed attach is retried before every pass, but reported once.
    attach_failed: bool,
}

impl Watch {
    pub fn new(root: PathBuf) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx_cb = tx.clone();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = tx_cb.send(res);
        })
        .map_err(|e| {
            log_line(&format!("no file watcher ({e}): local changes sync on the next poll"))
        })
        .ok();
        Self {
            root,
            watcher,
            rx,
            _tx: tx,
            attached: None,
            attach_failed: false,
        }
    }

    /// Does this event mean the folder has to be scanned again?
    pub fn matters(&self, ev: &notify::Result<notify::Event>) -> bool {
        event_matters(ev, &self.root)
    }

    /// Watch the directory that is at the root NOW. ~/Cloud can be moved
    /// away and a new one put in its place; the old watch would keep
    /// following the old directory. Call before a scan: a (re)attached watch
    /// has seen nothing of what changed before it.
    pub fn ensure(&mut self) {
        use notify::Watcher;
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let now = std::fs::metadata(&self.root)
            .ok()
            .filter(|m| m.is_dir())
            .map(|m| (m.dev(), m.ino()));
        if now == self.attached {
            return;
        }
        if self.attached.take().is_some() {
            let _ = watcher.unwatch(&self.root);
        }
        if now.is_none() {
            return;
        }
        match watcher.watch(&self.root, notify::RecursiveMode::Recursive) {
            Ok(()) => {
                self.attached = now;
                self.attach_failed = false;
            }
            Err(e) => {
                if !self.attach_failed {
                    log_line(&format!(
                        "watch {} failed ({e}): local changes sync on the next poll",
                        self.root.display()
                    ));
                }
                self.attach_failed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_paths_do_not_wake_the_engine() {
        let root = Path::new("/home/u/Cloud");
        assert!(path_matters(Path::new("/home/u/Cloud/notes/a.txt"), root));
        assert!(path_matters(Path::new("/home/u/Cloud"), root));
        // The engine's own download temp, an editor backup, git internals.
        assert!(!path_matters(Path::new("/home/u/Cloud/notes/a.txt.fox-tmp"), root));
        assert!(!path_matters(Path::new("/home/u/Cloud/doc.txt~"), root));
        assert!(!path_matters(Path::new("/home/u/Cloud/proj/.git/index"), root));
    }

    #[test]
    fn only_events_that_can_change_content_count() {
        use notify::event::{AccessKind, AccessMode, CreateKind, EventKind};
        let root = Path::new("/home/u/Cloud");
        let ev = |kind, path: &str| -> notify::Result<notify::Event> {
            Ok(notify::Event::new(kind).add_path(PathBuf::from(path)))
        };

        let create = EventKind::Create(CreateKind::File);
        assert!(event_matters(&ev(create, "/home/u/Cloud/a.txt"), root));
        assert!(!event_matters(&ev(create, "/home/u/Cloud/a.txt.fox-tmp"), root));

        let closed = EventKind::Access(AccessKind::Close(AccessMode::Write));
        assert!(event_matters(&ev(closed, "/home/u/Cloud/a.txt"), root));
        let opened = EventKind::Access(AccessKind::Open(AccessMode::Any));
        assert!(!event_matters(&ev(opened, "/home/u/Cloud/a.txt"), root));

        // An event that names no path, or a watcher error: look.
        assert!(event_matters(&Ok(notify::Event::new(create)), root));
        assert!(event_matters(&Err(notify::Error::generic("overflow")), root));
    }
}

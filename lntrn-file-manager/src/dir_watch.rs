//! Instant directory refresh: an inotify watch on the directory being
//! browsed, surfaced to the main loop as a pollable eventfd.
//!
//! The notify callback runs on its own thread. It filters to events that
//! change the listing (create/remove/modify — NOT access, which our own
//! directory reads and thumbnail decodes would trigger in a feedback
//! loop), sets a dirty flag, and pokes an eventfd so the main loop's
//! poll() wakes immediately instead of waiting out its idle timeout.
//! Reloads are debounced so an unzip burst or a chunked download
//! coalesces into a few listing rebuilds instead of hundreds. Filesystems
//! that don't deliver inotify (sshfs, some FUSE mounts) still refresh via
//! the 3s mtime poll in wayland_loop.rs.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Coalesce window: the first event of a burst schedules one reload this
/// long after it. Short enough to feel instant, long enough to batch.
const DEBOUNCE: Duration = Duration::from_millis(120);

pub struct DirWatcher {
    watcher: Option<RecommendedWatcher>,
    watched: Option<PathBuf>,
    dirty: Arc<AtomicBool>,
    eventfd: RawFd,
    pending_since: Option<Instant>,
}

impl DirWatcher {
    pub fn new() -> Self {
        let dirty = Arc::new(AtomicBool::new(false));
        let eventfd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let flag = Arc::clone(&dirty);
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_)
            ) {
                flag.store(true, Ordering::Release);
                let one: u64 = 1;
                unsafe {
                    libc::write(eventfd, &one as *const u64 as *const libc::c_void, 8);
                }
            }
        })
        .ok();
        Self { watcher, watched: None, dirty, eventfd, pending_since: None }
    }

    /// Move the (non-recursive) watch to `dir`. No-op when already watching
    /// it, so it's safe to call every frame.
    pub fn watch(&mut self, dir: &Path) {
        if self.watched.as_deref() == Some(dir) {
            return;
        }
        if let Some(w) = &mut self.watcher {
            if let Some(old) = self.watched.take() {
                let _ = w.unwatch(&old);
            }
            // Failure (permissions, vanished dir, FUSE quirks) is fine —
            // the mtime poll fallback still refreshes eventually.
            if w.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                self.watched = Some(dir.to_path_buf());
            }
        }
        // Navigation reloads anyway — drop stale events from the old dir.
        self.dirty.store(false, Ordering::Release);
        self.pending_since = None;
    }

    /// Fd for the main loop's poll() set — readable when fs events arrived.
    pub fn fd(&self) -> RawFd {
        self.eventfd
    }

    /// Clear the eventfd after poll() reported it readable.
    pub fn drain_fd(&self) {
        let mut buf = 0u64;
        unsafe {
            libc::read(self.eventfd, &mut buf as *mut u64 as *mut libc::c_void, 8);
        }
    }

    /// True exactly once per burst, `DEBOUNCE` after its first event — the
    /// caller reloads the listing then.
    pub fn take_due_reload(&mut self) -> bool {
        if self.dirty.swap(false, Ordering::Acquire) && self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
        match self.pending_since {
            Some(t) if t.elapsed() >= DEBOUNCE => {
                self.pending_since = None;
                true
            }
            _ => false,
        }
    }

    /// A reload is scheduled or events are unprocessed — the loop should
    /// poll fast instead of idling.
    pub fn reload_pending(&self) -> bool {
        self.pending_since.is_some() || self.dirty.load(Ordering::Acquire)
    }
}

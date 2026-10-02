//! Instant directory refresh: an inotify watch on the directory being
//! browsed, surfaced to the main loop as a pollable eventfd.
//!
//! The notify callback runs on its own thread. It filters to events that
//! change the listing (create/remove/modify — NOT access, which our own
//! directory reads and thumbnail decodes would trigger in a feedback
//! loop), sets a dirty flag, and pokes an eventfd so the main loop's
//! poll() wakes immediately instead of waiting out its idle timeout.
//! Reloads are debounced and rate-limited (`ReloadGate`) so an unzip burst
//! is one listing rebuild and a download that writes for minutes is one a
//! second, not eight. Filesystems that don't deliver inotify still refresh
//! via the 3s mtime poll in housekeeping.rs.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Coalesce window: the first event of a burst schedules one reload this
/// long after it. Short enough to feel instant, long enough to batch.
const DEBOUNCE: Duration = Duration::from_millis(120);

/// Least time between two watcher-driven reloads. A file that is being
/// written (a download, a copy from another program) sends events for as
/// long as it grows; each reload re-lists the folder on the UI thread and
/// starts a git scan.
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// When a stream of change events turns into reloads: the first one
/// `DEBOUNCE` after the first event, then at most one per `MIN_INTERVAL`,
/// and always one after the last event, so the listing ends up current.
#[derive(Default)]
pub(crate) struct ReloadGate {
    /// First event not yet covered by a reload.
    pending_since: Option<Instant>,
    last_reload: Option<Instant>,
}

impl ReloadGate {
    pub(crate) fn note_event(&mut self, now: Instant) {
        self.pending_since.get_or_insert(now);
    }

    pub(crate) fn pending(&self) -> bool {
        self.pending_since.is_some()
    }

    /// When the pending reload is due, if one is pending.
    pub(crate) fn due_at(&self) -> Option<Instant> {
        let debounced = self.pending_since? + DEBOUNCE;
        Some(match self.last_reload {
            Some(last) => debounced.max(last + MIN_INTERVAL),
            None => debounced,
        })
    }

    /// True exactly once per due reload; the caller reloads then.
    pub(crate) fn take_due(&mut self, now: Instant) -> bool {
        match self.due_at() {
            Some(due) if now >= due => {
                self.pending_since = None;
                self.last_reload = Some(now);
                true
            }
            _ => false,
        }
    }

    /// Start over (the watch moved: navigation reloads anyway, and the
    /// interval is about the folder that was being written to).
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

pub struct DirWatcher {
    watcher: Option<RecommendedWatcher>,
    watched: Option<PathBuf>,
    dirty: Arc<AtomicBool>,
    eventfd: RawFd,
    gate: ReloadGate,
    /// Last directory a watch could not be placed on, and when. `watch` is
    /// called every frame; a folder that can't be watched is retried once a
    /// second, not sixty times.
    failed: Option<(PathBuf, Instant)>,
}

const RETRY_FAILED_WATCH: Duration = Duration::from_secs(1);

impl DirWatcher {
    pub fn new() -> Self {
        let dirty = Arc::new(AtomicBool::new(false));
        let eventfd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let flag = Arc::clone(&dirty);
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            // need_rescan: the kernel queue overflowed and events were lost.
            if event.need_rescan()
                || matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_)
                )
            {
                // Wake the loop for the first event only: until it has
                // looked, more of them change nothing, and a file being
                // written sends thousands a second.
                if !flag.swap(true, Ordering::AcqRel) {
                    let one: u64 = 1;
                    unsafe {
                        libc::write(eventfd, &one as *const u64 as *const libc::c_void, 8);
                    }
                }
            }
        })
        .ok();
        Self {
            watcher,
            watched: None,
            dirty,
            eventfd,
            gate: ReloadGate::default(),
            failed: None,
        }
    }

    /// Move the (non-recursive) watch to `dir`. No-op when already watching
    /// it, so it's safe to call every frame.
    pub fn watch(&mut self, dir: &Path) {
        if self.watched.as_deref() == Some(dir) {
            return;
        }
        if self
            .failed
            .as_ref()
            .is_some_and(|(p, t)| p == dir && t.elapsed() < RETRY_FAILED_WATCH)
        {
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
                self.failed = None;
            } else {
                self.failed = Some((dir.to_path_buf(), Instant::now()));
            }
        }
        // Navigation reloads anyway — drop stale events from the old dir.
        self.dirty.store(false, Ordering::Release);
        self.gate.clear();
    }

    /// Stop watching. Used on slow mounts, where inotify never fires anyway
    /// and a stale watch on the previous folder would trigger reloads of
    /// the wrong directory.
    pub fn unwatch(&mut self) {
        if let (Some(w), Some(old)) = (&mut self.watcher, self.watched.take()) {
            let _ = w.unwatch(&old);
        }
        self.dirty.store(false, Ordering::Release);
        self.gate.clear();
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

    /// True when the listing should be read again now (see `ReloadGate`).
    pub fn take_due_reload(&mut self, now: Instant) -> bool {
        // While a reload is already scheduled the flag stays up: the
        // watcher thread then has no reason to wake the loop again.
        if !self.gate.pending() && self.dirty.swap(false, Ordering::AcqRel) {
            self.gate.note_event(now);
        }
        if !self.gate.take_due(now) {
            return false;
        }
        // Everything up to here is covered by the listing about to be read.
        self.dirty.store(false, Ordering::Release);
        // The watch sits on an inode. If the folder was deleted and
        // recreated (a build's output dir, a re-extract), the kernel
        // dropped it; placing it again is a no-op when it is still
        // there and re-arms it when it is not.
        if let (Some(w), Some(dir)) = (&mut self.watcher, self.watched.clone()) {
            // Unwatch first: after a rename of the folder, the old
            // watch still sits on the moved inode and would keep
            // reporting changes from wherever it went.
            let _ = w.unwatch(&dir);
            if w.watch(&dir, RecursiveMode::NonRecursive).is_err() {
                self.watched = None;
                self.failed = Some((dir, Instant::now()));
            }
        }
        true
    }

    /// A reload is scheduled or events are unprocessed.
    pub fn reload_pending(&self) -> bool {
        self.gate.due_at().is_some() || self.dirty.load(Ordering::Acquire)
    }

    /// When the loop has to look again for the scheduled reload to happen
    /// on time. (Fresh events wake it through the eventfd.)
    pub fn wake_at(&self) -> Option<Instant> {
        self.gate.due_at()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_single_event_reloads_once_after_the_debounce() {
        let t0 = Instant::now();
        let mut gate = ReloadGate::default();
        assert_eq!(gate.due_at(), None);
        gate.note_event(t0);
        assert!(!gate.take_due(t0 + ms(50)));
        assert!(gate.take_due(t0 + DEBOUNCE));
        // Nothing more until something happens again.
        assert_eq!(gate.due_at(), None);
        assert!(!gate.take_due(t0 + ms(5_000)));
    }

    #[test]
    fn a_burst_is_one_reload() {
        let t0 = Instant::now();
        let mut gate = ReloadGate::default();
        for i in 0..100 {
            gate.note_event(t0 + ms(i));
        }
        assert!(gate.take_due(t0 + DEBOUNCE));
        assert_eq!(gate.due_at(), None);
    }

    #[test]
    fn a_sustained_stream_reloads_once_per_interval_and_once_at_the_end() {
        let t0 = Instant::now();
        let mut gate = ReloadGate::default();
        let mut reloads = Vec::new();
        // An event every 20 ms for five seconds, the loop looking every 10.
        for tick in 0..=700u64 {
            let now = t0 + ms(tick * 10);
            if tick % 2 == 0 && tick <= 500 {
                gate.note_event(now);
            }
            if gate.take_due(now) {
                reloads.push(tick * 10);
            }
        }
        assert_eq!(reloads.first(), Some(&120), "the first is prompt");
        assert!(
            reloads.windows(2).all(|w| w[1] - w[0] >= 1_000),
            "never two within the interval: {reloads:?}"
        );
        // Five seconds of events: the prompt one, one per second, and the
        // closing one that covers the events after the last reload.
        assert_eq!(reloads.len(), 6, "{reloads:?}");
        assert!(
            *reloads.last().unwrap() >= 5_000,
            "the stream's end is covered"
        );
        assert_eq!(gate.due_at(), None);
    }

    #[test]
    fn events_after_a_reload_wait_out_the_interval() {
        let t0 = Instant::now();
        let mut gate = ReloadGate::default();
        gate.note_event(t0);
        assert!(gate.take_due(t0 + DEBOUNCE));
        gate.note_event(t0 + ms(300));
        assert_eq!(gate.due_at(), Some(t0 + DEBOUNCE + MIN_INTERVAL));
        assert!(!gate.take_due(t0 + ms(600)));
        assert!(gate.take_due(t0 + DEBOUNCE + MIN_INTERVAL));
        // Long after the last reload only the debounce applies again.
        gate.note_event(t0 + ms(10_000));
        assert_eq!(gate.due_at(), Some(t0 + ms(10_000) + DEBOUNCE));
    }

    #[test]
    fn moving_the_watch_drops_what_was_pending() {
        let t0 = Instant::now();
        let mut gate = ReloadGate::default();
        gate.note_event(t0);
        gate.clear();
        assert!(!gate.take_due(t0 + ms(1_000)));
    }
}

//! What the main loop looks after on every wake-up, whether or not a frame
//! is drawn: the folder watchers and their reloads, the mtime fallback, the
//! git badges, and the results of work running on other threads.
//!
//! All of this used to sit behind the "is a frame due?" gate. An untouched
//! window draws no frames, so a file that landed in the folder, a finished
//! git scan or a finished copy only showed once the mouse moved. `tick`
//! runs ahead of that gate and reports whether anything visible changed;
//! only then is a frame drawn. `wake_at` tells the loop when it has to look
//! again on its own (a reload that is waiting out its debounce), so idle
//! stays idle: no timer, no frames, while nothing is pending.

use std::os::fd::RawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::app::App;
use crate::dir_watch::DirWatcher;
use crate::file_info::FileInfoCache;
use crate::git_status::GitStatus;

/// Commits and stages made in a terminal change git state without any
/// event in the folder shown, so a repository is looked at again on a
/// timer: every few seconds while the window is in use...
const GIT_RESCAN_ACTIVE: Duration = Duration::from_secs(5);
/// ...and rarely once it has not been drawn for a while. (A scan that finds
/// nothing new draws nothing, so an untouched window settles here.)
const GIT_RESCAN_IDLE: Duration = Duration::from_secs(30);
const IN_USE_WINDOW: Duration = Duration::from_secs(10);
/// Fallback for filesystems that do not deliver inotify events.
const MTIME_POLL: Duration = Duration::from_secs(3);
/// While frames are drawn the device list is refreshed this often, which
/// keeps the free-space bars current. (Hot-plugs are noticed by the device
/// watcher's own thread.)
const DEVICE_REFRESH: Duration = Duration::from_secs(2);

pub(crate) struct Housekeeping {
    dir_watcher: DirWatcher,
    /// The unfocused split pane's folder is on screen as well; without a
    /// watch of its own it only changed when the focused pane reloaded.
    inactive_watcher: DirWatcher,
    pub(crate) git: GitStatus,
    /// The folder git was last pointed at.
    git_dir: PathBuf,
    last_git_scan: Instant,
    last_frame: Instant,
    /// The folder the mtime tracker is for, and what it last saw.
    mtime_dir: PathBuf,
    last_dir_mtime: Option<SystemTime>,
    last_dir_check: Instant,
    last_devices_check: Instant,
}

impl Housekeeping {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            dir_watcher: DirWatcher::new(),
            inactive_watcher: DirWatcher::new(),
            git: GitStatus::new(),
            git_dir: PathBuf::new(),
            last_git_scan: now,
            last_frame: now,
            mtime_dir: PathBuf::new(),
            last_dir_mtime: None,
            last_dir_check: now,
            last_devices_check: now,
        }
    }

    /// The watchers' eventfds, for the loop's poll set: a change in a shown
    /// folder wakes it at once.
    pub(crate) fn fds(&self) -> [RawFd; 2] {
        [self.dir_watcher.fd(), self.inactive_watcher.fd()]
    }

    /// Clear the eventfds after poll() reported one readable.
    pub(crate) fn drain_fds(&self) {
        self.dir_watcher.drain_fd();
        self.inactive_watcher.drain_fd();
    }

    /// Run on every wake-up of the loop and again right after a frame (the
    /// frame's input may have navigated, swapped panes or started work).
    /// True when the window has to be drawn again.
    pub(crate) fn tick(&mut self, app: &mut App, file_info: &mut FileInfoCache) -> bool {
        let now = Instant::now();
        let mut redraw = false;

        // ── Results of work on other threads ────────────────────────────
        redraw |= app.poll_search();
        // An operation ended (or its dialog arrived).
        redraw |= app.poll_op_progress();
        // Same for a privileged command on its sudo thread.
        redraw |= app.poll_priv();
        // Media probes; a result needs a frame to show.
        redraw |= file_info.poll();
        // Root mode never outlives the folder, tab or pane it was for.
        app.expire_root_mode();
        // A link into a phone or a share was shown as a file until its
        // target had been asked about, off this thread (links.rs).
        if crate::links::take_changed() {
            app.reload();
            redraw = true;
        }

        // A click moved the focus to the other pane. Both listings are read
        // again now that the click has been handled: the watchers below are
        // about to be re-pointed and drop what they had pending.
        if app.take_focus_refresh() {
            app.reload();
            redraw = true;
        }

        // ── Watchers ────────────────────────────────────────────────────
        // On a slow mount (MTP phone, sshfs) every stat is a device round
        // trip that can block for minutes behind a jmtpfs download, so the
        // watcher, the mtime poll and git all stand down there.
        let slow_dir = crate::fs::is_slow_path(&app.current_dir);
        if slow_dir {
            self.dir_watcher.unwatch();
        } else {
            self.dir_watcher.watch(&app.current_dir);
        }
        match app
            .inactive_dir()
            .filter(|dir| !crate::fs::is_slow_path(dir))
        {
            // One watch is enough when both panes show the same folder:
            // `reload` refreshes the unfocused pane too.
            Some(dir) if dir != app.current_dir => self.inactive_watcher.watch(dir),
            _ => self.inactive_watcher.unwatch(),
        }

        // Files came or went in a shown folder (debounced and rate-limited,
        // see dir_watch.rs).
        if self.dir_watcher.take_due_reload(now) {
            app.reload();
            if !slow_dir {
                // Keep the mtime tracker in step so the fallback poll below
                // does not schedule a second reload for the same change.
                self.last_dir_mtime = dir_mtime(&app.current_dir);
                self.last_git_scan = now;
                self.git.refresh();
            }
            redraw = true;
        }
        if self.inactive_watcher.take_due_reload(now) {
            app.reload_inactive_pane();
            redraw = true;
        }

        // ── Git badges / branch ─────────────────────────────────────────
        redraw |= self.git.poll(now);
        if app.current_dir != self.git_dir {
            redraw |= self.git_follow(app);
        } else if self.git.in_repo()
            && now.saturating_duration_since(self.last_git_scan) >= self.git_rescan_interval(now)
        {
            self.last_git_scan = now;
            self.git.refresh();
        }

        // ── Mtime fallback ──────────────────────────────────────────────
        if app.current_dir != self.mtime_dir {
            // Navigation: reset the tracker, the listing is fresh.
            self.mtime_dir = app.current_dir.clone();
            self.last_dir_mtime = if slow_dir {
                None
            } else {
                dir_mtime(&app.current_dir)
            };
            self.last_dir_check = now;
        } else if !slow_dir && now.saturating_duration_since(self.last_dir_check) >= MTIME_POLL {
            self.last_dir_check = now;
            let current = dir_mtime(&app.current_dir);
            // With a watcher reload already on its way, that one covers the
            // change (and keeps to the reload rate limit).
            if current != self.last_dir_mtime && !self.dir_watcher.reload_pending() {
                self.last_dir_mtime = current;
                app.reload();
                self.last_git_scan = now;
                self.git.refresh();
                redraw = true;
            }
        }

        redraw
    }

    /// Point git at the folder shown if that changed: the previous
    /// folder's branch and badges go, a scan of the new one starts. Also
    /// called just before a frame is drawn, so the frame that shows a new
    /// folder never carries the old folder's branch chip. True when what
    /// is shown changed.
    pub(crate) fn git_follow(&mut self, app: &App) -> bool {
        if app.current_dir == self.git_dir {
            return false;
        }
        self.git_dir = app.current_dir.clone();
        self.last_git_scan = Instant::now();
        if crate::fs::is_slow_path(&app.current_dir) {
            self.git.clear()
        } else {
            self.git.enter(&app.current_dir)
        }
    }

    /// A frame was drawn: the window is in use.
    pub(crate) fn frame_drawn(&mut self, app: &App) {
        let now = Instant::now();
        self.last_frame = now;
        if now.saturating_duration_since(self.last_devices_check) >= DEVICE_REFRESH {
            self.last_devices_check = now;
            app.refresh_devices();
        }
    }

    fn git_rescan_interval(&self, now: Instant) -> Duration {
        if now.saturating_duration_since(self.last_frame) < IN_USE_WINDOW {
            GIT_RESCAN_ACTIVE
        } else {
            GIT_RESCAN_IDLE
        }
    }

    /// When the loop has to wake on its own for something scheduled here:
    /// a reload waiting out its debounce or interval, a git scan waiting
    /// out its cooldown. `None`: nothing is pending.
    pub(crate) fn wake_at(&self) -> Option<Instant> {
        [
            self.dir_watcher.wake_at(),
            self.inactive_watcher.wake_at(),
            self.git.wake_at(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

fn dir_mtime(dir: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

/// The earlier of two optional deadlines.
pub(crate) fn sooner(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// How long the loop's poll() may sleep: until `wake_at`, and never longer
/// than `idle_ms` (the cadence of its own periodic checks).
pub(crate) fn poll_timeout_ms(now: Instant, wake_at: Option<Instant>, idle_ms: i32) -> i32 {
    match wake_at {
        None => idle_ms,
        Some(at) => {
            let wait = at.saturating_duration_since(now);
            // Rounded up: waking a fraction of a millisecond early would
            // find nothing due and spin once more.
            let ms = wait.as_micros().div_ceil(1000);
            ms.min(idle_ms as u128) as i32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_sleeps_until_the_next_deadline_and_no_longer_than_idle() {
        let now = Instant::now();
        assert_eq!(poll_timeout_ms(now, None, 500), 500);
        // Already due: do not sleep.
        assert_eq!(
            poll_timeout_ms(now + Duration::from_secs(1), Some(now), 500),
            0
        );
        assert_eq!(poll_timeout_ms(now, Some(now), 500), 0);
        assert_eq!(
            poll_timeout_ms(now, Some(now + Duration::from_millis(120)), 500),
            120
        );
        // Rounded up, so the wake-up is not a hair too early.
        assert_eq!(
            poll_timeout_ms(now, Some(now + Duration::from_micros(40_200)), 500),
            41
        );
        assert_eq!(
            poll_timeout_ms(now, Some(now + Duration::from_secs(30)), 500),
            500
        );
    }

    #[test]
    fn the_sooner_of_two_deadlines() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        assert_eq!(sooner(None, None), None);
        assert_eq!(sooner(Some(now), None), Some(now));
        assert_eq!(sooner(None, Some(later)), Some(later));
        assert_eq!(sooner(Some(later), Some(now)), Some(now));
    }

    #[test]
    fn a_repository_is_rescanned_often_in_use_and_rarely_when_idle() {
        let mut hk = Housekeeping::new();
        let now = Instant::now();
        hk.last_frame = now;
        assert_eq!(hk.git_rescan_interval(now), GIT_RESCAN_ACTIVE);
        assert_eq!(hk.git_rescan_interval(now + IN_USE_WINDOW), GIT_RESCAN_IDLE);
        // Nothing scheduled in a fresh loop: it may sleep its idle timeout.
        assert_eq!(hk.wake_at(), None);
    }
}

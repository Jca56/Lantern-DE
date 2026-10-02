//! The cache of file facts for the status bar, the preview pane and the
//! Properties dialog.
//!
//! Probing runs on one background worker. `get` returns at once: what is
//! known, or an extension-only placeholder with a probe queued behind it;
//! the full record lands via `poll` a moment later. On slow mounts (MTP,
//! sshfs…) nothing is probed at all: jmtpfs downloads the whole file on
//! the first read, so even "read 32 header bytes" means pulling a 3 GB
//! video across USB. That was the Fox-freezes-on-phone bug.
//!
//! A record belongs to one state of the file: the size and date its
//! listing showed when it was asked for. When those change (a download
//! finished, an image was edited) the file is probed again. Not on every
//! change, though: a file that is still growing changes many times a
//! second, and each probe of a video is an ffprobe process. A changed
//! file waits out `REPROBE_AFTER` since its last probe and shows what was
//! known until then.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::{build_info, placeholder, FileInfo};

/// What a listing knows of a file's state: its size and modification time.
pub type Stamp = (u64, Option<SystemTime>);

/// The least time between two probes of one file.
const REPROBE_AFTER: Duration = Duration::from_secs(2);
/// One small record per file ever selected or previewed; past this many
/// the cache starts over.
const MAX_RECORDS: usize = 2000;

type Prober = Arc<dyn Fn(&Path) -> FileInfo + Send + Sync>;
type Jobs = Arc<(Mutex<Vec<PathBuf>>, Condvar)>;

struct Record {
    info: FileInfo,
    /// The state of the file `info` was (or is being) probed for.
    stamp: Stamp,
    /// A probe is queued or running.
    pending: bool,
    asked_at: Instant,
    /// The file has changed since: probe it again when allowed.
    changed_to: Option<Stamp>,
}

pub struct FileInfoCache {
    records: HashMap<PathBuf, Record>,
    /// Paths waiting for the worker, newest last — the worker pops from the
    /// back so the file the user just selected beats a backlog left behind
    /// by a fast keyboard scroll.
    jobs: Jobs,
    rx: Receiver<(PathBuf, FileInfo)>,
    in_flight: usize,
    /// Files that changed and are waiting out `REPROBE_AFTER`.
    waiting: Vec<PathBuf>,
}

impl FileInfoCache {
    pub fn new() -> Self {
        Self::with_prober(Arc::new(build_info))
    }

    fn with_prober(prober: Prober) -> Self {
        let jobs: Jobs = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let worker_jobs = Arc::clone(&jobs);
        let _ = std::thread::Builder::new()
            .name("fox-probe".into())
            .spawn(move || probe_loop(worker_jobs, tx, prober));
        Self {
            records: HashMap::new(),
            jobs,
            rx,
            in_flight: 0,
            waiting: Vec::new(),
        }
    }

    /// The record for `path` as its listing shows it (`stamp`), or an
    /// instant placeholder with a probe queued behind it. Never touches
    /// the file on the calling thread.
    pub fn get(&mut self, path: &Path, stamp: Stamp) -> &FileInfo {
        self.get_at(path, stamp, Instant::now())
    }

    fn get_at(&mut self, path: &Path, stamp: Stamp, now: Instant) -> &FileInfo {
        if !self.records.contains_key(path) {
            if self.records.len() >= MAX_RECORDS {
                // What is being probed stays, so that its answer finds a
                // record to land in.
                self.records.retain(|_, r| r.pending);
                self.waiting.clear();
            }
            let slow = crate::fs::is_slow_path(path);
            let mut info = placeholder(path);
            info.probing = !slow;
            self.records.insert(
                path.to_path_buf(),
                Record {
                    info,
                    stamp,
                    pending: !slow,
                    asked_at: now,
                    changed_to: None,
                },
            );
            if !slow {
                self.queue(path);
            }
        } else if let Some(record) = self.records.get_mut(path) {
            let known = record.changed_to.unwrap_or(record.stamp);
            if known != stamp {
                if crate::fs::is_slow_path(path) {
                    // Nothing was probed and nothing will be.
                    record.stamp = stamp;
                } else {
                    record.changed_to = Some(stamp);
                    if !self.waiting.iter().any(|p| p == path) {
                        self.waiting.push(path.to_path_buf());
                    }
                    self.start_due(now);
                }
            }
        }
        &self.records[path].info
    }

    fn queue(&mut self, path: &Path) {
        self.in_flight += 1;
        let (lock, cv) = &*self.jobs;
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(path.to_path_buf());
        cv.notify_one();
    }

    /// Probe again the changed files whose wait is over.
    fn start_due(&mut self, now: Instant) {
        let mut start = Vec::new();
        let records = &mut self.records;
        self.waiting.retain(|path| {
            let Some(record) = records.get_mut(path) else {
                return false;
            };
            let Some(stamp) = record.changed_to else {
                return false;
            };
            if record.pending || now < record.asked_at + REPROBE_AFTER {
                return true;
            }
            record.stamp = stamp;
            record.changed_to = None;
            record.pending = true;
            record.asked_at = now;
            // What is shown stays until the new answer is in.
            record.info.probing = true;
            start.push(path.clone());
            false
        });
        for path in start {
            self.queue(&path);
        }
    }

    /// Drain finished probes and start the ones that were waiting. Returns
    /// true when something landed and the status bar / preview should
    /// redraw.
    pub fn poll(&mut self) -> bool {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> bool {
        let mut landed = false;
        while let Ok((path, info)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if let Some(record) = self.records.get_mut(&path) {
                record.info = info;
                record.pending = false;
            }
            landed = true;
        }
        self.start_due(now);
        landed
    }

    /// Probes still running — the event loop polls instead of idling.
    pub fn probing(&self) -> bool {
        self.in_flight > 0
    }

    /// When `poll` has to be called again for a changed file to be probed.
    /// `None`: nothing is waiting on the clock.
    pub fn wake_at(&self) -> Option<Instant> {
        self.waiting
            .iter()
            .filter_map(|path| self.records.get(path))
            // One that is being probed is picked up when that lands.
            .filter(|record| !record.pending)
            .map(|record| record.asked_at + REPROBE_AFTER)
            .min()
    }
}

fn probe_loop(jobs: Jobs, tx: Sender<(PathBuf, FileInfo)>, prober: Prober) {
    loop {
        let path = {
            let (lock, cv) = &*jobs;
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(p) = q.pop() {
                    break p;
                }
                q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let info = prober(&path);
        if tx.send((path, info)).is_err() {
            return; // cache dropped — shutting down
        }
        // An answer is something to draw; a loop that sleeps is woken for
        // it (one probed on the clock has no frame waiting on it).
        crate::bg::wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A cache whose "probe" reports how many times it ran, as the
    /// duration.
    fn rig() -> (FileInfoCache, Arc<AtomicUsize>) {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = runs.clone();
        let cache = FileInfoCache::with_prober(Arc::new(move |path: &Path| {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            FileInfo {
                duration: Some(format!("probe {n}")),
                ..placeholder(path)
            }
        }));
        (cache, runs)
    }

    fn land(cache: &mut FileInfoCache, now: Instant) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while cache.probing() {
            cache.poll_at(now);
            assert!(Instant::now() < deadline, "the probe never landed");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn stamp(size: u64) -> Stamp {
        (
            size,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(size)),
        )
    }

    #[test]
    fn a_file_is_probed_once_for_as_long_as_it_does_not_change() {
        let (mut cache, runs) = rig();
        let path = Path::new("/nonexistent-fox-test/clip.mp4");
        let t0 = Instant::now();
        let first = cache.get_at(path, stamp(10), t0).clone();
        assert!(first.probing && first.duration.is_none());
        assert_eq!(first.type_name, "MP4 Video");
        land(&mut cache, t0);
        for ms in [1, 500, 60_000] {
            let info = cache.get_at(path, stamp(10), t0 + Duration::from_millis(ms));
            assert_eq!(info.duration.as_deref(), Some("probe 1"));
            assert!(!info.probing);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(cache.wake_at(), None);
    }

    #[test]
    fn a_changed_file_is_probed_again() {
        let (mut cache, runs) = rig();
        let path = Path::new("/nonexistent-fox-test/photo.png");
        let t0 = Instant::now();
        cache.get_at(path, stamp(10), t0);
        land(&mut cache, t0);

        // Edited an hour later: the old facts show while the new are read.
        let later = t0 + Duration::from_secs(3600);
        let info = cache.get_at(path, stamp(20), later).clone();
        assert_eq!(info.duration.as_deref(), Some("probe 1"));
        assert!(info.probing);
        land(&mut cache, later);
        let info = cache.get_at(path, stamp(20), later);
        assert_eq!(info.duration.as_deref(), Some("probe 2"));
        assert!(!info.probing);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_growing_file_is_not_probed_on_every_change() {
        let (mut cache, runs) = rig();
        let path = Path::new("/nonexistent-fox-test/download.mkv");
        let t0 = Instant::now();
        cache.get_at(path, stamp(1), t0);
        land(&mut cache, t0);

        // It grows ten times within the first second.
        for i in 0..10u64 {
            let now = t0 + Duration::from_millis(100 * (i + 1));
            cache.get_at(path, stamp(2 + i), now);
            cache.poll_at(now);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1, "no probe per change");
        assert!(!cache.probing());
        // The loop is told when to look again, and the last state is what
        // gets probed then, with nobody having to ask for the file again.
        let wake = cache.wake_at().expect("a probe is waiting on the clock");
        assert_eq!(wake, t0 + REPROBE_AFTER);
        cache.poll_at(wake);
        assert!(cache.probing());
        land(&mut cache, wake);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(cache.wake_at(), None);
        let info = cache.get_at(path, stamp(11), wake);
        assert_eq!(info.duration.as_deref(), Some("probe 2"));
        assert!(!info.probing);
    }

    #[test]
    fn a_change_while_a_probe_runs_is_picked_up_afterwards() {
        let (mut cache, runs) = rig();
        let path = Path::new("/nonexistent-fox-test/a.mp3");
        let t0 = Instant::now();
        cache.get_at(path, stamp(1), t0);
        // Changed before the first probe even landed.
        cache.get_at(path, stamp(2), t0);
        land(&mut cache, t0);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let later = t0 + REPROBE_AFTER;
        cache.poll_at(later);
        land(&mut cache, later);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_cache_does_not_grow_without_bound() {
        let (mut cache, _) = rig();
        let t0 = Instant::now();
        for i in 0..(MAX_RECORDS + 50) {
            let path = PathBuf::from(format!("/nonexistent-fox-test/{i}.txt"));
            cache.get_at(&path, stamp(1), t0);
            // As the loop does between frames.
            if i % 100 == 99 {
                land(&mut cache, t0);
            }
        }
        land(&mut cache, t0);
        assert!(cache.records.len() <= MAX_RECORDS);
        assert!(cache.records.len() >= 50, "the newest are still there");
    }
}

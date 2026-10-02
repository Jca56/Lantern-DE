//! The thumbnail workers and their queue.
//!
//! The queue follows the screen. Every frame the views ask again for the
//! thumbnails of the rows they show (`want`), and `end_frame` then reduces
//! the queue to exactly those, in the order they were asked for. A job for
//! a row that scrolled away before a worker got to it is dropped, and its
//! key leaves `pending` so the row is asked for afresh when it comes back.
//! Before this the queue was first-in first-out and never shrank: dragging
//! the scrollbar through a few thousand photos made the workers decode all
//! of them, oldest first, while the rows on screen waited their turn.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

use super::{generate, ThumbKind};

/// Finished job delivered back to the render thread.
pub struct ThumbResult {
    pub key: String,
    /// `None` means generation failed (corrupt, oversized, unsupported) —
    /// the caller records the key so the file isn't retried every frame.
    pub rgba: Option<(Vec<u8>, u32, u32)>,
}

struct ThumbJob {
    key: String,
    path: PathBuf,
    kind: ThumbKind,
}

type Lane = (Mutex<VecDeque<ThumbJob>>, Condvar);

/// Fixed-size worker pool. Workers block on a condvar when idle and live for
/// the process lifetime.
pub struct ThumbPool {
    queue: Arc<Lane>,
    /// Single-worker lane for slow mounts (MTP phones, sshfs). jmtpfs pulls
    /// the whole file on the first read under a global device lock, so
    /// running those on the main pool would queue four full downloads ahead
    /// of every readdir/stat the render thread needs from the phone.
    slow_queue: Arc<Lane>,
    rx: mpsc::Receiver<ThumbResult>,
    /// Keys queued or in flight.
    pending: HashSet<String>,
    /// The pending keys asked for since the last `end_frame`, in the order
    /// they were asked for (the top of the view first).
    wanted: Vec<String>,
}

impl ThumbPool {
    pub fn new() -> Self {
        let queue: Arc<Lane> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        // 2–4 workers: enough to hide decode latency, few enough that
        // concurrent decode allocations stay bounded.
        let workers = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).clamp(2, 4))
            .unwrap_or(2);
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            std::thread::spawn(move || worker_loop(queue, tx));
        }
        let slow_queue: Arc<Lane> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        {
            let queue = Arc::clone(&slow_queue);
            let tx = tx.clone();
            std::thread::spawn(move || worker_loop(queue, tx));
        }
        Self {
            queue,
            slow_queue,
            rx,
            pending: HashSet::new(),
            wanted: Vec::new(),
        }
    }

    /// Queue a thumbnail for a row on screen. `slow`: on the slow lane,
    /// one job at a time, never competing with the main pool (files on MTP
    /// and network mounts).
    pub fn submit(&mut self, key: String, path: PathBuf, kind: ThumbKind, slow: bool) {
        if !self.pending.insert(key.clone()) {
            return;
        }
        self.wanted.push(key.clone());
        let (lock, cv) = if slow {
            &*self.slow_queue
        } else {
            &*self.queue
        };
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(ThumbJob { key, path, kind });
        cv.notify_one();
    }

    /// A row on screen asks for `key` again. True when its job is queued or
    /// in flight (and there is nothing to submit).
    pub fn want(&mut self, key: &str) -> bool {
        if !self.pending.contains(key) {
            return false;
        }
        self.wanted.push(key.to_string());
        true
    }

    /// Every view has asked for what it shows: drop the queued jobs nobody
    /// asked for this frame and put the rest in the order of the asking.
    /// Jobs a worker already took finish normally.
    pub fn end_frame(&mut self) {
        if !self.pending.is_empty() {
            let mut rank: HashMap<&str, usize> = HashMap::with_capacity(self.wanted.len());
            for (i, key) in self.wanted.iter().enumerate() {
                rank.entry(key.as_str()).or_insert(i);
            }
            for (lock, _) in [&*self.queue, &*self.slow_queue] {
                let dropped = follow_screen(&mut lock.lock().unwrap_or_else(|e| e.into_inner()), &rank);
                for key in dropped {
                    self.pending.remove(&key);
                }
            }
        }
        self.wanted.clear();
    }

    /// True while jobs are queued or in flight.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn try_recv(&mut self) -> Option<ThumbResult> {
        let result = self.rx.try_recv().ok()?;
        self.pending.remove(&result.key);
        Some(result)
    }
}

/// Keep the jobs `rank` names, lowest rank first; hand back the keys of the
/// others.
fn follow_screen(queue: &mut VecDeque<ThumbJob>, rank: &HashMap<&str, usize>) -> Vec<String> {
    let mut kept: Vec<(usize, ThumbJob)> = Vec::with_capacity(queue.len());
    let mut dropped = Vec::new();
    for job in queue.drain(..) {
        match rank.get(job.key.as_str()) {
            Some(&r) => kept.push((r, job)),
            None => dropped.push(job.key),
        }
    }
    kept.sort_by_key(|(r, _)| *r);
    queue.extend(kept.into_iter().map(|(_, job)| job));
    dropped
}

fn worker_loop(queue: Arc<Lane>, tx: mpsc::Sender<ThumbResult>) {
    loop {
        let job = {
            let (lock, cv) = &*queue;
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = q.pop_front() {
                    break job;
                }
                q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        // A decoder panicking on a corrupt file must still produce a result:
        // otherwise the key sits in `pending` forever (the window redraws at
        // 60 fps waiting for it) and this worker is gone for good.
        let rgba = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generate(&job.path, job.kind)
        }))
        .unwrap_or(None);
        if tx.send(ThumbResult { key: job.key, rgba }).is_err() {
            return; // IconCache dropped — shutting down
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(key: &str) -> ThumbJob {
        ThumbJob {
            key: key.to_string(),
            path: PathBuf::from(key),
            kind: ThumbKind::Image,
        }
    }

    fn keys(queue: &VecDeque<ThumbJob>) -> Vec<&str> {
        queue.iter().map(|j| j.key.as_str()).collect()
    }

    #[test]
    fn the_queue_is_cut_down_to_what_is_on_screen_in_screen_order() {
        // Queued while the user scrolled past rows a..e; the screen now
        // shows d, e and the new rows f, g, with f above d.
        let mut queue: VecDeque<ThumbJob> = ["a", "b", "c", "d", "e", "g", "f"]
            .into_iter()
            .map(job)
            .collect();
        let on_screen = ["f", "d", "e", "g"];
        let rank: HashMap<&str, usize> =
            on_screen.iter().enumerate().map(|(i, k)| (*k, i)).collect();

        let dropped = follow_screen(&mut queue, &rank);

        assert_eq!(keys(&queue), on_screen);
        assert_eq!(dropped, ["a", "b", "c"]);
    }

    #[test]
    fn nothing_on_screen_empties_the_queue() {
        let mut queue: VecDeque<ThumbJob> = ["a", "b"].into_iter().map(job).collect();
        let dropped = follow_screen(&mut queue, &HashMap::new());
        assert!(queue.is_empty());
        assert_eq!(dropped, ["a", "b"]);
    }

    /// A pool without workers: jobs stay queued, so the bookkeeping can be
    /// looked at.
    fn idle_pool() -> ThumbPool {
        let lane = || -> Arc<Lane> { Arc::new((Mutex::new(VecDeque::new()), Condvar::new())) };
        let (_tx, rx) = mpsc::channel();
        ThumbPool {
            queue: lane(),
            slow_queue: lane(),
            rx,
            pending: HashSet::new(),
            wanted: Vec::new(),
        }
    }

    fn queued(pool: &ThumbPool) -> Vec<String> {
        let (lock, _) = &*pool.queue;
        lock.lock().unwrap().iter().map(|j| j.key.clone()).collect()
    }

    #[test]
    fn a_row_that_left_the_screen_is_dropped_and_can_be_asked_for_again() {
        let mut pool = idle_pool();
        // Frame 1 shows a and b.
        for key in ["a", "b"] {
            assert!(!pool.want(key));
            pool.submit(key.to_string(), PathBuf::from(key), ThumbKind::Image, false);
        }
        pool.end_frame();
        assert_eq!(queued(&pool), ["a", "b"]);

        // Frame 2: scrolled. b is still there, c is new and above it.
        assert!(!pool.want("c"));
        pool.submit("c".to_string(), PathBuf::from("c"), ThumbKind::Image, false);
        assert!(pool.want("b"));
        pool.end_frame();
        assert_eq!(queued(&pool), ["c", "b"]);
        assert!(pool.has_pending());

        // a is no longer pending: when its row comes back it is submitted
        // again instead of waiting for a job that no longer exists.
        assert!(!pool.want("a"));

        // Frame 3: nothing with a thumbnail on screen.
        pool.end_frame();
        assert!(queued(&pool).is_empty());
        assert!(!pool.has_pending());
    }

    #[test]
    fn the_slow_lane_follows_the_screen_too() {
        let mut pool = idle_pool();
        pool.submit("phone/1".to_string(), PathBuf::from("1"), ThumbKind::Image, true);
        pool.submit("phone/2".to_string(), PathBuf::from("2"), ThumbKind::Image, true);
        pool.end_frame();
        assert!(pool.want("phone/2"));
        pool.end_frame();
        let (lock, _) = &*pool.slow_queue;
        let left: Vec<String> = lock.lock().unwrap().iter().map(|j| j.key.clone()).collect();
        assert_eq!(left, ["phone/2"]);
        assert!(queued(&pool).is_empty());
    }

    #[test]
    fn submitting_a_pending_key_twice_queues_it_once() {
        let mut pool = idle_pool();
        pool.submit("a".to_string(), PathBuf::from("a"), ThumbKind::Image, false);
        pool.submit("a".to_string(), PathBuf::from("a"), ThumbKind::Image, false);
        assert_eq!(queued(&pool), ["a"]);
    }
}

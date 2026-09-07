//! Video thumbnails for the Files view: an on-disk PNG cache filled by a
//! bounded ffmpeg worker pool.
//!
//! The Files view asks for a thumbnail for *every* video in the listed
//! directory. The old implementation spawned one thread plus one
//! `ffmpeg` per file at once — a folder of a few hundred clips meant a
//! few hundred concurrent decoders. Now requests go through a FIFO
//! drained by at most [`MAX_WORKERS`] threads, each running a single
//! ffmpeg at a time.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Concurrent ffmpeg extractions. Two keeps a video-heavy folder from
/// pegging every core while still filling the visible rows quickly.
const MAX_WORKERS: usize = 2;

struct ThumbJob {
    src: PathBuf,
    dst: PathBuf,
    size: u32,
}

struct Pool {
    queue: VecDeque<ThumbJob>,
    /// Sources queued or in flight — dedupes repeated requests from
    /// every re-list of the same directory.
    pending: HashSet<PathBuf>,
    /// Worker threads currently alive.
    workers: usize,
}

fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(|| {
        Mutex::new(Pool {
            queue: VecDeque::new(),
            pending: HashSet::new(),
            workers: 0,
        })
    })
}

/// Disk cache location for a video thumbnail. Keyed by hash of the
/// absolute path so two files at the same name don't collide. Public
/// so the Files view can probe existence before pushing an IconRequest.
pub fn video_thumb_path(src: &Path) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut dir = PathBuf::from(home);
    dir.push(".cache/lntrn-cc/thumbs");
    let _ = std::fs::create_dir_all(&dir);
    let key = simple_hash(src.to_string_lossy().as_bytes());
    dir.push(format!("{:016x}.png", key));
    dir
}

/// Cheap FNV-1a hash — good enough for cache keys, not cryptographic.
fn simple_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Queue a background ffmpeg job to extract `src`'s thumbnail into the
/// on-disk cache. Idempotent — a second call for the same path while a
/// previous job is queued or running is a no-op. Returns immediately;
/// the cache file appears whenever ffmpeg finishes.
pub fn ensure_video_thumb_async(src: PathBuf, size: u32) {
    let dst = video_thumb_path(&src);
    if dst.exists() {
        return;
    }
    let Ok(mut p) = pool().lock() else { return };
    if p.pending.contains(&src) {
        return;
    }
    p.pending.insert(src.clone());
    p.queue.push_back(ThumbJob { src, dst, size });
    if p.workers < MAX_WORKERS {
        p.workers += 1;
        let spawned = std::thread::Builder::new()
            .name("video-thumbs".into())
            .spawn(worker)
            .is_ok();
        if !spawned {
            p.workers -= 1;
        }
    }
}

/// Drain the queue until it's empty, then exit. The empty-check and the
/// worker-count decrement happen under the same lock as the enqueue
/// path's `workers < MAX_WORKERS` test, so a job can never be queued
/// with no worker alive to take it.
fn worker() {
    loop {
        let job = {
            let Ok(mut p) = pool().lock() else { return };
            match p.queue.pop_front() {
                Some(job) => job,
                None => {
                    p.workers -= 1;
                    return;
                }
            }
        };
        let _ = extract_video_thumb(&job.src, &job.dst, job.size);
        if let Ok(mut p) = pool().lock() {
            p.pending.remove(&job.src);
        }
    }
}

/// Shell out to ffmpeg to extract a thumbnail frame at ~1s in. PNG so
/// our existing decoder can pick it up. Returns whether the file now
/// exists.
fn extract_video_thumb(src: &Path, dst: &Path, size: u32) -> bool {
    use std::process::{Command, Stdio};
    let scale = format!("scale={}:-1:force_original_aspect_ratio=decrease", size);
    let status = Command::new("ffmpeg")
        .args(["-y", "-ss", "1", "-i"])
        .arg(src)
        .args(["-vframes", "1", "-vf"])
        .arg(&scale)
        .arg(dst)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(status, Ok(s) if s.success() && dst.exists())
}

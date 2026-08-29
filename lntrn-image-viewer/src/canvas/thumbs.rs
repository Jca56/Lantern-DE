//! Background thumbnail pipeline for the canvas sidebar — adapted from
//! lntrn-file-manager's thumbs.rs (worker pool + on-disk cache), minus the
//! ffmpeg video path since the canvas is image-only.
//!
//! The render thread never decodes media: it submits jobs and drains finished
//! RGBA buffers each frame. The disk cache is keyed by hash(path, mtime, size)
//! so edited files regenerate and revisited folders load near-instantly.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

pub const THUMB_SIZE: u32 = 320;

/// Decode guards: oversized sources fail cleanly instead of ballooning RAM.
const MAX_DECODE_DIM: u32 = 16_384;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

pub struct ThumbResult {
    pub key: String,
    /// `None` = generation failed; callers record the key so the file isn't
    /// retried every frame.
    pub rgba: Option<(Vec<u8>, u32, u32)>,
}

struct ThumbJob {
    key: String,
    path: PathBuf,
}

/// Fixed-size worker pool. Workers block on a condvar when idle.
pub struct ThumbPool {
    queue: Arc<(Mutex<VecDeque<ThumbJob>>, Condvar)>,
    rx: mpsc::Receiver<ThumbResult>,
}

impl ThumbPool {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let workers = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).clamp(2, 4))
            .unwrap_or(2);
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            std::thread::spawn(move || worker_loop(queue, tx));
        }
        Self { queue, rx }
    }

    pub fn submit(&self, key: String, path: PathBuf) {
        let (lock, cv) = &*self.queue;
        lock.lock().unwrap().push_back(ThumbJob { key, path });
        cv.notify_one();
    }

    /// Drop queued (not yet started) jobs, returning their keys so the caller
    /// can clear its pending set. In-flight jobs finish normally.
    pub fn clear_queue(&self) -> Vec<String> {
        let (lock, _) = &*self.queue;
        lock.lock().unwrap().drain(..).map(|j| j.key).collect()
    }

    pub fn try_recv(&self) -> Option<ThumbResult> {
        self.rx.try_recv().ok()
    }
}

fn worker_loop(queue: Arc<(Mutex<VecDeque<ThumbJob>>, Condvar)>, tx: mpsc::Sender<ThumbResult>) {
    loop {
        let job = {
            let (lock, cv) = &*queue;
            let mut q = lock.lock().unwrap();
            loop {
                if let Some(job) = q.pop_front() {
                    break job;
                }
                q = cv.wait(q).unwrap();
            }
        };
        let rgba = generate(&job.path);
        if tx.send(ThumbResult { key: job.key, rgba }).is_err() {
            return; // pool dropped — shutting down
        }
    }
}

// ── Disk cache ───────────────────────────────────────────────────────────────

fn thumb_cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".cache/lntrn-image-viewer/thumbs")
}

fn cache_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    // Size is part of the key so bumping THUMB_SIZE regenerates old caches.
    THUMB_SIZE.hash(&mut h);
    if let Ok(md) = std::fs::metadata(path) {
        md.len().hash(&mut h);
        if let Ok(m) = md.modified() {
            if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                d.as_secs().hash(&mut h);
                d.subsec_nanos().hash(&mut h);
            }
        }
    }
    thumb_cache_dir().join(format!("{:016x}.png", h.finish()))
}

// ── Generation (worker threads — no GPU access) ──────────────────────────────

fn generate(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let cached = cache_file(path);
    if let Ok(img) = image::open(&cached) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        return Some((rgba.into_raw(), w, h));
    }

    let thumb = if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        rasterize_svg_file(path)?
    } else {
        decode_image_limited(path)?
    };

    let _ = std::fs::create_dir_all(thumb_cache_dir());
    let _ = thumb.save(&cached);

    let (w, h) = thumb.dimensions();
    Some((thumb.into_raw(), w, h))
}

fn decode_image_limited(path: &Path) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    Some(img.thumbnail(THUMB_SIZE, THUMB_SIZE).to_rgba8())
}

fn rasterize_svg_file(path: &Path) -> Option<image::RgbaImage> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = (THUMB_SIZE as f32 / size.width()).min(THUMB_SIZE as f32 / size.height());
    let w = (size.width() * scale).ceil() as u32;
    let h = (size.height() * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1))?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    image::RgbaImage::from_raw(w.max(1), h.max(1), pixmap.take())
}

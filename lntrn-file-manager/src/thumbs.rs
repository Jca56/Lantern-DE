//! Background thumbnail pipeline: a bounded worker pool plus an on-disk
//! cache shared by image, SVG, video and audio-artwork thumbnails.
//!
//! The render thread never decodes media. It submits jobs via [`ThumbPool`]
//! and drains finished RGBA buffers each frame (`IconCache::poll_thumbs`).
//! Workers check `~/.cache/lntrn-file-manager/thumbs/` first — cache files
//! are keyed by hash(path, mtime, size) so edited files regenerate and
//! revisited folders load near-instantly across app restarts.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

/// Max dimension of generated thumbnails (matches icons::ICON_RENDER_SIZE).
pub const THUMB_SIZE: u32 = 192;

/// Decode guards: a source image exceeding these fails cleanly to the
/// generic icon instead of ballooning RAM (full-res RGBA of a panorama can
/// be GBs — decoding several at once is what OOM-killed the app before).
const MAX_DECODE_DIM: u32 = 16_384;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

/// What a worker should do with the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbKind {
    /// Raster or SVG image file.
    Image,
    /// Video — representative frame via ffmpeg.
    Video,
    /// WAV / MP3 — embedded cover art via `audio_tags`.
    Audio,
}

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

/// Fixed-size worker pool. Workers block on a condvar when idle and live for
/// the process lifetime.
pub struct ThumbPool {
    queue: Arc<(Mutex<VecDeque<ThumbJob>>, Condvar)>,
    /// Single-worker lane for slow mounts (MTP phones, sshfs). jmtpfs pulls
    /// the whole file on the first read under a global device lock, so
    /// running those on the main pool would queue four full downloads ahead
    /// of every readdir/stat the render thread needs from the phone.
    slow_queue: Arc<(Mutex<VecDeque<ThumbJob>>, Condvar)>,
    rx: mpsc::Receiver<ThumbResult>,
}

impl ThumbPool {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
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
        let slow_queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        {
            let queue = Arc::clone(&slow_queue);
            let tx = tx.clone();
            std::thread::spawn(move || worker_loop(queue, tx));
        }
        Self {
            queue,
            slow_queue,
            rx,
        }
    }

    pub fn submit(&self, key: String, path: PathBuf, kind: ThumbKind) {
        Self::push(&self.queue, ThumbJob { key, path, kind });
    }

    /// Queue on the slow lane: one job at a time, never competing with the
    /// main pool. For files on MTP / network mounts.
    pub fn submit_slow(&self, key: String, path: PathBuf, kind: ThumbKind) {
        Self::push(&self.slow_queue, ThumbJob { key, path, kind });
    }

    fn push(queue: &(Mutex<VecDeque<ThumbJob>>, Condvar), job: ThumbJob) {
        let (lock, cv) = queue;
        lock.lock().unwrap().push_back(job);
        cv.notify_one();
    }

    /// Drop all queued (not yet started) jobs on both lanes, returning their
    /// keys so the caller can clear its pending set. In-flight jobs finish
    /// normally.
    pub fn clear_queue(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for (lock, _) in [&*self.queue, &*self.slow_queue] {
            keys.extend(lock.lock().unwrap().drain(..).map(|j| j.key));
        }
        keys
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
        let rgba = generate(&job.path, job.kind);
        if tx.send(ThumbResult { key: job.key, rgba }).is_err() {
            return; // IconCache dropped — shutting down
        }
    }
}

// ── Disk cache ───────────────────────────────────────────────────────────────

fn thumb_cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".cache/lntrn-file-manager/thumbs")
}

/// Cache filename derived from path + mtime + size: editing or replacing a
/// file changes the hash, so stale thumbnails are never served.
fn cache_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
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

// ── Generation ───────────────────────────────────────────────────────────────

/// Produce thumbnail RGBA for a path, via disk cache when possible. Runs on
/// worker threads (no GPU access); also used synchronously for the rare
/// custom-folder-icon path in icons.rs.
pub fn generate(path: &Path, kind: ThumbKind) -> Option<(Vec<u8>, u32, u32)> {
    let cached = cache_file(path);
    if let Ok(img) = image::open(&cached) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        return Some((rgba.into_raw(), w, h));
    }

    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let thumb = match kind {
        ThumbKind::Video => video_frame(path)?,
        ThumbKind::Audio => audio_artwork(path)?,
        ThumbKind::Image if is_svg => rasterize_svg_file(path)?,
        ThumbKind::Image => decode_image_limited(path)?,
    };

    let _ = std::fs::create_dir_all(thumb_cache_dir());
    let _ = thumb.save(&cached);

    let (w, h) = thumb.dimensions();
    Some((thumb.into_raw(), w, h))
}

/// Decode with explicit limits so a huge or malicious file errors out
/// instead of exhausting memory, then downscale to thumbnail size.
fn decode_image_limited(path: &Path) -> Option<image::RgbaImage> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    decode_limited(reader)
}

fn decode_limited<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<R>,
) -> Option<image::RgbaImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    Some(img.thumbnail(THUMB_SIZE, THUMB_SIZE).to_rgba8())
}

/// Cover art embedded in a WAV (`id3 ` chunk) or MP3 (APIC frame). Files
/// without artwork return None and fall back to the procedural note icon.
fn audio_artwork(path: &Path) -> Option<image::RgbaImage> {
    let art = crate::audio_tags::read(path).ok()?.tags.artwork?;
    let reader = image::ImageReader::new(std::io::Cursor::new(art.data))
        .with_guessed_format()
        .ok()?;
    decode_limited(reader)
}

fn rasterize_svg_file(path: &Path) -> Option<image::RgbaImage> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = (THUMB_SIZE as f32 / size.width()).min(THUMB_SIZE as f32 / size.height());
    let w = (size.width() * scale).ceil() as u32;
    let h = (size.height() * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    image::RgbaImage::from_raw(w, h, pixmap.take())
}

/// Extract a representative frame via ffmpeg, already scaled to thumb size.
fn video_frame(path: &Path) -> Option<image::RgbaImage> {
    use std::process::Command;
    let output = Command::new("ffmpeg")
        .args(["-ss", "1", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!(
                "scale={s}:{s}:force_original_aspect_ratio=decrease",
                s = THUMB_SIZE
            ),
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-loglevel",
            "error",
            "pipe:1",
        ])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(image::load_from_memory(&output.stdout).ok()?.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RIFF/WAVE, 8 kHz mono 8-bit, 100 samples of silence.
    fn minimal_wav() -> Vec<u8> {
        let mut fmt = Vec::new();
        for v in [1u16, 1] {
            fmt.extend_from_slice(&v.to_le_bytes());
        }
        for v in [8000u32, 8000] {
            fmt.extend_from_slice(&v.to_le_bytes());
        }
        for v in [1u16, 8] {
            fmt.extend_from_slice(&v.to_le_bytes());
        }
        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&100u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 100]);
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    /// Single test fn: `generate` reads `$HOME`, so the env override must
    /// not race a parallel test.
    #[test]
    fn generate_thumbnail_and_disk_cache() {
        let tmp = std::env::temp_dir().join(format!("lntrn-fm-thumb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HOME", &tmp);

        // Generation: a 512×300 source becomes a ≤192px RGBA thumb and
        // leaves one file in the disk cache.
        let src = tmp.join("src.png");
        image::RgbaImage::from_fn(512, 300, |x, _| image::Rgba([x as u8, 80, 200, 255]))
            .save(&src)
            .unwrap();
        let (rgba, w, h) = generate(&src, ThumbKind::Image).expect("thumbnail should generate");
        assert!(w <= THUMB_SIZE && h <= THUMB_SIZE);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let cache_files = || std::fs::read_dir(thumb_cache_dir()).unwrap().count();
        assert_eq!(cache_files(), 1);

        // Cache hit: overwrite the cached PNG with a recognizable 10×10 and
        // confirm a second call serves it instead of re-decoding the source.
        let cached = cache_file(&src);
        image::RgbaImage::from_pixel(10, 10, image::Rgba([255, 0, 0, 255]))
            .save(&cached)
            .unwrap();
        let (_, w2, h2) = generate(&src, ThumbKind::Image).expect("cache hit should succeed");
        assert_eq!((w2, h2), (10, 10));

        // Failure: garbage bytes with an image extension must return None,
        // not panic, and must not pollute the cache.
        let bad = tmp.join("bad.jpg");
        std::fs::write(&bad, b"definitely not a jpeg").unwrap();
        assert!(generate(&bad, ThumbKind::Image).is_none());
        assert_eq!(cache_files(), 1);

        // Audio: a WAV with embedded PNG cover art thumbnails to that art
        // (and lands in the disk cache); a tag-less WAV yields None.
        let wav = tmp.join("track.wav");
        std::fs::write(&wav, minimal_wav()).unwrap();
        assert!(generate(&wav, ThumbKind::Audio).is_none());
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(64, 48, image::Rgba([0, 200, 120, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let tags = crate::audio_tags::AudioTags {
            artwork: Some(crate::audio_tags::Artwork {
                mime: "image/png".into(),
                data: png,
            }),
            ..Default::default()
        };
        crate::audio_tags::wav::write(&wav, &tags).unwrap();
        let (rgba, w, h) = generate(&wav, ThumbKind::Audio).expect("artwork thumb");
        // Scaled to the thumb box (small sources upscale, like image thumbs)
        // with the 4:3 aspect kept and the solid colour intact.
        assert!(w <= THUMB_SIZE && h <= THUMB_SIZE);
        assert_eq!(w * 48, h * 64);
        assert_eq!(&rgba[..4], &[0, 200, 120, 255]);
        assert_eq!(cache_files(), 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

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
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(job);
        cv.notify_one();
    }

    /// Drop all queued (not yet started) jobs on both lanes, returning their
    /// keys so the caller can clear its pending set. In-flight jobs finish
    /// normally.
    pub fn clear_queue(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for (lock, _) in [&*self.queue, &*self.slow_queue] {
            keys.extend(
                lock.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .drain(..)
                    .map(|j| j.key),
            );
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
    thumb_cache_dir().join(format!("{CACHE_GEN}{:016x}.png", h.finish()))
}

/// Prefix of the current generation of cache files. Bump it when the pixels
/// a given source produces change (v2: EXIF orientation applied, SVG alpha
/// no longer premultiplied twice); files of older generations are removed
/// the first time a thumbnail is generated.
const CACHE_GEN: &str = "v2-";

fn prune_old_generations() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(rd) = std::fs::read_dir(thumb_cache_dir()) else {
            return;
        };
        for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".png") && !name.starts_with(CACHE_GEN) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    });
}

// ── Generation ───────────────────────────────────────────────────────────────

/// Produce thumbnail RGBA for a path, via disk cache when possible. Runs on
/// worker threads (no GPU access); also used synchronously for the rare
/// custom-folder-icon path in icons.rs.
pub fn generate(path: &Path, kind: ThumbKind) -> Option<(Vec<u8>, u32, u32)> {
    // Regular files only. A FIFO named *.png blocks the worker forever on
    // open; a symlink to /dev/zero reads without end.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return None;
    }
    prune_old_generations();
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
    reader: image::ImageReader<R>,
) -> Option<image::RgbaImage> {
    // Shrink first, turn second: rotating the full-size decode would need a
    // second full-size buffer per worker. The thumbnail box is square, so
    // the result is the same picture.
    let (img, orientation) = decode_unrotated(reader, MAX_DECODE_DIM, MAX_DECODE_BYTES)?;
    let mut thumb = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
    thumb.apply_orientation(orientation);
    Some(thumb.to_rgba8())
}

/// Decode within the given limits and turn the picture the way its EXIF
/// orientation says — phone photos are stored sideways with a tag telling
/// the viewer to rotate them. Shared with Quick Look.
pub fn decode_oriented<R: std::io::BufRead + std::io::Seek>(
    reader: image::ImageReader<R>,
    max_dim: u32,
    max_bytes: u64,
) -> Option<image::DynamicImage> {
    let (mut img, orientation) = decode_unrotated(reader, max_dim, max_bytes)?;
    img.apply_orientation(orientation);
    Some(img)
}

/// The decode itself, with the orientation the file asks for returned
/// beside the still-unrotated image.
fn decode_unrotated<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<R>,
    max_dim: u32,
    max_bytes: u64,
) -> Option<(image::DynamicImage, image::metadata::Orientation)> {
    use image::ImageDecoder;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dim);
    limits.max_image_height = Some(max_dim);
    limits.max_alloc = Some(max_bytes);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    // `into_decoder` hands the limits to the decoder but skips the check of
    // the output buffer that `decode()` adds on top; repeat it here.
    if decoder.total_bytes() > max_bytes {
        return None;
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let img = image::DynamicImage::from_decoder(decoder).ok()?;
    Some((img, orientation))
}

/// An SVG file's bytes, refusing anything that is not a regular file of a
/// sane size (the whole file is read into memory and parsed).
pub fn read_svg_capped(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    const MAX_SVG_BYTES: u64 = 16 * 1024 * 1024;
    use std::os::unix::fs::OpenOptionsExt;
    // Non-blocking open: opening a writer-less FIFO named *.svg would
    // otherwise block for good before the is_file check can reject it (this
    // runs on the render thread for custom folder icons). No effect on
    // regular files. NOCTTY: never adopt a terminal behind a symlink.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_SVG_BYTES {
        return None;
    }
    let mut data = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_SVG_BYTES).read_to_end(&mut data).ok()?;
    Some(data)
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
    let data = read_svg_capped(path)?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = (THUMB_SIZE as f32 / size.width()).min(THUMB_SIZE as f32 / size.height());
    let w = (size.width() * scale).ceil() as u32;
    let h = (size.height() * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia renders premultiplied alpha; everything downstream (PNG
    // cache, the texture shader) expects straight alpha.
    image::RgbaImage::from_raw(w, h, pixmap.take_demultiplied())
}

/// Extract a representative frame via ffmpeg, already scaled to thumb size.
fn video_frame(path: &Path) -> Option<image::RgbaImage> {
    let png = ffmpeg_frame_png(path, Some(THUMB_SIZE))?;
    Some(image::load_from_memory(&png).ok()?.to_rgba8())
}

/// One frame of a video as PNG bytes, optionally scaled to fit `fit` px.
/// Taken one second in (past the usual black first frame); a clip shorter
/// than that yields nothing there, so the first frame is the fallback.
/// Shared with Quick Look. `None` covers both "no ffmpeg" and "no frame".
pub fn ffmpeg_frame_png(path: &Path, fit: Option<u32>) -> Option<Vec<u8>> {
    use std::process::Command;
    for seek in ["1", "0"] {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-ss", seek, "-i"]).arg(path).args(["-frames:v", "1"]);
        if let Some(s) = fit {
            cmd.args([
                "-vf",
                &format!("scale={s}:{s}:force_original_aspect_ratio=decrease"),
            ]);
        }
        cmd.args([
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-loglevel",
            "error",
            "pipe:1",
        ]);
        let output = cmd.output().ok()?;
        if output.status.success() && !output.stdout.is_empty() {
            return Some(output.stdout);
        }
    }
    None
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

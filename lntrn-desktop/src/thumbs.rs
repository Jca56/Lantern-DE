//! Background thumbnail generation for image files on the desktop.
//!
//! Decoding a multi-megapixel JPEG can take 100ms+, so all decode work
//! runs on a dedicated worker thread. The worker produces a square
//! `size×size` RGBA buffer (the image contain-fit and centered, with
//! transparent margins); the main thread uploads it to a GPU texture and
//! swaps it in for the generic image glyph. Until a thumbnail is ready the
//! item keeps showing the glyph, so the desktop never blocks on decoding.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::SystemTime;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};
use resvg::tiny_skia;
use resvg::usvg;

/// Largest source file we'll try to decode for a thumbnail. Bigger files
/// keep the generic glyph rather than risk stalling the worker or spiking
/// memory on a stray huge image.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

struct Request {
    path: PathBuf,
    size: u32,
    /// Modification time captured when the decode was queued — stored on the
    /// resulting entry so a later in-place edit can be detected.
    mtime: SystemTime,
}

struct Done {
    path: PathBuf,
    /// Size the thumbnail was rendered at — used to drop results whose
    /// target size went stale because the output scale changed mid-decode.
    size: u32,
    mtime: SystemTime,
    /// Square `size*size*4` RGBA, or None if the file couldn't be decoded.
    rgba: Option<Vec<u8>>,
}

/// A decoded thumbnail plus the file mtime it was decoded from.
struct Ready {
    tex: GpuTexture,
    mtime: SystemTime,
}

pub struct ThumbCache {
    ready: HashMap<PathBuf, Ready>,
    inflight: HashSet<PathBuf>,
    /// Files that failed to decode, with the mtime that failed — a later
    /// edit (different mtime) is retried, but the same bytes aren't.
    failed: HashMap<PathBuf, SystemTime>,
    tx: Sender<Request>,
    rx: Receiver<Done>,
    size_px: u32,
}

impl ThumbCache {
    pub fn new(size_px: u32) -> Self {
        let (tx, worker_rx) = std::sync::mpsc::channel::<Request>();
        let (worker_tx, rx) = std::sync::mpsc::channel::<Done>();
        thread::Builder::new()
            .name("lntrn-thumbs".into())
            .spawn(move || {
                // recv() errors once the ThumbCache (and its `tx`) is
                // dropped, which ends the loop and the thread.
                while let Ok(req) = worker_rx.recv() {
                    let rgba = decode(&req.path, req.size);
                    let _ = worker_tx.send(Done {
                        path: req.path,
                        size: req.size,
                        mtime: req.mtime,
                        rgba,
                    });
                }
            })
            .expect("spawn thumbnail thread");
        Self {
            ready: HashMap::new(),
            inflight: HashSet::new(),
            failed: HashMap::new(),
            tx,
            rx,
            size_px: size_px.max(32),
        }
    }

    /// Queue a thumbnail decode for `path` at the given file mtime, unless the
    /// format isn't supported, a decode is already in flight, or we already
    /// have an up-to-date result (ready or failed) for that exact mtime. A
    /// newer mtime re-decodes; the stale thumbnail keeps showing until the
    /// fresh one lands, so there's no glyph flash on edit.
    pub fn request(&mut self, path: &Path, mtime: SystemTime) {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !is_thumbnailable(ext) {
            return;
        }
        if self.inflight.contains(path) {
            return;
        }
        if self.ready.get(path).is_some_and(|r| r.mtime == mtime) {
            return;
        }
        if self.failed.get(path).is_some_and(|&m| m == mtime) {
            return;
        }
        self.inflight.insert(path.to_path_buf());
        let _ = self.tx.send(Request {
            path: path.to_path_buf(),
            size: self.size_px,
            mtime,
        });
    }

    /// Upload any finished decodes to GPU textures. Returns true if at least
    /// one new thumbnail became ready (the caller should repaint).
    pub fn drain(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            self.inflight.remove(&done.path);
            // Stale size (output scale changed while decoding) — re-requested
            // at the new size on the next pass.
            if done.size != self.size_px {
                continue;
            }
            match done.rgba {
                Some(rgba) => {
                    let tex = tex_pass.upload(gpu, &rgba, done.size, done.size);
                    self.failed.remove(&done.path);
                    self.ready.insert(
                        done.path,
                        Ready {
                            tex,
                            mtime: done.mtime,
                        },
                    );
                    changed = true;
                }
                None => {
                    self.failed.insert(done.path, done.mtime);
                }
            }
        }
        changed
    }

    /// A ready thumbnail texture for `path`, if one has been decoded.
    pub fn get(&self, path: &Path) -> Option<&GpuTexture> {
        self.ready.get(path).map(|r| &r.tex)
    }

    /// True while decodes are outstanding — the caller keeps the frame pump
    /// alive so finished thumbnails get painted on an otherwise idle desktop.
    pub fn has_pending(&self) -> bool {
        !self.inflight.is_empty()
    }

    /// Reset on output-scale change. In-flight decodes finish but are
    /// discarded by the size check in `drain`; failures are forgotten so a
    /// re-request happens at the new size.
    pub fn clear(&mut self, size_px: u32) {
        self.ready.clear();
        self.inflight.clear();
        self.failed.clear();
        self.size_px = size_px.max(32);
    }
}

/// True if `ext` is an image format we can render a thumbnail for.
pub fn is_thumbnailable(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" | "tif"
    )
}

fn decode(path: &Path, size: u32) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("svg") {
        decode_svg(path, size)
    } else {
        decode_raster(path, size)
    }
}

fn decode_svg(path: &Path, size: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    // Default options — no font database. SVGs with `<text>` won't render
    // their text, but the vast majority of image-file SVGs are pure paths.
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &opt).ok()?;
    let svg_size = tree.size();
    let (w, h) = (svg_size.width(), svg_size.height());
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    // Contain-fit: scale by the larger dimension so the whole SVG fits.
    let scale = size as f32 / w.max(h);
    let render_w = ((w * scale).round() as u32).clamp(1, size);
    let render_h = ((h * scale).round() as u32).clamp(1, size);
    let mut pixmap = tiny_skia::Pixmap::new(render_w, render_h)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(center_into_square(pixmap.data(), render_w, render_h, size))
}

fn decode_raster(path: &Path, size: u32) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    // `resize` preserves aspect ratio within the bounds — i.e. contain-fit.
    // Lanczos3 keeps small thumbnails crisp; it's off the render thread so
    // the extra cost is hidden.
    let fitted = img.resize(size, size, image::imageops::FilterType::Lanczos3);
    let rgba = fitted.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some(center_into_square(rgba.as_raw(), w, h, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buf: &[u8], size: u32, x: u32, y: u32) -> u8 {
        buf[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn svg_thumbnail_contains_visible_pixels() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <circle cx="50" cy="50" r="48" fill="#ff0000"/></svg>"##;
        let path = std::env::temp_dir().join("lntrn-thumb-test.svg");
        std::fs::write(&path, svg).unwrap();

        let out = decode_svg(&path, 96).expect("svg decode");
        assert_eq!(out.len(), (96 * 96 * 4) as usize);
        // Center of the circle is opaque red; the very corner is transparent.
        let c = ((48 * 96 + 48) * 4) as usize;
        assert!(out[c] > 200 && out[c + 3] > 200, "center should be opaque red");
        assert_eq!(alpha_at(&out, 96, 0, 0), 0, "corner should be transparent");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn raster_thumbnail_is_contained_and_centered() {
        // 200×100 solid blue → contain-fit into 96×96 gives a 96×48 band,
        // centered vertically with transparent margins top and bottom.
        let img = image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 0, 255, 255]));
        let path = std::env::temp_dir().join("lntrn-thumb-test.png");
        img.save(&path).unwrap();

        let out = decode_raster(&path, 96).expect("raster decode");
        assert_eq!(out.len(), (96 * 96 * 4) as usize);
        // Middle row opaque blue, top/bottom margin rows transparent.
        let mid = ((48 * 96 + 48) * 4) as usize;
        assert!(out[mid + 2] > 200 && out[mid + 3] > 200, "center should be opaque blue");
        assert_eq!(alpha_at(&out, 96, 48, 0), 0, "top margin transparent");
        assert_eq!(alpha_at(&out, 96, 48, 95), 0, "bottom margin transparent");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unsupported_extensions_are_skipped() {
        assert!(is_thumbnailable("png"));
        assert!(is_thumbnailable("SVG"));
        assert!(!is_thumbnailable("txt"));
        assert!(!is_thumbnailable("heic"));
    }

    #[test]
    fn request_dedups_same_mtime_and_reissues_on_change() {
        use std::time::Duration;

        // No GPU needed — the `failed` path carries the same mtime comparison
        // as `ready` but needs no texture, so we drive it directly. Simulating
        // a failed decode lets us exercise dedup-vs-reissue without a GPU.
        let mut cache = ThumbCache::new(96);
        let path = std::env::temp_dir().join("lntrn-thumb-mtime.png");
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(2000);

        cache.request(&path, t0);
        assert!(cache.has_pending(), "first request should queue a decode");

        // Same mtime while in flight → no duplicate queue (still just one).
        cache.request(&path, t0);

        // Simulate the worker finishing the t0 decode as a failure.
        cache.inflight.remove(&path);
        cache.failed.insert(path.clone(), t0);

        // Same mtime → skip; newer mtime → re-queue.
        cache.request(&path, t0);
        assert!(!cache.has_pending(), "same mtime should not re-decode");
        cache.request(&path, t1);
        assert!(cache.has_pending(), "newer mtime should re-decode");
    }
}

/// Composite an `sw×sh` RGBA buffer centered into a transparent `size×size`
/// RGBA buffer. `sw`/`sh` must be ≤ `size` (guaranteed by contain-fit).
fn center_into_square(src: &[u8], sw: u32, sh: u32, size: u32) -> Vec<u8> {
    let sw = sw.min(size);
    let sh = sh.min(size);
    let mut out = vec![0u8; (size * size * 4) as usize];
    let ox = (size - sw) / 2;
    let oy = (size - sh) / 2;
    let row_bytes = (sw * 4) as usize;
    for y in 0..sh {
        let src_start = (y * sw * 4) as usize;
        let dst_start = (((oy + y) * size + ox) * 4) as usize;
        out[dst_start..dst_start + row_bytes]
            .copy_from_slice(&src[src_start..src_start + row_bytes]);
    }
    out
}

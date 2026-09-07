//! Icon resolver and GPU cache.
//!
//! Given a `.desktop` `Icon=` value (theme name or absolute path), find
//! a matching SVG/PNG on disk, rasterize it once at the panel's icon
//! size, upload to a `GpuTexture`, and cache by app_id so we never pay
//! the rasterization cost twice.
//!
//! Resolution + rasterization run on a small pool of background threads.
//! Resolving an unknown name probes ~60 directories × 6 candidate
//! filenames (≈360 `stat` calls) and an SVG rasterize is several ms;
//! doing that on the render thread the first time the all-apps grid
//! opened stalled the frame for a visible beat. Now `ensure_loaded`
//! just enqueues, the workers decode to RGBA, and `pump()` uploads the
//! results to the GPU on the main thread a frame or two later. The
//! render loop treats a finished upload as a dirty frame so icons pop in
//! as soon as they're ready.
//!
//! Search order (mirrors freedesktop spec, biased toward modern icon
//! themes that look good on dark surfaces):
//!
//!   1. Absolute path (when the .desktop's `Icon=` starts with `/`).
//!   2. Tela (`/usr/share/icons/Tela/...`) — Lantern's preferred theme.
//!   3. hicolor scalable / 256 / 128 / 64 / 48
//!   4. Adwaita scalable
//!   5. breeze 48
//!   6. /usr/share/pixmaps
//!
//! All paths are scanned for `.svg`, `.svgz`, `.png` in that order.
//! Implementation is read-only-reference parallel to `lntrn-bar`'s; no
//! shared code per the self-contained-crate rule.

mod video_thumbs;

pub use video_thumbs::{ensure_video_thumb_async, video_thumb_path};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

/// Background rasterizer threads. Two is enough to stream a hundred
/// fresh icons in well under a second without competing with the
/// render thread for cores.
const WORKERS: usize = 2;

/// Work item for the rasterizer pool.
struct Job {
    app_id: String,
    icon_name: String,
    size: u32,
}

/// Finished rasterization. `rgba` is `None` when the icon couldn't be
/// resolved or decoded — cached as a negative entry so we never retry.
struct Done {
    app_id: String,
    rgba: Option<Vec<u8>>,
    size: u32,
}

/// LRU-ish cache: app_id → loaded GPU texture.
/// (We don't bound the cache size for now — typical user has <500 apps,
///  each ~16KB at 64×64 RGBA. ~8MB ceiling.)
pub struct IconCache {
    map: HashMap<String, Option<GpuTexture>>,
    /// Keys handed to the pool whose result hasn't come back yet.
    pending: HashSet<String>,
    /// Phys-pixel icon size for all entries in this cache. Cache is
    /// invalidated if scale changes (we re-rasterize at the new size).
    icon_size: u32,
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl IconCache {
    pub fn new(icon_size: u32) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for i in 0..WORKERS {
            let rx = Arc::clone(&job_rx);
            let tx = done_tx.clone();
            std::thread::Builder::new()
                .name(format!("icon-raster-{i}"))
                .spawn(move || worker(rx, tx))
                .ok();
        }
        Self {
            map: HashMap::new(),
            pending: HashSet::new(),
            icon_size,
            jobs,
            done,
        }
    }

    /// Make sure the icon for `app_id` is loaded or loading. Returns
    /// `true` if a request was enqueued this call; `false` if the slot is
    /// already populated or in flight. Callers usually don't care about
    /// the return value — they invoke `peek` afterward to grab whatever
    /// texture is available right now.
    pub fn ensure_loaded(
        &mut self,
        _gpu: &GpuContext,
        _tex_pass: &TexturePass,
        app_id: &str,
        icon_name: Option<&str>,
    ) -> bool {
        if self.map.contains_key(app_id) || self.pending.contains(app_id) {
            return false;
        }
        let Some(name) = icon_name else {
            // Nothing to look up — negative-cache right away.
            self.map.insert(app_id.to_string(), None);
            return true;
        };
        let job = Job {
            app_id: app_id.to_string(),
            icon_name: name.to_string(),
            size: self.icon_size,
        };
        if self.jobs.send(job).is_err() {
            // Pool is gone (all workers died) — degrade to "no icon"
            // rather than re-queueing forever.
            self.map.insert(app_id.to_string(), None);
            return true;
        }
        self.pending.insert(app_id.to_string());
        true
    }

    /// Upload every finished rasterization to the GPU. Returns how many
    /// new textures became available (so the caller can schedule a
    /// redraw). Cheap when nothing is pending.
    pub fn pump(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) -> usize {
        let mut fresh = 0;
        while let Ok(d) = self.done.try_recv() {
            self.pending.remove(&d.app_id);
            if d.size != self.icon_size {
                // Rasterized for a size we no longer use (resize() ran in
                // between). Drop it; the next frame re-requests at the
                // right size.
                continue;
            }
            let tex = d.rgba.map(|rgba| tex_pass.upload(gpu, &rgba, d.size, d.size));
            if tex.is_some() {
                fresh += 1;
            }
            self.map.insert(d.app_id, tex);
        }
        fresh
    }

    /// Read-only lookup. `None` while the icon is still loading (or if
    /// it could not be resolved at all).
    pub fn peek(&self, app_id: &str) -> Option<&GpuTexture> {
        self.map.get(app_id).and_then(|opt| opt.as_ref())
    }

    /// Drop everything and reload at a new size. Called when the
    /// fractional scale of the output changes.
    #[allow(dead_code)]
    pub fn resize(&mut self, new_icon_size: u32) {
        if new_icon_size != self.icon_size {
            self.map.clear();
            self.pending.clear();
            self.icon_size = new_icon_size;
        }
    }
}

/// Pool thread: pull a job, resolve + decode it off the render thread,
/// hand the RGBA back. A panic inside resvg / the image decoders on a
/// hostile file is caught so the key never gets stuck in `pending`.
fn worker(rx: Arc<Mutex<Receiver<Job>>>, tx: Sender<Done>) {
    loop {
        let job = {
            let Ok(guard) = rx.lock() else { return };
            guard.recv()
        };
        let Ok(job) = job else { return };
        let size = job.size;
        let rgba = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_path(&job.icon_name).and_then(|p| rasterize(&p, size))
        }))
        .unwrap_or(None);
        if tx
            .send(Done {
                app_id: job.app_id,
                rgba,
                size,
            })
            .is_err()
        {
            return;
        }
    }
}

// ── Path resolution ─────────────────────────────────────────────────────────

/// Resolve a freedesktop icon name (or absolute path) to a real file.
/// Returns the first match across the standard search dirs.
pub fn resolve_path(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        if p.exists() {
            return Some(p);
        }
    }

    // Lower-case fallback so e.g. `Firefox` also matches `firefox.svg`.
    let candidates = [name.to_string(), name.to_lowercase()];

    for dir in icon_dirs() {
        let dir = Path::new(&dir);
        if !dir.exists() {
            continue;
        }
        for cand in &candidates {
            for ext in &["svg", "svgz", "png"] {
                let p = dir.join(format!("{}.{}", cand, ext));
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn icon_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = Vec::with_capacity(16);

    // ~/.lantern/icons/ — canonical Lantern app-icon directory. Every
    // `lntrn-*` app's SVG lives here and this is also where the user
    // can drop overrides for any other app (matched by Icon= name).
    // Searched first so user overrides shadow system theme files.
    dirs.push(format!("{home}/.lantern/icons"));
    // Folder icons — needed by the Files view's quick-locations
    // sidebar (lntrn-folder-downloads.svg etc.) live under subdirs.
    dirs.push(format!("{home}/.lantern/icons/folders/Standard"));
    dirs.push(format!("{home}/.lantern/icons/folders/Colors"));
    dirs.push(format!("{home}/.lantern/icons/folders/Awesome"));

    // User-local freedesktop themes.
    dirs.push(format!("{home}/.local/share/icons/Tela/scalable/apps"));
    // User-local hicolor — scan every sized subdir, not just scalable.
    // Steam writes game icons here as PNGs (`steam_icon_<appid>.png`) at
    // sizes like 256/48/32/16 and NEVER in scalable, so without the sized
    // dirs every Steam game resolves to a blank icon. Largest-first so we
    // pick up the sharpest available size (coverage varies per game — some
    // only ship a 32x32, so we have to go all the way down to 16x16).
    for size in [
        "scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "24x24",
        "16x16",
    ] {
        dirs.push(format!("{home}/.local/share/icons/hicolor/{size}/apps"));
    }
    dirs.push(format!("{home}/.icons"));

    // Flatpak icon exports — system-wide and per-user. Flatpak apps
    // (Steam, Discord, etc.) ship icons here, not in /usr/share/icons.
    // Without these dirs, the Icon= name from a Flatpak .desktop file
    // (e.g. `com.valvesoftware.Steam`) resolves to nothing.
    for base in [
        "/var/lib/flatpak/exports/share/icons".to_string(),
        format!("{home}/.local/share/flatpak/exports/share/icons"),
    ] {
        for size in [
            "scalable", "512x512", "256x256", "128x128", "64x64", "48x48",
        ] {
            dirs.push(format!("{base}/hicolor/{size}/apps"));
        }
    }

    // System Tela (preferred theme).
    dirs.push("/usr/share/icons/Tela/scalable/apps".into());
    dirs.push("/usr/share/icons/Tela/256/apps".into());
    dirs.push("/usr/share/icons/Tela/128/apps".into());
    dirs.push("/usr/share/icons/Tela/64/apps".into());
    dirs.push("/usr/share/icons/Tela/48/apps".into());

    // hicolor (the freedesktop default fallback theme).
    dirs.push("/usr/share/icons/hicolor/scalable/apps".into());
    dirs.push("/usr/share/icons/hicolor/256x256/apps".into());
    dirs.push("/usr/share/icons/hicolor/128x128/apps".into());
    dirs.push("/usr/share/icons/hicolor/64x64/apps".into());
    dirs.push("/usr/share/icons/hicolor/48x48/apps".into());

    // Other common themes.
    dirs.push("/usr/share/icons/Adwaita/scalable/apps".into());
    dirs.push("/usr/share/icons/breeze/apps/48".into());

    // places + mimetypes for pinned folders / files (`folder`,
    // `text-x-generic`, `image-x-generic`, `video-x-generic`).
    dirs.push("/usr/share/icons/Adwaita/scalable/places".into());
    dirs.push("/usr/share/icons/Adwaita/scalable/mimetypes".into());
    dirs.push("/usr/share/icons/hicolor/scalable/places".into());
    dirs.push("/usr/share/icons/hicolor/scalable/mimetypes".into());
    dirs.push("/usr/share/icons/Tela/scalable/places".into());
    dirs.push("/usr/share/icons/Tela/scalable/mimetypes".into());

    // Catch-all.
    dirs.push("/usr/share/pixmaps".into());

    dirs
}

// ── Rasterization ───────────────────────────────────────────────────────────

/// Decode `path` to a `size × size` straight-alpha RGBA buffer. Pure CPU
/// — runs on the pool threads; the GPU upload happens in `pump()`.
fn rasterize(path: &Path, size: u32) -> Option<Vec<u8>> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    let data = std::fs::read(path).ok()?;
    match ext.as_deref() {
        Some("svg") | Some("svgz") => rasterize_svg(&data, size, size),
        Some("png") => rasterize_png(&data, size, size),
        Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("bmp") | Some("tif")
        | Some("tiff") | Some("ico") => rasterize_image_crate(&data, size, size),
        _ => None,
    }
}

/// Decode any image format the `image` crate handles and resize to
/// `w × h` with letterboxing so the original aspect ratio is preserved.
fn rasterize_image_crate(data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let (sw, sh) = rgba.dimensions();
    if sw == 0 || sh == 0 {
        return None;
    }
    // Aspect-preserving fit.
    let sx = w as f32 / sw as f32;
    let sy = h as f32 / sh as f32;
    let s = sx.min(sy);
    let rw = (sw as f32 * s).round().max(1.0) as u32;
    let rh = (sh as f32 * s).round().max(1.0) as u32;
    let resized = image::imageops::resize(&rgba, rw, rh, image::imageops::FilterType::Triangle);
    let mut out = vec![0u8; (w * h * 4) as usize];
    let off_x = (w - rw) / 2;
    let off_y = (h - rh) / 2;
    for y in 0..rh {
        for x in 0..rw {
            let src_i = ((y * rw + x) * 4) as usize;
            let dst_i = (((y + off_y) * w + x + off_x) * 4) as usize;
            if dst_i + 3 < out.len() && src_i + 3 < resized.as_raw().len() {
                out[dst_i..dst_i + 4].copy_from_slice(&resized.as_raw()[src_i..src_i + 4]);
            }
        }
    }
    Some(out)
}

/// Rasterize SVG/SVGZ to RGBA at the requested size, centered with
/// aspect ratio preserved. Converts premultiplied → straight alpha
/// (matching the rest of Lantern's pipeline).
fn rasterize_svg(data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opts).ok()?;

    let tree_size = tree.size();
    let sx = w as f32 / tree_size.width();
    let sy = h as f32 / tree_size.height();
    let scale = sx.min(sy);

    let rendered_w = tree_size.width() * scale;
    let rendered_h = tree_size.height() * scale;
    let off_x = (w as f32 - rendered_w) / 2.0;
    let off_y = (h as f32 - rendered_h) / 2.0;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform =
        resvg::tiny_skia::Transform::from_translate(off_x, off_y).post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut rgba = pixmap.take();
    premul_to_straight(&mut rgba);
    Some(rgba)
}

/// Decode PNG and resize to `w × h`.
fn rasterize_png(data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());

    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for ch in buf.chunks_exact(3) {
                out.extend_from_slice(ch);
                out.push(255);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for ch in buf.chunks_exact(2) {
                let g = ch[0];
                out.extend_from_slice(&[g, g, g, ch[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &g in &buf {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::Indexed => return None, // rare for app icons
    };

    let src_w = info.width;
    let src_h = info.height;
    if src_w == w && src_h == h {
        return Some(rgba);
    }
    // Bilinear (Triangle) resize instead of nearest-neighbor so the small
    // PNGs many Steam games ship (often only a 32x32) don't come out blocky
    // when scaled up to the panel's icon size. Icons and our target are both
    // square, so a straight stretch preserves the aspect ratio.
    let src = image::RgbaImage::from_raw(src_w, src_h, rgba)?;
    let resized = image::imageops::resize(&src, w, h, image::imageops::FilterType::Triangle);
    Some(resized.into_raw())
}

/// Convert in-place from premultiplied RGBA to straight alpha.
fn premul_to_straight(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as f32 / 255.0;
        if a > 0.0 {
            pixel[0] = (pixel[0] as f32 / a).min(255.0) as u8;
            pixel[1] = (pixel[1] as f32 / a).min(255.0) as u8;
            pixel[2] = (pixel[2] as f32 / a).min(255.0) as u8;
        }
    }
}

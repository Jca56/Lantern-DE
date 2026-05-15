//! Icon resolver and GPU cache.
//!
//! Given a `.desktop` `Icon=` value (theme name or absolute path), find
//! a matching SVG/PNG on disk, rasterize it once at the panel's icon
//! size, upload to a `GpuTexture`, and cache by app_id so we never pay
//! the rasterization cost twice.
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

/// LRU-ish cache: app_id → loaded GPU texture.
/// (We don't bound the cache size for now — typical user has <500 apps,
///  each ~16KB at 64×64 RGBA. ~8MB ceiling.)
pub struct IconCache {
    map: HashMap<String, Option<GpuTexture>>,
    /// Phys-pixel icon size for all entries in this cache. Cache is
    /// invalidated if scale changes (we re-rasterize at the new size).
    icon_size: u32,
}

impl IconCache {
    pub fn new(icon_size: u32) -> Self {
        Self {
            map: HashMap::new(),
            icon_size,
        }
    }

    /// Ensure the icon for `app_id` is loaded into the cache. Returns
    /// `true` if a (re)load happened; `false` if the slot was already
    /// populated. Callers usually don't care about the return value —
    /// they invoke `peek` afterward to grab the texture reference.
    pub fn ensure_loaded(
        &mut self,
        gpu: &GpuContext,
        tex_pass: &TexturePass,
        app_id: &str,
        icon_name: Option<&str>,
    ) -> bool {
        if self.map.contains_key(app_id) {
            return false;
        }
        let tex = icon_name
            .and_then(|n| resolve_path(n))
            .and_then(|p| rasterize(gpu, tex_pass, &p, self.icon_size));
        self.map.insert(app_id.to_string(), tex);
        true
    }

    /// Read-only lookup. Caller is responsible for having called
    /// `ensure_loaded` first if the entry might not exist yet.
    pub fn peek(&self, app_id: &str) -> Option<&GpuTexture> {
        self.map.get(app_id).and_then(|opt| opt.as_ref())
    }

    /// Drop everything and reload at a new size. Called when the
    /// fractional scale of the output changes.
    #[allow(dead_code)]
    pub fn resize(&mut self, new_icon_size: u32) {
        if new_icon_size != self.icon_size {
            self.map.clear();
            self.icon_size = new_icon_size;
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
    dirs.push(format!("{home}/.local/share/icons/hicolor/scalable/apps"));
    dirs.push(format!("{home}/.icons"));

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

fn rasterize(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
    size: u32,
) -> Option<GpuTexture> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    let data = std::fs::read(path).ok()?;
    let rgba = match ext.as_deref() {
        Some("svg") | Some("svgz") => rasterize_svg(&data, size, size)?,
        Some("png") => rasterize_png(&data, size, size)?,
        Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("bmp")
        | Some("tif") | Some("tiff") | Some("ico") => rasterize_image_crate(&data, size, size)?,
        _ => return None,
    };

    Some(tex_pass.upload(gpu, &rgba, size, size))
}

/// Disk cache location for a video thumbnail. Keyed by hash of the
/// absolute path so two files at the same name don't collide. Public
/// so the Files view can probe existence before pushing an IconRequest.
pub fn video_thumb_path(src: &Path) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut dir = std::path::PathBuf::from(home);
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

/// Kick off a background ffmpeg job to extract `src`'s thumbnail to
/// the on-disk cache. Idempotent — a second call for the same path
/// while a previous job is still running is a no-op. Returns
/// immediately; the cache file appears whenever ffmpeg finishes.
pub fn ensure_video_thumb_async(src: std::path::PathBuf, size: u32) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static PENDING: OnceLock<Mutex<HashSet<std::path::PathBuf>>> = OnceLock::new();
    let pending = PENDING.get_or_init(|| Mutex::new(HashSet::new()));
    let dst = video_thumb_path(&src);
    if dst.exists() {
        return;
    }
    {
        let mut g = pending.lock().unwrap();
        if g.contains(&src) {
            return;
        }
        g.insert(src.clone());
    }
    std::thread::spawn(move || {
        let _ = extract_video_thumb(&src, &dst, size);
        if let Ok(mut g) = pending.lock() {
            g.remove(&src);
        }
    });
}

/// Shell out to ffmpeg to extract a thumbnail frame at ~1s in. PNG so
/// our existing decoder can pick it up. Returns whether the file now
/// exists.
fn extract_video_thumb(src: &Path, dst: &Path, size: u32) -> bool {
    use std::process::{Command, Stdio};
    let scale = format!("scale={}:-1:force_original_aspect_ratio=decrease", size);
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-ss", "1",
            "-i",
        ])
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
    let resized = image::imageops::resize(
        &rgba,
        rw,
        rh,
        image::imageops::FilterType::Triangle,
    );
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
    let transform = resvg::tiny_skia::Transform::from_translate(off_x, off_y)
        .post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut rgba = pixmap.take();
    premul_to_straight(&mut rgba);
    Some(rgba)
}

/// Decode PNG and nearest-neighbor resize to `w × h`.
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
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let sx = (x as f32 * src_w as f32 / w as f32) as u32;
            let sy = (y as f32 * src_h as f32 / h as f32) as u32;
            let si = ((sy * src_w + sx) * 4) as usize;
            let di = ((y * w + x) * 4) as usize;
            if si + 3 < rgba.len() {
                out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
            }
        }
    }
    Some(out)
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

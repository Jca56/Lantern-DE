use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lntrn_render::{Color, GpuContext, GpuTexture, Painter, TexturePass};

use crate::fs::FileEntry;
use crate::thumbs::{ThumbKind, ThumbPool};

const ICON_RENDER_SIZE: u32 = 192;

/// Max image/video thumbnails kept on the GPU at once (~147KB each at
/// 192px RGBA → ~75MB cap). Past this, navigation purges them; the disk
/// cache makes regeneration cheap.
const THUMB_RAM_CAP: usize = 512;

fn icon_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/icons/folders")
}

// ── Icon cache ───────────────────────────────────────────────────────────────

pub struct IconCache {
    cache: HashMap<String, GpuTexture>,
    cached_dir: PathBuf,
    /// Background workers generating image/video thumbnails.
    pool: ThumbPool,
    /// Keys queued or in flight on the pool.
    pending: HashSet<String>,
    /// Keys whose generation failed — never retried this visit, so a corrupt
    /// or oversized file costs one attempt instead of one per frame.
    failed: HashSet<String>,
}

impl IconCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            cached_dir: PathBuf::new(),
            pool: ThumbPool::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
        }
    }

    /// Called on navigation. Thumbnails stay cached across directory changes
    /// (instant back-nav) until the RAM cap is hit; queued-but-unstarted jobs
    /// for the old directory are dropped so the new one renders first.
    pub fn ensure_dir(&mut self, dir: &Path) {
        if self.cached_dir != dir {
            let thumb_count = self
                .cache
                .keys()
                .filter(|k| k.starts_with("thumb:"))
                .count();
            if thumb_count > THUMB_RAM_CAP {
                self.cache.retain(|k, _| !k.starts_with("thumb:"));
            }
            for key in self.pool.clear_queue() {
                self.pending.remove(&key);
            }
            self.failed.clear();
            self.cached_dir = dir.to_path_buf();
        }
    }

    /// Drain completed thumbnails from the worker pool and upload to GPU.
    /// Call once per frame before rendering.
    pub fn poll_thumbs(&mut self, gpu: &GpuContext, tex: &TexturePass) {
        while let Some(result) = self.pool.try_recv() {
            self.pending.remove(&result.key);
            match result.rgba {
                Some((rgba, w, h)) => {
                    let texture = tex.upload(gpu, &rgba, w, h);
                    self.cache.insert(result.key, texture);
                }
                None => {
                    self.failed.insert(result.key);
                }
            }
        }
    }

    /// True while thumbnail jobs are queued or in flight — the event loop
    /// polls faster so finished thumbs appear promptly.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Check if an icon texture is already cached for this entry.
    pub fn has_icon(&self, entry: &FileEntry) -> bool {
        self.cache.contains_key(&cache_key(entry))
    }

    /// Drop any cached icon entries that involve this path. Called after
    /// the user changes a folder's icon so the next render reads the new
    /// xattr instead of serving the stale texture.
    pub fn invalidate(&mut self, path: &Path) {
        let needle = path.to_string_lossy().to_string();
        self.cache.retain(|k, _| !k.contains(&needle));
        self.failed.retain(|k| !k.contains(&needle));
    }

    /// Read-only access to a cached icon texture.
    pub fn get(&self, entry: &FileEntry) -> Option<&GpuTexture> {
        self.cache.get(&cache_key(entry))
    }

    /// Get cached icon or start loading it. Folder icons (small embedded
    /// SVGs) load synchronously; image and video thumbnails are queued on
    /// the worker pool and return None until ready.
    pub fn get_or_load(
        &mut self,
        entry: &FileEntry,
        gpu: &GpuContext,
        tex: &TexturePass,
    ) -> Option<&GpuTexture> {
        let key = cache_key(entry);
        if !self.cache.contains_key(&key)
            && !self.pending.contains(&key)
            && !self.failed.contains(&key)
        {
            if entry.is_dir {
                match load_folder_icon(entry, gpu, tex) {
                    Some(texture) => {
                        self.cache.insert(key.clone(), texture);
                    }
                    None => {
                        self.failed.insert(key.clone());
                    }
                }
            } else if let Some(kind) = thumb_kind(&entry.name) {
                self.pending.insert(key.clone());
                self.pool.submit(key.clone(), entry.path.clone(), kind);
            }
            // Other file types: procedural fallback icon, no texture.
        }
        self.cache.get(&key)
    }
    /// Read-only access to a cached folder color texture.
    pub fn get_folder_color(&self, color: &str) -> Option<&GpuTexture> {
        let key = format!("folder_color:{color}");
        self.cache.get(&key)
    }

    /// Eagerly load an SVG icon (no-op if cached). Used by the Properties
    /// icon picker's two-pass render: load everything mutably, then borrow
    /// each texture immutably to build draw calls without borrow conflicts.
    pub fn ensure_svg_path(&mut self, svg_path: &Path, gpu: &GpuContext, tex: &TexturePass) {
        let key = format!("svg:{}", svg_path.display());
        if !self.cache.contains_key(&key) {
            if let Some(t) = rasterize_svg(svg_path, gpu, tex) {
                self.cache.insert(key.clone(), t);
            }
        }
    }

    /// Immutable lookup matching `ensure_svg_path`.
    pub fn get_svg_path(&self, svg_path: &Path) -> Option<&GpuTexture> {
        let key = format!("svg:{}", svg_path.display());
        self.cache.get(&key)
    }

    /// Get or load a colored folder icon by color name (e.g. "red", "blue", "").
    /// Empty string means the plain/default folder.
    pub fn get_or_load_folder_color(
        &mut self,
        color: &str,
        gpu: &GpuContext,
        tex: &TexturePass,
    ) -> Option<&GpuTexture> {
        let key = format!("folder_color:{color}");
        if !self.cache.contains_key(&key) {
            let icon_name = if color.is_empty() {
                "folders/Colors/lntrn-folder-yellow.svg".to_string()
            } else {
                format!("folders/Colors/lntrn-folder-{color}.svg")
            };
            let texture = if let Some(data) = lntrn_icons::get(&icon_name) {
                rasterize_svg_bytes(data, gpu, tex)
            } else {
                let base = icon_dir();
                let svg_path = base.join("Colors").join(format!(
                    "lntrn-folder-{}.svg",
                    if color.is_empty() { "yellow" } else { color }
                ));
                rasterize_svg(&svg_path, gpu, tex)
            };
            if let Some(t) = texture {
                self.cache.insert(key.clone(), t);
            }
        }
        self.cache.get(&key)
    }
}

// ── Cache key logic ──────────────────────────────────────────────────────────

fn cache_key(entry: &FileEntry) -> String {
    if entry.is_dir {
        // Include xattr icon/color in cache key so custom folders get unique textures
        let icon = get_folder_icon(&entry.path).unwrap_or_default();
        let color = get_folder_color(&entry.path).unwrap_or_default();
        format!("dir:{}:{}:{}", entry.name.to_lowercase(), icon, color)
    } else if thumb_kind(&entry.name).is_some() {
        // Per-file thumbnail, keyed on size + mtime (already stat'd by the
        // listing — no syscall here) so a file that changed on disk gets a
        // fresh texture and a mid-download decode failure retries once the
        // file finishes instead of sticking in the failed set.
        let stamp = entry
            .modified
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("thumb:{}:{}:{}", entry.path.display(), entry.size, stamp)
    } else {
        // File type icons are shared by extension
        let ext = entry
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_lowercase();
        format!("type:{ext}")
    }
}

// ── Loading ──────────────────────────────────────────────────────────────────

/// Try to get embedded folder icon bytes. Returns None if the icon
/// requires disk access (xattr custom icon path).
fn folder_icon_embedded(entry: &FileEntry) -> Option<&'static [u8]> {
    // xattr custom icon path? Must go to disk.
    if get_folder_icon(&entry.path).is_some() {
        return None;
    }
    // xattr custom color? Try embedded.
    if let Some(color) = get_folder_color(&entry.path) {
        return lntrn_icons::get(&format!("folders/Colors/lntrn-folder-{color}.svg"));
    }
    // Standard named folders
    let icon_name = match entry.name.to_lowercase().as_str() {
        "desktop" => "folders/Standard/lntrn-folder-desktop.svg",
        "documents" => "folders/Standard/lntrn-folder-documents.svg",
        "downloads" => "folders/Standard/lntrn-folder-downloads.svg",
        "music" => "folders/Standard/lntrn-folder-music.svg",
        "pictures" => "folders/Standard/lntrn-folder-pictures.svg",
        "projects" => "folders/Standard/lntrn-folder-projects.svg",
        "videos" => "folders/Standard/lntrn-folder-videos.svg",
        _ => "folders/Colors/lntrn-folder-yellow.svg",
    };
    lntrn_icons::get(icon_name)
}

fn load_folder_icon(entry: &FileEntry, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    // Try embedded first
    if let Some(data) = folder_icon_embedded(entry) {
        return rasterize_svg_bytes(data, gpu, tex);
    }
    // Fall back to disk (xattr custom icons)
    let icon_path = folder_icon_path(entry);
    if is_svg_file(&icon_path) {
        rasterize_svg(&icon_path, gpu, tex)
    } else {
        // Custom image icon — one-off, goes through the shared thumbnail
        // generator (disk cache + decode limits) synchronously.
        let (rgba, w, h) = crate::thumbs::generate(&icon_path, ThumbKind::Image)?;
        Some(tex.upload(gpu, &rgba, w, h))
    }
}

fn folder_icon_path(entry: &FileEntry) -> PathBuf {
    let base = icon_dir();

    // Check xattr for custom icon path first (any image/SVG)
    if let Some(icon_path) = get_folder_icon(&entry.path) {
        let p = PathBuf::from(&icon_path);
        if p.exists() {
            return p;
        }
    }

    // Check xattr for custom color
    if let Some(color) = get_folder_color(&entry.path) {
        let color_svg = format!("lntrn-folder-{color}.svg");
        let color_path = base.join("Colors").join(&color_svg);
        if color_path.exists() {
            return color_path;
        }
    }

    // Special folder icons by name
    let svg_name = match entry.name.to_lowercase().as_str() {
        "desktop" => "lntrn-folder-desktop.svg",
        "documents" => "lntrn-folder-documents.svg",
        "downloads" => "lntrn-folder-downloads.svg",
        "music" => "lntrn-folder-music.svg",
        "pictures" => "lntrn-folder-pictures.svg",
        "projects" => "lntrn-folder-projects.svg",
        "videos" => "lntrn-folder-videos.svg",
        _ => return base.join("Colors").join("lntrn-folder-yellow.svg"),
    };
    base.join("Standard").join(svg_name)
}

const XATTR_FOLDER_COLOR: &str = "user.lantern.folder_color";
const XATTR_FOLDER_ICON: &str = "user.lantern.folder_icon";

/// Read a custom icon path xattr from a directory.
pub fn get_folder_icon(path: &Path) -> Option<String> {
    read_xattr(path, XATTR_FOLDER_ICON)
}

/// Set a custom icon path xattr on a directory.
pub fn set_folder_icon(path: &Path, icon_path: &str) {
    write_xattr(path, XATTR_FOLDER_ICON, icon_path);
}

/// Remove a custom folder icon xattr — reverts to the default folder icon.
pub fn clear_folder_icon(path: &Path) {
    remove_xattr(path, XATTR_FOLDER_ICON);
}

/// Read the folder color xattr from a directory path.
pub fn get_folder_color(path: &Path) -> Option<String> {
    read_xattr(path, XATTR_FOLDER_COLOR)
}

/// Set a folder color xattr on a directory path.
pub fn set_folder_color(path: &Path, color: &str) {
    write_xattr(path, XATTR_FOLDER_COLOR, color);
}

fn read_xattr(path: &Path, attr: &str) -> Option<String> {
    use std::ffi::CString;
    let c_path = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let c_name = CString::new(attr).ok()?;
    let mut buf = [0u8; 512];
    let len = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
        )
    };
    if len > 0 {
        Some(String::from_utf8_lossy(&buf[..len as usize]).to_string())
    } else {
        None
    }
}

fn write_xattr(path: &Path, attr: &str, value: &str) {
    use std::ffi::CString;
    let Some(c_path) = CString::new(path.as_os_str().as_encoded_bytes()).ok() else {
        return;
    };
    let Some(c_name) = CString::new(attr).ok() else {
        return;
    };
    unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr() as *const libc::c_void,
            value.len(),
            0,
        );
    }
}

fn remove_xattr(path: &Path, attr: &str) {
    use std::ffi::CString;
    let Some(c_path) = CString::new(path.as_os_str().as_encoded_bytes()).ok() else {
        return;
    };
    let Some(c_name) = CString::new(attr).ok() else {
        return;
    };
    unsafe {
        libc::removexattr(c_path.as_ptr(), c_name.as_ptr());
    }
}

// ── SVG rasterization ────────────────────────────────────────────────────────

fn rasterize_svg_bytes(data: &[u8], gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;

    let svg_size = tree.size();
    let scale = (ICON_RENDER_SIZE as f32 / svg_size.width())
        .min(ICON_RENDER_SIZE as f32 / svg_size.height());
    let w = (svg_size.width() * scale).ceil() as u32;
    let h = (svg_size.height() * scale).ceil() as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Some(tex.upload(gpu, pixmap.data(), w, h))
}

fn rasterize_svg(path: &Path, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let data = std::fs::read(path).ok()?;
    rasterize_svg_bytes(&data, gpu, tex)
}

// ── File type detection ──────────────────────────────────────────────────────

fn is_image_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tiff" | "tif"
    )
}

/// Raster formats the compositor's wallpaper loader can decode — gates the
/// "Set as Wallpaper" context-menu item (no SVG: the compositor decodes
/// wallpapers with the `image` crate).
pub fn is_raster_image_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif"
    )
}

/// WAV / MP3 — anything `audio_tags` can pull cover art out of.
pub fn is_audio_file(name: &str) -> bool {
    crate::audio_tags::container_for(Path::new(name)).is_some()
}

/// Which thumbnail job a file name maps to, if any.
fn thumb_kind(name: &str) -> Option<ThumbKind> {
    if is_video_file(name) {
        Some(ThumbKind::Video)
    } else if is_audio_file(name) {
        Some(ThumbKind::Audio)
    } else if is_image_file(name) {
        Some(ThumbKind::Image)
    } else {
        None
    }
}

/// Eighth-note glyph, `u` = unit size (glyph spans roughly 10u × 12u).
/// Shared by the sidebar "Music" place and tag-less audio file icons.
pub fn draw_note_glyph(painter: &mut Painter, cx: f32, cy: f32, u: f32, stroke: f32, color: Color) {
    painter.line(cx - 2.0 * u, cy - 6.0 * u, cx - 2.0 * u, cy + 4.0 * u, stroke, color);
    painter.circle_filled(cx - 4.0 * u, cy + 5.0 * u, 3.0 * u, color);
    painter.line(cx - 2.0 * u, cy - 6.0 * u, cx + 4.0 * u, cy - 4.0 * u, stroke, color);
    painter.line(cx + 4.0 * u, cy - 4.0 * u, cx + 4.0 * u, cy + 1.0 * u, stroke, color);
    painter.circle_filled(cx + 2.0 * u, cy + 2.0 * u, 2.5 * u, color);
}

pub fn is_video_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "mp4" | "m4v" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv"
    )
}

fn is_svg_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or(false, |e| e.eq_ignore_ascii_case("svg"))
}

// ── Texture draw helpers ─────────────────────────────────────────────────────

/// Compute draw rect that fits the texture in a bounding box, maintaining aspect ratio.
pub fn fit_in_box(
    tex: &GpuTexture,
    box_x: f32,
    box_y: f32,
    box_w: f32,
    box_h: f32,
) -> (f32, f32, f32, f32) {
    let tw = tex.width as f32;
    let th = tex.height as f32;
    let scale = (box_w / tw).min(box_h / th);
    let w = tw * scale;
    let h = th * scale;
    let x = box_x + (box_w - w) * 0.5;
    let y = box_y + (box_h - h) * 0.5;
    (x, y, w, h)
}

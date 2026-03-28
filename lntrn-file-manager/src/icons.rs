use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::fs::FileEntry;

const ICON_RENDER_SIZE: u32 = 192;

fn icon_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/icons/folders")
}

/// RGBA image data produced by a background ffmpeg thread.
struct VideoThumbResult {
    key: String,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

// ── Icon cache ───────────────────────────────────────────────────────────────

pub struct IconCache {
    cache: HashMap<String, GpuTexture>,
    cached_dir: PathBuf,
    /// Keys currently being generated in background threads.
    pending_videos: HashSet<String>,
    /// Receiver for completed video thumbnails.
    video_rx: mpsc::Receiver<VideoThumbResult>,
    /// Sender cloned into each background thread.
    video_tx: mpsc::Sender<VideoThumbResult>,
}

impl IconCache {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            cached_dir: PathBuf::new(),
            pending_videos: HashSet::new(),
            video_rx: rx,
            video_tx: tx,
        }
    }

    /// Clear thumbnail cache when navigating to a new directory.
    /// Keeps folder/type icons since they're reusable.
    pub fn ensure_dir(&mut self, dir: &Path) {
        if self.cached_dir != dir {
            // Only clear thumbnails (path-specific), keep type/folder icons
            self.cache.retain(|k, _| !k.starts_with("thumb:"));
            self.pending_videos.clear();
            self.cached_dir = dir.to_path_buf();
        }
    }

    /// Drain completed video thumbnails from background threads and upload to GPU.
    /// Call once per frame before rendering.
    pub fn poll_video_thumbs(&mut self, gpu: &GpuContext, tex: &TexturePass) {
        while let Ok(result) = self.video_rx.try_recv() {
            self.pending_videos.remove(&result.key);
            let texture = tex.upload(gpu, &result.rgba, result.width, result.height);
            self.cache.insert(result.key, texture);
        }
    }

    /// Check if an icon texture is already cached for this entry.
    pub fn has_icon(&self, entry: &FileEntry) -> bool {
        self.cache.contains_key(&cache_key(entry))
    }

    /// Read-only access to a cached icon texture.
    pub fn get(&self, entry: &FileEntry) -> Option<&GpuTexture> {
        self.cache.get(&cache_key(entry))
    }

    /// Get cached icon or load it. Returns None if loading fails or is still pending.
    pub fn get_or_load(
        &mut self,
        entry: &FileEntry,
        gpu: &GpuContext,
        tex: &TexturePass,
    ) -> Option<&GpuTexture> {
        let key = cache_key(entry);
        if !self.cache.contains_key(&key) && !self.pending_videos.contains(&key) {
            if is_video_file(&entry.name) {
                // Spawn background thread for video thumbnails
                self.pending_videos.insert(key.clone());
                let path = entry.path.clone();
                let tx = self.video_tx.clone();
                let k = key.clone();
                std::thread::spawn(move || {
                    if let Some((rgba, w, h)) = extract_video_frame(&path) {
                        let _ = tx.send(VideoThumbResult { key: k, rgba, width: w, height: h });
                    }
                });
            } else if let Some(texture) = load_icon(entry, gpu, tex) {
                self.cache.insert(key.clone(), texture);
            }
        }
        self.cache.get(&key)
    }
    /// Read-only access to a cached folder color texture.
    pub fn get_folder_color(&self, color: &str) -> Option<&GpuTexture> {
        let key = format!("folder_color:{color}");
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
            let base = icon_dir();
            let svg_path = if color.is_empty() {
                base.join("Colors").join("lntrn-folder-yellow.svg")
            } else {
                base.join("Colors").join(format!("lntrn-folder-{color}.svg"))
            };
            if let Some(texture) = rasterize_svg(&svg_path, gpu, tex) {
                self.cache.insert(key.clone(), texture);
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
    } else if is_image_file(&entry.name) {
        // Each image gets its own thumbnail
        format!("thumb:{}", entry.path.display())
    } else if is_video_file(&entry.name) {
        // Each video gets its own thumbnail
        format!("thumb:{}", entry.path.display())
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

fn load_icon(entry: &FileEntry, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    if entry.is_dir {
        let icon_path = folder_icon_path(entry);
        if is_svg_file(&icon_path) {
            rasterize_svg(&icon_path, gpu, tex)
        } else {
            load_image_thumbnail(&icon_path, gpu, tex)
        }
    } else if is_image_file(&entry.name) {
        if is_svg_file(&entry.path) {
            rasterize_svg(&entry.path, gpu, tex)
        } else {
            load_image_thumbnail(&entry.path, gpu, tex)
        }
    } else {
        None // No file type icons yet — procedural fallback
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
    let Some(c_path) = CString::new(path.as_os_str().as_encoded_bytes()).ok() else { return };
    let Some(c_name) = CString::new(attr).ok() else { return };
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

// ── SVG rasterization ────────────────────────────────────────────────────────

fn rasterize_svg(path: &Path, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;

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

// ── Image thumbnails ─────────────────────────────────────────────────────────

fn load_image_thumbnail(path: &Path, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let img = image::open(path).ok()?;
    let thumb = img.thumbnail(ICON_RENDER_SIZE, ICON_RENDER_SIZE);
    let rgba = thumb.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some(tex.upload(gpu, &rgba, w, h))
}

// ── Video thumbnails ─────────────────────────────────────────────────────────

fn video_thumb_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".cache/lntrn-file-manager/video-thumbs")
}

/// Simple hash of a path string for use as a cache filename.
fn path_hash(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Extract a video frame via ffmpeg, with disk caching.
/// Runs on background thread — no GPU access.
fn extract_video_frame(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    use std::process::Command;

    let cache_dir = video_thumb_dir();
    let _ = std::fs::create_dir_all(&cache_dir);
    let cached = cache_dir.join(format!("{:016x}.png", path_hash(path)));

    // Check disk cache first
    if cached.exists() {
        if let Ok(img) = image::open(&cached) {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            return Some((rgba.into_raw(), w, h));
        }
    }

    // Extract frame with ffmpeg
    let output = Command::new("ffmpeg")
        .args(["-ss", "1", "-i"])
        .arg(path)
        .args([
            "-frames:v", "1",
            "-vf", &format!("scale={s}:{s}:force_original_aspect_ratio=decrease", s = ICON_RENDER_SIZE),
            "-f", "image2pipe",
            "-vcodec", "png",
            "-loglevel", "error",
            "pipe:1",
        ])
        .output()
        .ok()?;

    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }

    // Save to disk cache
    let _ = std::fs::write(&cached, &output.stdout);

    let img = image::load_from_memory(&output.stdout).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

// ── File type detection ──────────────────────────────────────────────────────

fn is_image_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tiff" | "tif"
    )
}

fn is_video_file(name: &str) -> bool {
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

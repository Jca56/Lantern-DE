use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lntrn_render::{Color, GpuContext, GpuTexture, Painter, TexturePass};

use crate::fs::FileEntry;
use crate::icon_store::{thumb_key, thumb_path, TextureStore};
use crate::thumbs::{ThumbKind, ThumbPool};

const ICON_RENDER_SIZE: u32 = 192;

fn icon_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/icons/folders")
}

// ── Icon cache ───────────────────────────────────────────────────────────────

pub struct IconCache {
    /// Thumbnails in it are bounded: the least recently shown ones go when
    /// new ones arrive (icon_store.rs).
    cache: TextureStore<GpuTexture>,
    /// The folders on screen: the focused pane's and, in split view, the
    /// other pane's.
    shown_dirs: Vec<PathBuf>,
    /// Background workers generating image/video thumbnails, and the keys
    /// queued or in flight on them.
    pool: ThumbPool,
    /// Keys whose generation failed — never retried this visit, so a corrupt
    /// or oversized file costs one attempt instead of one per frame.
    failed: HashSet<String>,
}

impl IconCache {
    pub fn new() -> Self {
        Self {
            cache: TextureStore::new(),
            shown_dirs: Vec::new(),
            pool: ThumbPool::new(),
            failed: HashSet::new(),
        }
    }

    /// Called at the start of every frame with the folders it shows.
    /// Thumbnails stay cached across directory changes (instant back-nav),
    /// within the store's bound. When the folders change, the failures
    /// recorded outside them are forgotten, so a revisit tries those files
    /// again. A split view swapping focus between its panes shows the same
    /// two folders as before: that is not a navigation.
    pub fn begin_frame(&mut self, shown: [Option<&Path>; 2]) {
        self.cache.begin_frame();
        let same = shown
            .iter()
            .flatten()
            .all(|dir| self.shown_dirs.iter().any(|s| s == dir))
            && self
                .shown_dirs
                .iter()
                .all(|s| shown.iter().flatten().any(|dir| s == dir));
        if same {
            return;
        }
        self.shown_dirs.clear();
        for dir in shown.into_iter().flatten() {
            if !self.shown_dirs.iter().any(|s| s == dir) {
                self.shown_dirs.push(dir.to_path_buf());
            }
        }
        let shown_dirs = &self.shown_dirs;
        self.failed.retain(|key| {
            thumb_path(key).is_some_and(|p| shown_dirs.iter().any(|d| Path::new(p).starts_with(d)))
        });
    }

    /// Every view of the frame has asked for the icons it shows: the
    /// thumbnail queue is cut down to those, top row first.
    pub fn end_requests(&mut self) {
        self.pool.end_frame();
    }

    /// Drain completed thumbnails from the worker pool and upload to GPU.
    /// Call once per frame before rendering.
    pub fn poll_thumbs(&mut self, gpu: &GpuContext, tex: &TexturePass) {
        while let Some(result) = self.pool.try_recv() {
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
        self.pool.has_pending()
    }

    /// Check if an icon texture is already cached for this entry.
    pub fn has_icon(&self, entry: &FileEntry) -> bool {
        self.cache.contains(&cache_key(entry))
    }

    /// Drop any cached icon entries that involve this path. Called when
    /// the user changes a folder's icon. (The folder's own texture is keyed
    /// by the icon and colour its entry carries, so it changes when the
    /// entry does: `App::apply_folder_icon`.)
    pub fn invalidate(&mut self, path: &Path) {
        let needle = path.to_string_lossy().to_string();
        self.cache.retain(|k| !k.contains(&needle));
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
        // `want`: a thumbnail still on its way is asked for again every
        // frame its row is on screen, which keeps its job in the queue.
        if !self.cache.contains(&key) && !self.failed.contains(&key) && !self.pool.want(&key) {
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
                if crate::fs::is_slow_path(&entry.path) {
                    // On MTP the first read downloads the entire file. A
                    // photo is a few MB — fine one at a time on the slow
                    // lane. A video frame or an audio tag costs the same
                    // full download for a file that may be gigabytes, so
                    // those keep the generic icon.
                    match kind {
                        ThumbKind::Image => {
                            self.pool
                                .submit(key.clone(), entry.path.clone(), kind, true);
                        }
                        ThumbKind::Video | ThumbKind::Audio => {
                            self.failed.insert(key.clone());
                        }
                    }
                } else {
                    self.pool
                        .submit(key.clone(), entry.path.clone(), kind, false);
                }
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
    /// True when the texture was made just now: the frame that asked for
    /// it was drawn without it.
    pub fn ensure_svg_path(
        &mut self,
        svg_path: &Path,
        gpu: &GpuContext,
        tex: &TexturePass,
    ) -> bool {
        let key = format!("svg:{}", svg_path.display());
        if !self.cache.contains(&key) && !self.failed.contains(&key) {
            match rasterize_svg(svg_path, gpu, tex) {
                Some(t) => {
                    self.cache.insert(key, t);
                    return true;
                }
                // Remember the failure: this is called every frame while
                // the picker is open, and a broken SVG was re-read and
                // re-parsed each time.
                None => {
                    self.failed.insert(key);
                }
            }
        }
        false
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
        if !self.cache.contains(&key) {
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
        // Custom icon and colour are part of the key so such folders get a
        // texture of their own. They come with the entry (read when its
        // listing was built): this runs three times per visible entry per
        // frame and must not touch the filesystem.
        let icon = entry.folder_icon.as_deref().unwrap_or_default();
        let color = entry.folder_color.as_deref().unwrap_or_default();
        // The name only matters for the seven standard folders. Keying on
        // every folder's own name rasterised the same yellow SVG once per
        // distinct name, on the render thread, into a texture never freed.
        let name = entry.name.to_lowercase();
        let standard = if STANDARD_FOLDERS.contains(&name.as_str()) {
            name.as_str()
        } else {
            ""
        };
        format!("dir:{standard}:{icon}:{color}")
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
        thumb_key(&entry.path, entry.size, stamp)
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

/// Folder names that have an icon of their own (see `folder_icon_embedded`).
const STANDARD_FOLDERS: [&str; 7] = [
    "desktop",
    "documents",
    "downloads",
    "music",
    "pictures",
    "projects",
    "videos",
];

// ── Loading ──────────────────────────────────────────────────────────────────

/// Try to get embedded folder icon bytes. Returns None if the icon
/// requires disk access (xattr custom icon path).
fn folder_icon_embedded(entry: &FileEntry) -> Option<&'static [u8]> {
    // xattr custom icon path? Must go to disk.
    if entry.folder_icon.is_some() {
        return None;
    }
    // xattr custom color? Try embedded.
    if let Some(color) = &entry.folder_color {
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

    // Check xattr for custom icon path first (any image/SVG). This runs on
    // the render thread (once per icon), so an image kept on a phone or a
    // network share is not looked at: the folder gets the stock icon.
    if let Some(icon_path) = &entry.folder_icon {
        let p = PathBuf::from(icon_path);
        if !crate::fs::is_slow_path(&p) && p.exists() {
            return p;
        }
    }

    // Check xattr for custom color
    if let Some(color) = &entry.folder_color {
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

/// A folder's custom icon path and colour, straight from its attributes.
/// Two syscalls, each a device round-trip on a slow mount: for listing and
/// worker threads, never for drawing (entries carry the result).
pub fn read_folder_attrs(path: &Path) -> (Option<String>, Option<String>) {
    (
        read_xattr(path, XATTR_FOLDER_ICON),
        read_xattr(path, XATTR_FOLDER_COLOR),
    )
}

/// Set a custom icon path xattr on a directory.
pub fn set_folder_icon(path: &Path, icon_path: &str) {
    write_xattr(path, XATTR_FOLDER_ICON, icon_path);
}

/// Remove a custom folder icon xattr — reverts to the default folder icon.
pub fn clear_folder_icon(path: &Path) {
    remove_xattr(path, XATTR_FOLDER_ICON);
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

    // Straight alpha for the texture shader (tiny-skia output is
    // premultiplied; uploading it as-is darkened every soft edge).
    Some(tex.upload(gpu, &pixmap.take_demultiplied(), w, h))
}

fn rasterize_svg(path: &Path, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let data = crate::thumbs::read_svg_capped(path)?;
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

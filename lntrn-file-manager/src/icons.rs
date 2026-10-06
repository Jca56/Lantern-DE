mod folders;
mod kinds;
mod theme;
mod xattr;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lntrn_render::{Color, GpuContext, GpuTexture, Painter, TexturePass};

use crate::fs::FileEntry;
use crate::icon_store::{thumb_key, thumb_path, TextureStore};
use crate::thumbs::{ThumbKind, ThumbPool};

use folders::{icon_dir, is_standard_folder, load_folder_icon};
use kinds::thumb_kind;
pub use kinds::{is_audio_file, is_raster_image_file, is_video_file};
use theme::ThemeIcons;
pub use xattr::{clear_folder_icon, read_folder_attrs, set_folder_color, set_folder_icon};

const ICON_RENDER_SIZE: u32 = 192;

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
    /// The icon theme's SVG for a name (icons/theme.rs).
    theme: ThemeIcons,
}

impl IconCache {
    pub fn new() -> Self {
        Self {
            cache: TextureStore::new(),
            shown_dirs: Vec::new(),
            pool: ThumbPool::new(),
            failed: HashSet::new(),
            theme: ThemeIcons::installed(),
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
        texture_keys(&self.theme, entry).iter().flatten().any(|key| self.cache.contains(key))
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
        texture_keys(&self.theme, entry).iter().flatten().find_map(|key| self.cache.get(key))
    }

    /// A file's own thumbnail, once it is cached: `get` without the
    /// theme's icon standing in. For what belongs on a real thumbnail
    /// only, like a video's play badge.
    pub fn thumb(&self, entry: &FileEntry) -> Option<&GpuTexture> {
        if entry.is_dir || thumb_kind(&entry.name).is_none() {
            return None;
        }
        self.cache.get(&cache_key(entry))
    }

    /// Get cached icon or start loading it. Folder and theme icons (small
    /// SVGs, shared by every entry they are for) load synchronously; image
    /// and video thumbnails are queued on the worker pool, and until one
    /// is ready the theme's icon is returned in its place (None without a
    /// theme).
    pub fn get_or_load(
        &mut self,
        entry: &FileEntry,
        gpu: &GpuContext,
        tex: &TexturePass,
    ) -> Option<&GpuTexture> {
        let themed = self.load_themed(entry, gpu, tex);
        let key = cache_key(entry);
        // `want`: a thumbnail still on its way is asked for again every
        // frame its row is on screen, which keeps its job in the queue.
        if !self.cache.contains(&key) && !self.failed.contains(&key) && !self.pool.want(&key) {
            if entry.is_dir {
                // The stock folder is not drawn for a folder that has the
                // theme's icon.
                if !themed {
                    match load_folder_icon(entry, gpu, tex) {
                        Some(texture) => {
                            self.cache.insert(key.clone(), texture);
                        }
                        None => {
                            self.failed.insert(key.clone());
                        }
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
            // Other file types: the theme's icon, or without a theme the
            // procedural fallback icon, no texture.
        }
        self.get(entry)
    }

    /// Load the theme's icon for `entry`, if it has one; true once it is
    /// cached. An icon that does not draw is given up, and the entry gets
    /// what the theme has next: for a file, the plain file.
    fn load_themed(&mut self, entry: &FileEntry, gpu: &GpuContext, tex: &TexturePass) -> bool {
        for _ in 0..2 {
            let Some(svg) = self.theme.svg_for(entry) else {
                return false;
            };
            self.ensure_svg_path(&svg, gpu, tex);
            if self.cache.contains(&svg_key(&svg)) {
                return true;
            }
            self.theme.give_up(&svg);
        }
        false
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
        let key = svg_key(svg_path);
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
        self.cache.get(&svg_key(svg_path))
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

/// The keys of the textures an entry can be drawn with, the one to prefer
/// first. A file's thumbnail comes before the theme's icon for its name,
/// which stands in until the thumbnail is there (for good, when there
/// never is one); a folder the theme has an icon for takes that, and the
/// stock folder only if it could not be drawn.
fn texture_keys(theme: &ThemeIcons, entry: &FileEntry) -> [Option<String>; 2] {
    let themed = theme.svg_for(entry).map(|svg| svg_key(&svg));
    if entry.is_dir {
        [themed, Some(cache_key(entry))]
    } else if thumb_kind(&entry.name).is_some() {
        [Some(cache_key(entry)), themed]
    } else {
        [themed, None]
    }
}

/// The key of an SVG file's texture: the picker's icons and the theme's.
fn svg_key(svg_path: &Path) -> String {
    format!("svg:{}", svg_path.display())
}

/// The key of an entry's own texture: a folder's icon, a file's thumbnail.
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
        let standard = if is_standard_folder(&name) {
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
        // No texture of its own: the theme's icon for its name, if any
        // (`texture_keys`). Nothing is ever stored under this.
        "type:".to_string()
    }
}

// ── SVG rasterization ────────────────────────────────────────────────────────

fn rasterize_svg_bytes(data: &[u8], gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let (rgba, w, h) = svg_pixels(data)?;
    Some(tex.upload(gpu, &rgba, w, h))
}

/// An SVG drawn to fit `ICON_RENDER_SIZE`: straight-alpha RGBA and its size.
/// `None` for one that draws nothing at all: that is no icon, and what
/// asked for it falls back as it does for a file that cannot be read.
fn svg_pixels(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
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
    let rgba = pixmap.take_demultiplied();
    rgba.chunks_exact(4).any(|px| px[3] > 0).then_some((rgba, w, h))
}

fn rasterize_svg(path: &Path, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    let data = crate::thumbs::read_svg_capped(path)?;
    rasterize_svg_bytes(&data, gpu, tex)
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

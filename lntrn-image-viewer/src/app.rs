use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::{GpuContext, GpuTexture, Rect, TexturePass};

use crate::gif::GifPlayer;
use crate::info::ImageInfo;
use crate::loaders;
use crate::shuffle::Shuffle;

// ── Supported formats ───────────────────────────────────────────────────────

const SUPPORTED_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "tiff", "tif", "svg",
];

pub(crate) fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let lower = ext.to_ascii_lowercase();
            SUPPORTED_EXTS.iter().any(|e| *e == lower)
        })
        .unwrap_or(false)
}

pub(crate) fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
}

fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("gif"))
        .unwrap_or(false)
}

/// Absolute path with the folder canonicalized but the file itself left
/// alone, so a symlinked image stays the link: Left/Right keep walking this
/// folder, and Delete trashes the link rather than whatever it points at.
/// None when the file isn't there (a dangling link counts as missing).
fn resolve(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    let name = abs.file_name()?.to_os_string();
    let dir = abs.parent()?.canonicalize().ok()?;
    let resolved = dir.join(name);
    resolved.exists().then_some(resolved)
}

// ── Loaded image ────────────────────────────────────────────────────────────

pub struct LoadedImage {
    pub texture: GpuTexture,
    /// The picture's own size: pixels for rasters, user units for SVG.
    /// Layout and the zoom readout go by this, never by the texture, which
    /// can be smaller (past the GPU's limit) or a fresh SVG rasterization.
    pub natural_w: f32,
    pub natural_h: f32,
    /// SVG source, kept to re-rasterize crisply at the displayed size.
    pub svg_source: Option<String>,
}

// ── App state ───────────────────────────────────────────────────────────────

/// Viewer-mode modal dialogs.
pub enum ViewerDialog {
    ConfirmTrash(PathBuf),
    Error(String),
}

/// How long a status-bar flash ("Copied to clipboard") stays up.
const FLASH_TTL: Duration = Duration::from_secs(2);

pub struct App {
    pub image: Option<LoadedImage>,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub is_panning: bool,
    pub last_pan_x: f32,
    pub last_pan_y: f32,
    pub file_name: String,
    pub status_text: String,
    pub dimensions_text: String,
    /// Super+F11 "rice mode": no title bar, no status bar, just the picture.
    pub chrome_hidden: bool,
    /// Absolute path of the open file — also set when it failed to load, so
    /// Left/Right carry on from it and Delete can trash a broken file.
    pub path: Option<PathBuf>,
    /// File facts + EXIF for the info overlay.
    pub info: Option<ImageInfo>,
    /// I key: show the info overlay.
    pub show_info: bool,
    /// Some(last advance) while a slideshow is running.
    pub slideshow: Option<Instant>,
    pub slideshow_interval: Duration,
    /// Modal in front of the picture (trash confirm / error).
    pub dialog: Option<ViewerDialog>,
    /// Transient status-bar message (replaces the path briefly).
    pub flash: Option<(String, Instant)>,
    // Directory navigation
    pub dir_files: Vec<PathBuf>,
    pub dir_index: usize,
    /// The open file is in `dir_files` (a hidden file opened directly isn't),
    /// so `dir_index` really is its position.
    pub listed: bool,
    pub shuffle: bool,
    shuffler: Shuffle,
    pub gif: Option<GifPlayer>,
}

impl App {
    pub fn new() -> Self {
        Self {
            image: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            is_panning: false,
            last_pan_x: 0.0,
            last_pan_y: 0.0,
            file_name: String::new(),
            status_text: "No image loaded".into(),
            dimensions_text: String::new(),
            chrome_hidden: false,
            path: None,
            info: None,
            show_info: false,
            slideshow: None,
            slideshow_interval: Duration::from_secs(4),
            dialog: None,
            flash: None,
            dir_files: Vec::new(),
            dir_index: 0,
            listed: false,
            shuffle: false,
            shuffler: Shuffle::new(),
            gif: None,
        }
    }

    pub fn open_image(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, path: &Path) {
        let Some(abs) = resolve(path) else {
            self.fail(path.to_path_buf(), format!("File not found: {}", path.display()));
            return;
        };
        self.sync_dir_list(&abs);
        self.gif = None;
        let max_dim = loaders::max_texture_dim(gpu);

        if is_gif(&abs) {
            // Streams from a worker. A still GIF just never gets a second
            // frame; one too big for a texture comes back None and takes
            // the raster path below, which downscales.
            if let Some((player, first)) = GifPlayer::open(&abs, max_dim) {
                let tex = tex_pass.upload(gpu, &first.rgba, first.width, first.height);
                let natural = (first.width as f32, first.height as f32);
                let info = ImageInfo::gather(&abs, "GIF", None);
                self.set_loaded(abs, tex, natural, None, info);
                self.gif = Some(player);
                return;
            }
        }

        if is_svg(&abs) {
            match loaders::load_svg_texture(gpu, tex_pass, &abs) {
                Some((tex, svg)) => {
                    let info = ImageInfo::gather(&abs, "SVG", None);
                    let natural = (svg.native_w, svg.native_h);
                    self.set_loaded(abs, tex, natural, Some(svg.source), info);
                }
                None => self.fail_unreadable(abs),
            }
        } else {
            match loaders::load_raster_texture(gpu, tex_pass, &abs) {
                Some(r) => {
                    let format = crate::info::format_name(r.format);
                    let info = ImageInfo::gather(&abs, &format, r.exif.as_deref());
                    let natural = (r.width as f32, r.height as f32);
                    self.set_loaded(abs, r.tex, natural, None, info);
                }
                None => self.fail_unreadable(abs),
            }
        }
    }

    /// Rescan the sibling list when the folder changed, and find the open
    /// file in it.
    fn sync_dir_list(&mut self, abs: &Path) {
        let Some(parent) = abs.parent() else { return };
        let rescan = self.dir_files.first().and_then(|f| f.parent()) != Some(parent);
        if rescan {
            self.scan_directory(parent);
        }
        match self.dir_files.iter().position(|f| f == abs) {
            Some(i) => {
                self.dir_index = i;
                self.listed = true;
            }
            None => self.listed = false,
        }
        if self.shuffle {
            self.shuffler
                .follow(self.dir_files.len(), self.dir_index, rescan);
        }
    }

    fn set_loaded(
        &mut self,
        abs: PathBuf,
        tex: GpuTexture,
        natural: (f32, f32),
        svg_source: Option<String>,
        info: ImageInfo,
    ) {
        self.info = Some(info);
        self.file_name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_default();
        self.status_text = abs.to_string_lossy().into();
        self.path = Some(abs);
        self.dimensions_text = format!(
            "{} × {}",
            natural.0.ceil() as u32,
            natural.1.ceil() as u32
        );
        self.fit_to_view();
        self.image = Some(LoadedImage {
            texture: tex,
            natural_w: natural.0.max(1e-3),
            natural_h: natural.1.max(1e-3),
            svg_source,
        });
    }

    fn fail_unreadable(&mut self, abs: PathBuf) {
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.fail(abs, format!("Can't open {name}: unsupported or damaged file"));
    }

    /// The file couldn't be shown: drop the old picture so nothing stale
    /// stays up under the new file's name, and say why.
    fn fail(&mut self, path: PathBuf, msg: String) {
        self.image = None;
        self.gif = None;
        self.info = None;
        self.file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_default();
        self.dimensions_text.clear();
        self.status_text = msg;
        self.path = Some(path);
        self.fit_to_view();
    }

    /// Where the picture sits inside `canvas`: fitted, then zoomed and
    /// panned. Render, the zoom readout and SVG re-rasterizing all use this.
    pub fn display_rect(&self, canvas: Rect) -> Option<Rect> {
        let img = self.image.as_ref()?;
        let fit = (canvas.w / img.natural_w).min(canvas.h / img.natural_h);
        let k = fit * self.zoom;
        let (w, h) = (img.natural_w * k, img.natural_h * k);
        Some(Rect::new(
            canvas.x + (canvas.w - w) * 0.5 + self.pan_x,
            canvas.y + (canvas.h - h) * 0.5 + self.pan_y,
            w,
            h,
        ))
    }

    /// On-screen size as a percentage of the picture's real size.
    pub fn zoom_percent(&self, canvas: Rect) -> Option<u32> {
        let img = self.image.as_ref()?;
        let r = self.display_rect(canvas)?;
        Some((r.w / img.natural_w * 100.0).round() as u32)
    }

    /// Re-rasterize the loaded SVG to match its on-screen size inside
    /// `canvas`, so it stays sharp when the window/zoom grows. Never below
    /// native size, never past the GPU's limit, aspect always kept; only
    /// re-renders on a ≥25% change (hysteresis avoids churning a texture
    /// every frame during a drag-resize). No-op for raster images.
    pub fn maybe_rerender_svg(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, canvas: Rect) {
        let Some(shown) = self.display_rect(canvas) else { return };
        let Some(img) = &self.image else { return };
        let Some(source) = &img.svg_source else { return };

        let scale = (shown.w / img.natural_w).max(1.0);
        let max_dim = loaders::max_texture_dim(gpu);
        let (want_w, want_h) = loaders::svg_raster_size(img.natural_w, img.natural_h, scale, max_dim);
        let (cur_w, cur_h) = (img.texture.width as f32, img.texture.height as f32);
        let grew = want_w as f32 > cur_w * 1.25 || want_h as f32 > cur_h * 1.25;
        let shrank = (want_w as f32) < cur_w * 0.75 && (want_h as f32) < cur_h * 0.75;
        if !grew && !shrank {
            return;
        }
        let tex = loaders::rasterize_svg(
            gpu,
            tex_pass,
            source,
            img.natural_w,
            img.natural_h,
            want_w,
            want_h,
        );
        if let (Some(tex), Some(img)) = (tex, &mut self.image) {
            img.texture = tex;
        }
    }

    /// Build the Left/Right sibling list, ordered the way Fox currently lists
    /// this folder (same sort key, direction, and hidden-file rule).
    fn scan_directory(&mut self, dir: &Path) {
        let listing = crate::dir_sort::read_fox_listing();
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_supported(p))
            .filter(|p| {
                listing.show_hidden
                    || !p
                        .file_name()
                        .map(|n| n.to_string_lossy().starts_with('.'))
                        .unwrap_or(false)
            })
            .collect();
        crate::dir_sort::sort_like_fox(&mut files, listing);
        self.dir_files = files;
        self.dir_index = 0;
    }

    pub fn next_image(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) {
        let n = self.dir_files.len();
        if n == 0 {
            return;
        }
        let shuffled = self.shuffle.then(|| self.shuffler.next(n, self.dir_index)).flatten();
        self.dir_index = shuffled.unwrap_or((self.dir_index + 1) % n);
        let path = self.dir_files[self.dir_index].clone();
        self.open_image(gpu, tex_pass, &path);
    }

    pub fn prev_image(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) {
        let n = self.dir_files.len();
        if n == 0 {
            return;
        }
        let shuffled = self.shuffle.then(|| self.shuffler.prev()).flatten();
        let back = self.dir_index.min(n).checked_sub(1).unwrap_or(n - 1);
        self.dir_index = shuffled.unwrap_or(back);
        let path = self.dir_files[self.dir_index].clone();
        self.open_image(gpu, tex_pass, &path);
    }

    // ── Timers: GIF frames, slideshow, flash ────────────────────────────

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }

    /// The flash message while it's still fresh.
    pub fn flash_text(&self) -> Option<&str> {
        self.flash
            .as_ref()
            .filter(|(_, at)| at.elapsed() < FLASH_TTL)
            .map(|(m, _)| m.as_str())
    }

    pub fn toggle_slideshow(&mut self) {
        self.slideshow = match self.slideshow {
            Some(_) => None,
            None => Some(Instant::now()),
        };
    }

    /// Nudge the slideshow interval by whole seconds (1–60 s).
    pub fn adjust_slideshow(&mut self, steps: i64) {
        let secs = (self.slideshow_interval.as_secs() as i64 + steps).clamp(1, 60);
        self.slideshow_interval = Duration::from_secs(secs as u64);
        self.flash(format!("Slideshow interval: {secs}s"));
    }

    /// The next moment a timer wants the loop awake. A GIF frame that's due
    /// but not decoded yet isn't a deadline (`is_waiting` covers it) — a
    /// deadline already in the past would spin the loop.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let gif = self
            .gif
            .as_ref()
            .filter(|g| !g.is_waiting(now))
            .and_then(|g| g.next_due());
        let slide = self
            .slideshow
            .filter(|_| self.dir_files.len() > 1)
            .map(|at| at + self.slideshow_interval);
        let flash = self.flash.as_ref().map(|(_, at)| *at + FLASH_TTL);
        [gif, slide, flash].into_iter().flatten().min()
    }

    /// A GIF frame is due but still decoding: the loop should poll for it.
    pub fn is_waiting(&self, now: Instant) -> bool {
        self.gif.as_ref().is_some_and(|g| g.is_waiting(now))
    }

    /// Run whatever timers are due. Returns true if the picture changed.
    pub fn tick(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, now: Instant) -> bool {
        let mut changed = self.tick_gif(gpu, tex_pass, now);
        if self.flash.as_ref().is_some_and(|(_, at)| now >= *at + FLASH_TTL) {
            self.flash = None;
            changed = true;
        }
        if let Some(at) = self.slideshow {
            if now >= at + self.slideshow_interval && self.dir_files.len() > 1 {
                self.next_image(gpu, tex_pass);
                self.slideshow = Some(now);
                changed = true;
            }
        }
        changed
    }

    /// Show the next GIF frame once it's due, rewriting the texture in place
    /// (every frame is the full canvas size).
    fn tick_gif(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, now: Instant) -> bool {
        let Some(player) = self.gif.as_mut() else {
            return false;
        };
        let frame = player.tick(now);
        if player.is_still() {
            self.gif = None;
            return false;
        }
        if player.is_animated() {
            if let Some(info) = self.info.as_mut().filter(|i| i.format == "GIF") {
                info.format = "GIF (animated)".into();
            }
        }
        let Some(frame) = frame else { return false };
        let Some(mut img) = self.image.take() else {
            return false;
        };
        let mut slot = Some(img.texture);
        tex_pass.upload_reuse(gpu, &mut slot, &frame.rgba, frame.width, frame.height);
        if let Some(tex) = slot {
            img.texture = tex;
            self.image = Some(img);
        }
        true
    }

    // ── Trash ───────────────────────────────────────────────────────────

    /// Drop the open image from the sibling list (it's been trashed) and
    /// show the next one, or an empty view when the folder is now empty.
    pub fn remove_current(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) {
        // Locate the trashed file by path (the open image may not be in the
        // list at all, e.g. a hidden file opened directly).
        let idx = match self
            .dir_files
            .iter()
            .position(|p| Some(p) == self.path.as_ref())
        {
            Some(i) => {
                self.dir_files.remove(i);
                i
            }
            None => self.dir_index,
        };
        if self.dir_files.is_empty() {
            self.clear_image();
            return;
        }
        self.dir_index = idx.min(self.dir_files.len() - 1);
        if self.shuffle {
            self.shuffler.deal(self.dir_files.len(), self.dir_index);
        }
        let next = self.dir_files[self.dir_index].clone();
        self.open_image(gpu, tex_pass, &next);
    }

    fn clear_image(&mut self) {
        self.image = None;
        self.gif = None;
        self.path = None;
        self.info = None;
        self.listed = false;
        self.file_name.clear();
        self.dimensions_text.clear();
        self.status_text = "No images left in this folder".into();
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.shuffler.deal(self.dir_files.len(), self.dir_index);
        }
    }

    // ── View ────────────────────────────────────────────────────────────

    /// Zoom toward a point (cursor position) in physical pixel coords.
    pub fn zoom_at(&mut self, factor: f32, cx: f32, cy: f32, canvas_cx: f32, canvas_cy: f32) {
        let old_zoom = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.05, 50.0);
        let ratio = self.zoom / old_zoom;
        // Adjust pan so the point under cursor stays fixed
        let dx = cx - canvas_cx;
        let dy = cy - canvas_cy;
        self.pan_x = dx - ratio * (dx - self.pan_x);
        self.pan_y = dy - ratio * (dy - self.pan_y);
    }

    pub fn fit_to_view(&mut self) {
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_keeps_a_symlinked_file_as_the_link() {
        let dir = std::env::temp_dir().join(format!(
            "lntrn-image-viewer-resolve-{}",
            std::process::id()
        ));
        let (real, shown) = (dir.join("real"), dir.join("shown"));
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&shown).unwrap();
        std::fs::write(real.join("photo.png"), b"x").unwrap();
        std::os::unix::fs::symlink(real.join("photo.png"), shown.join("link.png")).unwrap();

        // Stays in the folder it was opened from: trashing it removes the
        // link, and Left/Right walk `shown`, not `real`.
        let got = resolve(&shown.join("link.png")).unwrap();
        assert_eq!(got, shown.canonicalize().unwrap().join("link.png"));
        assert!(resolve(&shown.join("missing.png")).is_none());
        // A dangling link has nothing to show.
        std::fs::remove_file(real.join("photo.png")).unwrap();
        assert!(resolve(&shown.join("link.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

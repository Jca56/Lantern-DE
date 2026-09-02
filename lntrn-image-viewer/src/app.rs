use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::info::ImageInfo;
use crate::loaders;

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

// ── Loaded image ────────────────────────────────────────────────────────────

pub struct LoadedImage {
    pub texture: GpuTexture,
    pub width: u32,
    pub height: u32,
    /// For vector images: the parsed SVG and its native aspect, so we can
    /// re-rasterize crisply at whatever pixel size it's displayed at.
    pub svg: Option<SvgImage>,
}

/// A loaded SVG kept around for on-demand re-rasterization at the display size.
pub struct SvgImage {
    pub source: String,
    /// Native (intrinsic) dimensions in SVG user units.
    pub native_w: f32,
    pub native_h: f32,
    /// Pixel size the current `texture` was rasterized at.
    pub rendered_w: u32,
    pub rendered_h: u32,
}

// ── GIF animation ───────────────────────────────────────────────────────────

pub struct GifFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub delay: Duration,
}

pub struct GifAnimation {
    pub frames: Vec<GifFrame>,
    pub current: usize,
    pub last_swap: Instant,
}

impl GifAnimation {
    pub fn current_delay(&self) -> Duration {
        self.frames[self.current].delay
    }

    /// Advance frame if enough time passed. Returns true if frame changed.
    pub fn tick(&mut self) -> bool {
        if self.frames.len() <= 1 {
            return false;
        }
        let elapsed = self.last_swap.elapsed();
        if elapsed >= self.current_delay() {
            self.current = (self.current + 1) % self.frames.len();
            self.last_swap = Instant::now();
            true
        } else {
            false
        }
    }
}

// ── Tiny PRNG (xorshift64) ──────────────────────────────────────────────────

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xa5a5_5a5a_a5a5_5a5a);
        Self {
            state: if nanos == 0 {
                0xdead_beef_cafe_babe
            } else {
                nanos
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() as usize) % n
    }
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
    /// Absolute path of the open image (None until something loads).
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
    // Shuffle playback
    pub shuffle: bool,
    shuffle_order: Vec<usize>,
    shuffle_pos: usize,
    rng: XorShift64,
    // GIF animation
    pub gif: Option<GifAnimation>,
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
            shuffle: false,
            shuffle_order: Vec::new(),
            shuffle_pos: 0,
            rng: XorShift64::from_time(),
            gif: None,
        }
    }

    pub fn open_image(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, path: &str) {
        let path = Path::new(path);
        let abs = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.status_text = format!("File not found: {} ({e})", path.display());
                return;
            }
        };

        // Scan directory for sibling images (only on first load or dir change)
        if let Some(parent) = abs.parent() {
            let should_rescan = self.dir_files.is_empty()
                || self
                    .dir_files
                    .first()
                    .and_then(|f| f.parent())
                    .map(|p| p != parent)
                    .unwrap_or(true);
            if should_rescan {
                self.scan_directory(parent);
            }
            // Find current file in the list
            if let Some(idx) = self.dir_files.iter().position(|f| f == &abs) {
                self.dir_index = idx;
            }
            // Keep shuffle ordering in sync
            if self.shuffle {
                if should_rescan || self.shuffle_order.len() != self.dir_files.len() {
                    self.regenerate_shuffle();
                } else if let Some(p) = self.shuffle_order.iter().position(|&i| i == self.dir_index)
                {
                    self.shuffle_pos = p;
                }
            }
        }

        // Check for animated GIF
        self.gif = None;
        if is_gif(&abs) {
            if let Some(anim) = loaders::load_gif_frames(&abs) {
                if anim.frames.len() > 1 {
                    // Upload first frame as texture
                    let f = &anim.frames[0];
                    let tex = tex_pass.upload(gpu, &f.rgba, f.width, f.height);
                    let (w, h) = (f.width, f.height);
                    let info = ImageInfo::gather(&abs, "GIF (animated)", None);
                    self.set_loaded(abs.clone(), tex, w, h, None, info);
                    self.gif = Some(anim);
                    return;
                }
            }
        }

        if is_svg(&abs) {
            match loaders::load_svg_texture(gpu, tex_pass, &abs) {
                Some((tex, w, h, svg)) => {
                    let info = ImageInfo::gather(&abs, "SVG", None);
                    self.set_loaded(abs, tex, w, h, Some(svg), info);
                }
                None => self.status_text = format!("Cannot load: {}", abs.display()),
            }
        } else {
            match loaders::load_raster_texture(gpu, tex_pass, &abs) {
                Some(r) => {
                    let format = crate::info::format_name(r.format);
                    let info = ImageInfo::gather(&abs, &format, r.exif.as_deref());
                    self.set_loaded(abs, r.tex, r.width, r.height, None, info);
                }
                None => self.status_text = format!("Cannot load: {}", abs.display()),
            }
        }
    }

    fn set_loaded(
        &mut self,
        abs: PathBuf,
        tex: GpuTexture,
        w: u32,
        h: u32,
        svg: Option<SvgImage>,
        info: ImageInfo,
    ) {
        self.info = Some(info);
        self.path = Some(abs.clone());
        self.file_name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_default();
        self.status_text = abs.to_string_lossy().into();
        // For SVG, report native (intrinsic) size rather than the rasterized size.
        match &svg {
            Some(s) => {
                self.dimensions_text = format!(
                    "{} × {}",
                    s.native_w.ceil() as u32,
                    s.native_h.ceil() as u32
                )
            }
            None => self.dimensions_text = format!("{w} × {h}"),
        }
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.image = Some(LoadedImage {
            texture: tex,
            width: w,
            height: h,
            svg,
        });
    }

    /// Re-rasterize the loaded SVG to match the on-screen pixel size, so it
    /// stays sharp when the window/zoom grows. `disp_w`/`disp_h` are the pixel
    /// dimensions the image is currently drawn at. No-op for raster images, and
    /// only re-renders when the target meaningfully exceeds the current texture
    /// (hysteresis avoids churning a texture every frame during a drag-resize).
    pub fn maybe_rerender_svg(
        &mut self,
        gpu: &GpuContext,
        tex_pass: &TexturePass,
        disp_w: f32,
        disp_h: f32,
    ) {
        let Some(img) = &self.image else { return };
        let Some(svg) = &img.svg else { return };

        // Target raster size: cover the displayed size, capped, never below native.
        let want_w = (disp_w.ceil() as u32)
            .clamp(svg.native_w.ceil() as u32, 8192)
            .max(1);
        let want_h = (disp_h.ceil() as u32)
            .clamp(svg.native_h.ceil() as u32, 8192)
            .max(1);

        // Re-render only on a ≥25% jump in either axis (up or down) to avoid
        // per-frame churn while still tracking large zoom changes.
        let grew = want_w as f32 > svg.rendered_w as f32 * 1.25
            || want_h as f32 > svg.rendered_h as f32 * 1.25;
        let shrank = (want_w as f32) < svg.rendered_w as f32 * 0.75
            && (want_h as f32) < svg.rendered_h as f32 * 0.75;
        if !grew && !shrank {
            return;
        }

        if let Some((tex, rw, rh)) = loaders::rasterize_svg(
            gpu,
            tex_pass,
            &svg.source,
            svg.native_w,
            svg.native_h,
            want_w,
            want_h,
        ) {
            if let Some(img) = &mut self.image {
                img.texture = tex;
                img.width = rw;
                img.height = rh;
                if let Some(svg) = &mut img.svg {
                    svg.rendered_w = rw;
                    svg.rendered_h = rh;
                }
            }
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
        if self.dir_files.is_empty() {
            return;
        }
        if self.shuffle && !self.shuffle_order.is_empty() {
            self.shuffle_pos += 1;
            if self.shuffle_pos >= self.shuffle_order.len() {
                // Reached end of shuffled playlist — reshuffle for the next pass
                self.regenerate_shuffle();
                self.shuffle_pos = 0;
            }
            self.dir_index = self.shuffle_order[self.shuffle_pos];
        } else {
            self.dir_index = (self.dir_index + 1) % self.dir_files.len();
        }
        let path = self.dir_files[self.dir_index].to_string_lossy().to_string();
        self.open_image(gpu, tex_pass, &path);
    }

    pub fn prev_image(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) {
        if self.dir_files.is_empty() {
            return;
        }
        if self.shuffle && !self.shuffle_order.is_empty() {
            self.shuffle_pos = if self.shuffle_pos == 0 {
                self.shuffle_order.len() - 1
            } else {
                self.shuffle_pos - 1
            };
            self.dir_index = self.shuffle_order[self.shuffle_pos];
        } else {
            self.dir_index = if self.dir_index == 0 {
                self.dir_files.len() - 1
            } else {
                self.dir_index - 1
            };
        }
        let path = self.dir_files[self.dir_index].to_string_lossy().to_string();
        self.open_image(gpu, tex_pass, &path);
    }

    // ── Slideshow / flash / trash ───────────────────────────────────────

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

    pub fn flash_remaining(&self) -> Option<Duration> {
        self.flash
            .as_ref()
            .map(|(_, at)| FLASH_TTL.saturating_sub(at.elapsed()))
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

    /// Time until the slideshow advances, if one is running.
    pub fn slideshow_remaining(&self) -> Option<Duration> {
        self.slideshow
            .map(|at| self.slideshow_interval.saturating_sub(at.elapsed()))
    }

    /// Advance the slideshow when its interval is up. Returns true on advance.
    pub fn tick_slideshow(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) -> bool {
        let Some(at) = self.slideshow else {
            return false;
        };
        if at.elapsed() < self.slideshow_interval || self.dir_files.len() < 2 {
            return false;
        }
        self.next_image(gpu, tex_pass);
        self.slideshow = Some(Instant::now());
        true
    }

    /// Whether anything time-driven needs the loop to keep ticking.
    pub fn is_animating(&self) -> bool {
        self.gif.is_some() || self.slideshow.is_some() || self.flash.is_some()
    }

    /// Drop the open image from the sibling list (it's been trashed) and
    /// show the next one, or an empty view when the folder is now empty.
    pub fn remove_current(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) {
        if self.dir_files.is_empty() {
            self.clear_image();
            return;
        }
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
            self.regenerate_shuffle();
        }
        let next = self.dir_files[self.dir_index].to_string_lossy().to_string();
        self.open_image(gpu, tex_pass, &next);
    }

    fn clear_image(&mut self) {
        self.image = None;
        self.gif = None;
        self.path = None;
        self.info = None;
        self.file_name.clear();
        self.dimensions_text.clear();
        self.status_text = "No images left in this folder".into();
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.regenerate_shuffle();
        }
    }

    fn regenerate_shuffle(&mut self) {
        let n = self.dir_files.len();
        self.shuffle_order = (0..n).collect();
        // Fisher-Yates
        for i in (1..n).rev() {
            let j = self.rng.below(i + 1);
            self.shuffle_order.swap(i, j);
        }
        self.shuffle_pos = self
            .shuffle_order
            .iter()
            .position(|&idx| idx == self.dir_index)
            .unwrap_or(0);
    }

    /// Tick GIF animation — re-uploads texture if frame changed. Returns true if needs redraw.
    pub fn tick_gif(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) -> bool {
        let gif = match &mut self.gif {
            Some(g) => g,
            None => return false,
        };
        if !gif.tick() {
            return false;
        }
        let frame = &gif.frames[gif.current];
        let tex = tex_pass.upload(gpu, &frame.rgba, frame.width, frame.height);
        if let Some(img) = &mut self.image {
            img.texture = tex;
        }
        true
    }

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

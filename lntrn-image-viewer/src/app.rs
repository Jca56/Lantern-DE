use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use lntrn_render::{GpuContext, GpuTexture, TexturePass};

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

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
}

/// Read just the header of an image to get its dimensions without a full decode.
/// Used to pick the initial window size before we even create the toplevel.
pub fn peek_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    if is_svg(path) {
        let data = std::fs::read_to_string(path).ok()?;
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb = svg_font_database();
        let tree = resvg::usvg::Tree::from_str(&data, &opt).ok()?;
        let s = tree.size();
        Some((s.width().ceil() as u32, s.height().ceil() as u32))
    } else {
        image::ImageReader::open(path)
            .ok()?
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()
    }
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
            if let Some(anim) = load_gif_frames(&abs) {
                if anim.frames.len() > 1 {
                    // Upload first frame as texture
                    let f = &anim.frames[0];
                    let tex = tex_pass.upload(gpu, &f.rgba, f.width, f.height);
                    let (w, h) = (f.width, f.height);
                    self.set_loaded(abs.clone(), tex, w, h, None);
                    self.gif = Some(anim);
                    return;
                }
            }
        }

        if is_svg(&abs) {
            match load_svg_texture(gpu, tex_pass, &abs) {
                Some((tex, w, h, svg)) => self.set_loaded(abs, tex, w, h, Some(svg)),
                None => self.status_text = format!("Cannot load: {}", abs.display()),
            }
        } else {
            match load_raster_texture(gpu, tex_pass, &abs) {
                Some((tex, w, h)) => self.set_loaded(abs, tex, w, h, None),
                None => self.status_text = format!("Cannot load: {}", abs.display()),
            }
        }
    }

    fn set_loaded(&mut self, abs: PathBuf, tex: GpuTexture, w: u32, h: u32, svg: Option<SvgImage>) {
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

        if let Some((tex, rw, rh)) = rasterize_svg(
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

    fn scan_directory(&mut self, dir: &Path) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_supported(p))
            .collect();
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
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

// ── Image loading helpers ───────────────────────────────────────────────────

fn load_raster_texture(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
) -> Option<(GpuTexture, u32, u32)> {
    let img = image::open(path).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let tex = tex_pass.upload(gpu, &rgba, w, h);
    Some((tex, w, h))
}

fn load_gif_frames(path: &Path) -> Option<GifAnimation> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = GifDecoder::new(BufReader::new(file)).ok()?;
    let frames_iter = decoder.into_frames();
    let mut frames = Vec::new();
    for result in frames_iter {
        let frame = result.ok()?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = if denom == 0 { 100 } else { numer / denom };
        // GIF spec: 0 or very small delay defaults to 100ms
        let delay_ms = if delay_ms < 20 { 100 } else { delay_ms };
        let buf = frame.into_buffer();
        let (w, h) = (buf.width(), buf.height());
        frames.push(GifFrame {
            rgba: buf.into_raw(),
            width: w,
            height: h,
            delay: Duration::from_millis(delay_ms as u64),
        });
    }
    if frames.is_empty() {
        return None;
    }
    Some(GifAnimation {
        frames,
        current: 0,
        last_swap: Instant::now(),
    })
}

pub(crate) fn svg_font_database() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// Initial SVG load: rasterize at native size and keep the source around so it
/// can be re-rasterized larger on demand (see `App::maybe_rerender_svg`).
fn load_svg_texture(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
) -> Option<(GpuTexture, u32, u32, SvgImage)> {
    let svg_data = std::fs::read_to_string(path).ok()?;
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb = svg_font_database();
    let tree = resvg::usvg::Tree::from_str(&svg_data, &opt).ok()?;
    let size = tree.size();
    let native_w = size.width();
    let native_h = size.height();

    // Start at native size; window/zoom growth triggers a sharper re-render.
    let want_w = (native_w.ceil() as u32).min(8192).max(1);
    let want_h = (native_h.ceil() as u32).min(8192).max(1);
    let (tex, rw, rh) =
        rasterize_svg(gpu, tex_pass, &svg_data, native_w, native_h, want_w, want_h)?;

    let svg = SvgImage {
        source: svg_data,
        native_w,
        native_h,
        rendered_w: rw,
        rendered_h: rh,
    };
    Some((tex, rw, rh, svg))
}

/// Rasterize an SVG source string to a GPU texture at `target_w × target_h`
/// pixels, preserving the native aspect ratio. Returns the texture and the
/// actual pixel size used.
fn rasterize_svg(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    source: &str,
    native_w: f32,
    native_h: f32,
    target_w: u32,
    target_h: u32,
) -> Option<(GpuTexture, u32, u32)> {
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb = svg_font_database();
    let tree = resvg::usvg::Tree::from_str(source, &opt).ok()?;

    let render_w = target_w.clamp(1, 8192);
    let render_h = target_h.clamp(1, 8192);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(render_w, render_h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        render_w as f32 / native_w,
        render_h as f32 / native_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let tex = tex_pass.upload(gpu, pixmap.data(), render_w, render_h);
    Some((tex, render_w, render_h))
}

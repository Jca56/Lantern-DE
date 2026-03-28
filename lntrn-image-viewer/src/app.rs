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

fn is_supported(path: &Path) -> bool {
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
    pub path: PathBuf,
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
                || self.dir_files.first()
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
                    self.set_loaded(abs.clone(), tex, w, h);
                    self.gif = Some(anim);
                    return;
                }
            }
        }

        let result = if is_svg(&abs) {
            load_svg_texture(gpu, tex_pass, &abs)
        } else {
            load_raster_texture(gpu, tex_pass, &abs)
        };

        match result {
            Some((tex, w, h)) => self.set_loaded(abs, tex, w, h),
            None => {
                self.status_text = format!("Cannot load: {}", abs.display());
            }
        }
    }

    fn set_loaded(&mut self, abs: PathBuf, tex: GpuTexture, w: u32, h: u32) {
        self.file_name = abs.file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_default();
        self.status_text = abs.to_string_lossy().into();
        self.dimensions_text = format!("{w} × {h}");
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.image = Some(LoadedImage {
            texture: tex,
            width: w,
            height: h,
            path: abs,
        });
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
        if self.dir_files.is_empty() { return; }
        self.dir_index = (self.dir_index + 1) % self.dir_files.len();
        let path = self.dir_files[self.dir_index].to_string_lossy().to_string();
        self.open_image(gpu, tex_pass, &path);
    }

    pub fn prev_image(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) {
        if self.dir_files.is_empty() { return; }
        self.dir_index = if self.dir_index == 0 {
            self.dir_files.len() - 1
        } else {
            self.dir_index - 1
        };
        let path = self.dir_files[self.dir_index].to_string_lossy().to_string();
        self.open_image(gpu, tex_pass, &path);
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

fn svg_font_database() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

fn load_svg_texture(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
) -> Option<(GpuTexture, u32, u32)> {
    let svg_data = std::fs::read_to_string(path).ok()?;
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb = svg_font_database();
    let tree = resvg::usvg::Tree::from_str(&svg_data, &opt).ok()?;

    let size = tree.size();
    let svg_w = size.width();
    let svg_h = size.height();
    // Render at native size, capped at 8192
    let render_w = (svg_w.ceil() as u32).min(8192).max(1);
    let render_h = (svg_h.ceil() as u32).min(8192).max(1);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(render_w, render_h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        render_w as f32 / svg_w,
        render_h as f32 / svg_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let tex = tex_pass.upload(gpu, pixmap.data(), render_w, render_h);
    Some((tex, render_w, render_h))
}

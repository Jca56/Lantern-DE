//! Quick Look: Space-bar overlay preview of the selected file.
//!
//! Space opens/closes, Esc closes, ←/→ move to the previous/next file
//! (key handling lives in wayland_actions/key.rs). Content loads on a
//! background thread; the render thread polls and uploads the texture.
//!
//! v1 surfaces: full-res images (incl. SVG), text files, and a full-res
//! still frame for videos. Video *playback* (ffmpeg frame streaming +
//! audio + MPRIS) is designed to replace the still-frame path later —
//! everything else (overlay, keys, loader plumbing) stays as is.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lntrn_render::{
    Color, GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass,
};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

/// Cap uploaded textures — no zoom yet, so pixels beyond ~4K are wasted VRAM.
const MAX_TEX_DIM: u32 = 4096;
/// Decode guards (more generous than thumbnails — this is one image at a
/// time, user-initiated — but still bounded so a corrupt header can't OOM).
const MAX_DECODE_DIM: u32 = 16_384;
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;
const TEXT_MAX_BYTES: usize = 256 * 1024;
const TEXT_MAX_LINES: usize = 500;

enum Loaded {
    /// `src_w/src_h` are the pre-downscale source dimensions for the meta line.
    Image {
        rgba: Vec<u8>,
        w: u32,
        h: u32,
        src_w: u32,
        src_h: u32,
    },
    Text(Vec<String>),
    Failed(String),
}

pub struct QuickLook {
    pub path: PathBuf,
    file_size: u64,
    is_video: bool,
    pending: Arc<Mutex<Option<Loaded>>>,
    texture: Option<GpuTexture>,
    /// Source dimensions before any downscale-for-VRAM (shown in the meta line).
    source_dims: Option<(u32, u32)>,
    text_lines: Option<Vec<String>>,
    error: Option<String>,
}

impl QuickLook {
    pub fn open(path: &Path) -> Self {
        let pending = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&pending);
        let bg_path = path.to_path_buf();
        std::thread::spawn(move || {
            let loaded = load(&bg_path);
            *slot.lock().unwrap() = Some(loaded);
        });
        Self {
            path: path.to_path_buf(),
            file_size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            is_video: crate::icons::is_video_file(
                path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
            ),
            pending,
            texture: None,
            source_dims: None,
            text_lines: None,
            error: None,
        }
    }

    /// Still waiting on the loader thread — the event loop polls fast.
    pub fn loading(&self) -> bool {
        self.texture.is_none() && self.text_lines.is_none() && self.error.is_none()
    }

    /// Collect the loader result and upload any image to the GPU. Called
    /// early in render_frame, before any texture borrows are taken.
    pub fn poll_upload(&mut self, gpu: &GpuContext, tex: &TexturePass) {
        if !self.loading() {
            return;
        }
        let Some(loaded) = self.pending.lock().unwrap().take() else {
            return;
        };
        match loaded {
            Loaded::Image {
                rgba,
                w,
                h,
                src_w,
                src_h,
            } => {
                self.texture = Some(tex.upload(gpu, &rgba, w, h));
                self.source_dims = Some((src_w, src_h));
            }
            Loaded::Text(lines) => self.text_lines = Some(lines),
            Loaded::Failed(reason) => self.error = Some(reason),
        }
    }
}

// ── Loading (background thread) ─────────────────────────────────────────────

fn load(path: &Path) -> Loaded {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if crate::icons::is_video_file(name) {
        return load_video_still(path);
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    if ext == "svg" {
        return load_svg(path);
    }
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" | "tif"
    ) {
        return load_image(path);
    }
    load_text(path)
}

fn load_image(path: &Path) -> Loaded {
    let reader = match image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
        Ok(r) => r,
        Err(_) => return Loaded::Failed("Unreadable file".into()),
    };
    let mut reader = reader;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    match reader.decode() {
        Ok(img) => {
            let (src_w, src_h) = (img.width(), img.height());
            let img = if src_w > MAX_TEX_DIM || src_h > MAX_TEX_DIM {
                img.thumbnail(MAX_TEX_DIM, MAX_TEX_DIM)
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            Loaded::Image {
                rgba: rgba.into_raw(),
                w,
                h,
                src_w,
                src_h,
            }
        }
        Err(_) => Loaded::Failed("Could not decode image (corrupt or too large)".into()),
    }
}

fn load_svg(path: &Path) -> Loaded {
    let Ok(data) = std::fs::read(path) else {
        return Loaded::Failed("Unreadable file".into());
    };
    let Ok(tree) = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()) else {
        return Loaded::Failed("Invalid SVG".into());
    };
    let size = tree.size();
    let target = 2048.0f32;
    let scale = (target / size.width()).min(target / size.height()).min(8.0);
    let w = (size.width() * scale).ceil() as u32;
    let h = (size.height() * scale).ceil() as u32;
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(w, h) else {
        return Loaded::Failed("SVG too large".into());
    };
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Loaded::Image {
        rgba: pixmap.take(),
        w,
        h,
        src_w: w,
        src_h: h,
    }
}

/// Full-resolution single frame via ffmpeg — placeholder until real playback.
fn load_video_still(path: &Path) -> Loaded {
    use std::process::Command;
    let output = Command::new("ffmpeg")
        .args(["-ss", "1", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-loglevel",
            "error",
            "pipe:1",
        ])
        .output();
    let Ok(output) = output else {
        return Loaded::Failed("ffmpeg not available".into());
    };
    if !output.status.success() || output.stdout.is_empty() {
        return Loaded::Failed("Could not extract video frame".into());
    }
    match image::load_from_memory(&output.stdout) {
        Ok(img) => {
            let (src_w, src_h) = (img.width(), img.height());
            let img = if src_w > MAX_TEX_DIM || src_h > MAX_TEX_DIM {
                img.thumbnail(MAX_TEX_DIM, MAX_TEX_DIM)
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            Loaded::Image {
                rgba: rgba.into_raw(),
                w,
                h,
                src_w,
                src_h,
            }
        }
        Err(_) => Loaded::Failed("Could not decode video frame".into()),
    }
}

fn load_text(path: &Path) -> Loaded {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return Loaded::Failed("Unreadable file".into());
    };
    let mut buf = vec![0u8; TEXT_MAX_BYTES];
    let mut filled = 0usize;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return Loaded::Failed("Unreadable file".into()),
        }
    }
    buf.truncate(filled);
    if buf.is_empty() {
        return Loaded::Text(vec!["(empty file)".to_string()]);
    }
    // NUL in the head = binary; don't dump garbage glyphs on the screen.
    if buf.iter().take(8192).any(|&b| b == 0) {
        return Loaded::Failed("No preview available for this file type".into());
    }
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<String> = text
        .lines()
        .take(TEXT_MAX_LINES)
        .map(|l| {
            // Tabs render as missing glyphs in the UI font; expand them.
            let l = l.replace('\t', "    ");
            if l.chars().count() > 400 {
                l.chars().take(400).collect()
            } else {
                l
            }
        })
        .collect();
    Loaded::Text(lines)
}

// ── Drawing (layer 1 / modal) ────────────────────────────────────────────────

/// Draw the overlay. Returns the texture draw for the modal texture batch,
/// if an image is ready.
#[allow(clippy::too_many_arguments)]
pub fn draw_quick_look<'a>(
    ql: &'a QuickLook,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    input: &mut InteractionContext,
    screen_w: f32,
    screen_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> Option<TextureDraw<'a>> {
    // Backdrop — clicking it closes (handled in click.rs via the zone).
    let backdrop = Rect::new(0.0, 0.0, screen_w, screen_h);
    input.add_zone(crate::ZONE_QUICK_LOOK, backdrop);
    painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.78));

    // Bottom info bar: filename + meta.
    let name = ql.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let name_font = 22.0 * s;
    let meta_font = 16.0 * s;
    let bar_h = name_font + meta_font + 26.0 * s;
    let name_y = screen_h - bar_h;
    let est_w = name.chars().count() as f32 * name_font * 0.52;
    TextLabel::new(name, (screen_w - est_w) * 0.5, name_y)
        .size(FontSize::Custom(name_font))
        .color(palette.text)
        .max_width(screen_w * 0.9)
        .draw(text, sw, sh);
    let mut meta = format_bytes(ql.file_size);
    if let Some(tex) = &ql.texture {
        let (dw, dh) = ql.source_dims.unwrap_or((tex.width, tex.height));
        meta = format!("{dw} × {dh}  •  {meta}");
    }
    if ql.is_video {
        meta = format!("Video (still frame)  •  {meta}");
    }
    let est_w = meta.chars().count() as f32 * meta_font * 0.52;
    TextLabel::new(
        &meta,
        (screen_w - est_w) * 0.5,
        name_y + name_font + 8.0 * s,
    )
    .size(FontSize::Custom(meta_font))
    .color(palette.text_secondary)
    .max_width(screen_w * 0.9)
    .draw(text, sw, sh);

    // Content area above the info bar.
    let margin = 40.0 * s;
    let content = Rect::new(
        margin,
        margin,
        screen_w - margin * 2.0,
        screen_h - bar_h - margin * 1.5,
    );

    if let Some(tex) = &ql.texture {
        let (x, y, w, h) =
            crate::icons::fit_in_box(tex, content.x, content.y, content.w, content.h);
        return Some(TextureDraw::new(tex, x, y, w, h));
    }

    if let Some(lines) = &ql.text_lines {
        // Text panel — centered card, monaco-ish density, clipped to fit.
        let panel_w = (900.0 * s).min(content.w);
        let panel = Rect::new(
            content.x + (content.w - panel_w) * 0.5,
            content.y,
            panel_w,
            content.h,
        );
        painter.rect_filled(panel, 10.0 * s, palette.surface);
        painter.rect_stroke(panel, 10.0 * s, 1.0 * s, palette.muted.with_alpha(0.25));
        let pad = 18.0 * s;
        let line_font = 16.0 * s;
        let line_h = line_font * 1.45;
        let max_lines = (((panel.h - pad * 2.0) / line_h).floor() as usize).min(lines.len());
        let mut ly = panel.y + pad;
        for line in lines.iter().take(max_lines) {
            if !line.is_empty() {
                TextLabel::new(line, panel.x + pad, ly)
                    .size(FontSize::Custom(line_font))
                    .color(palette.text)
                    .max_width(panel.w - pad * 2.0)
                    .draw(text, sw, sh);
            }
            ly += line_h;
        }
        if lines.len() > max_lines {
            let more = format!("… {} more lines", lines.len() - max_lines);
            TextLabel::new(&more, panel.x + pad, panel.y + panel.h - pad - line_font)
                .size(FontSize::Custom(line_font))
                .color(palette.muted)
                .draw(text, sw, sh);
        }
        return None;
    }

    // Loading / error state — centered message.
    let msg = ql.error.as_deref().unwrap_or("Loading…");
    let msg_font = 20.0 * s;
    let est_w = msg.chars().count() as f32 * msg_font * 0.52;
    TextLabel::new(msg, (screen_w - est_w) * 0.5, screen_h * 0.5 - msg_font)
        .size(FontSize::Custom(msg_font))
        .color(if ql.error.is_some() {
            palette.text_secondary
        } else {
            palette.muted
        })
        .draw(text, sw, sh);
    None
}

fn format_bytes(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = size as f64;
    if f >= GB {
        format!("{:.2} GB", f / GB)
    } else if f >= MB {
        format!("{:.1} MB", f / MB)
    } else if f >= KB {
        format!("{:.0} KB", f / KB)
    } else {
        format!("{size} B")
    }
}

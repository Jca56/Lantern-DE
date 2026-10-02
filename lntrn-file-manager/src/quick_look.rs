//! Quick Look: Space-bar overlay preview of the selected file.
//!
//! Space opens/closes, Esc closes, ←/→ move to the previous/next file
//! (key handling lives in wayland_actions/key.rs). Content loads on a
//! background thread; the render thread polls and uploads the texture.
//! Nothing here touches the file on the render thread, and on a slow mount
//! (phone, network share) the file is not read at all: there the first read
//! downloads the whole file under the device's one lock.
//!
//! v1 surfaces: full-res images (incl. SVG), text files, and a full-res
//! still frame for videos. Video *playback* (ffmpeg frame streaming +
//! audio + MPRIS) is designed to replace the still-frame path later —
//! everything else (overlay, keys, loader plumbing) stays as is.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Lines, plus whether the file had more than was read.
    Text(Vec<String>, bool),
    Failed(String),
}

/// Counts the previews opened. A loader only starts its work while it is
/// still the newest one.
static NEWEST: AtomicU64 = AtomicU64::new(0);
/// One loader works at a time. Stepping through a folder with the arrow
/// keys used to start a thread (and an ffmpeg) per key press, all of which
/// ran to the end even though only the last one was looked at.
static TURN: Mutex<()> = Mutex::new(());

const SLOW_MOUNT_NOTE: &str =
    "No full preview on phones and network folders: showing one would download the whole file.";

pub struct QuickLook {
    pub path: PathBuf,
    /// The listing entry, for the thumbnail the view may already have.
    entry: crate::fs::FileEntry,
    /// Nothing is loaded for this file (slow mount): the listing's
    /// thumbnail stands in, if there is one.
    thumb_only: bool,
    file_size: u64,
    is_video: bool,
    pending: Arc<Mutex<Option<Loaded>>>,
    texture: Option<GpuTexture>,
    /// Source dimensions before any downscale-for-VRAM (shown in the meta line).
    source_dims: Option<(u32, u32)>,
    text_lines: Option<Vec<String>>,
    /// The file is longer than `text_lines` (line or byte cap hit).
    text_truncated: bool,
    error: Option<String>,
}

impl QuickLook {
    /// `entry` comes from the listing, which already knows the size: no
    /// stat here (on a phone it would wait behind a running download).
    pub fn open(entry: &crate::fs::FileEntry) -> Self {
        let path = entry.path.as_path();
        let pending = Arc::new(Mutex::new(None));
        let thumb_only = crate::fs::is_slow_path(path);
        let generation = NEWEST.fetch_add(1, Ordering::SeqCst) + 1;
        if !thumb_only {
            let slot = Arc::clone(&pending);
            let bg_path = path.to_path_buf();
            std::thread::spawn(move || {
                let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
                if NEWEST.load(Ordering::SeqCst) != generation {
                    // Another file was opened while this one waited its
                    // turn; nobody will look at the result.
                    return;
                }
                // Always deliver something: a panicking decoder would
                // otherwise leave the overlay on "Loading…" (and the loop
                // at 60 fps).
                // Regular files only: opening a FIFO (or a tty) waits for a
                // writer that never comes, and this loader holds the one
                // turn there is; every later preview would wait behind it.
                // (A stat is fine here: slow mounts never get this far.)
                let regular = std::fs::metadata(&bg_path).is_ok_and(|m| m.is_file());
                let loaded = if regular {
                    std::panic::catch_unwind(|| load(&bg_path))
                        .unwrap_or_else(|_| Loaded::Failed("Could not load this file".into()))
                } else {
                    Loaded::Failed("No preview available for this kind of file".into())
                };
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(loaded);
            });
        }
        Self {
            path: path.to_path_buf(),
            entry: entry.clone(),
            thumb_only,
            file_size: entry.size,
            is_video: crate::icons::is_video_file(&entry.name),
            pending,
            texture: None,
            source_dims: None,
            text_lines: None,
            text_truncated: false,
            error: thumb_only.then(|| SLOW_MOUNT_NOTE.to_string()),
        }
    }

    /// The entry whose listing thumbnail may stand in for the preview
    /// (only when nothing is loaded for this file).
    pub fn thumbnail_entry(&self) -> Option<&crate::fs::FileEntry> {
        self.thumb_only.then_some(&self.entry)
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
        let Some(loaded) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
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
            Loaded::Text(lines, truncated) => {
                self.text_lines = Some(lines);
                self.text_truncated = truncated;
            }
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
    // Oriented first, so the dimensions shown are the ones the user sees.
    match crate::thumbs::decode_oriented(reader, MAX_DECODE_DIM, MAX_DECODE_BYTES) {
        Some(img) => {
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
        None => Loaded::Failed("Could not decode image (corrupt or too large)".into()),
    }
}

fn load_svg(path: &Path) -> Loaded {
    let Some(data) = crate::thumbs::read_svg_capped(path) else {
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
        rgba: pixmap.take_demultiplied(),
        w,
        h,
        src_w: w,
        src_h: h,
    }
}

/// Full-resolution single frame via ffmpeg — placeholder until real playback.
fn load_video_still(path: &Path) -> Loaded {
    let Some(png) = crate::thumbs::ffmpeg_frame_png(path, None) else {
        return Loaded::Failed("Could not extract video frame".into());
    };
    match image::load_from_memory(&png) {
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
    // Filled to the brim: there is (almost certainly) more file than buffer.
    let mut truncated = filled == buf.len();
    buf.truncate(filled);
    if buf.is_empty() {
        return Loaded::Text(vec!["(empty file)".to_string()], false);
    }
    // NUL in the head = binary; don't dump garbage glyphs on the screen.
    if buf.iter().take(8192).any(|&b| b == 0) {
        return Loaded::Failed("No preview available for this file type".into());
    }
    let text = String::from_utf8_lossy(&buf);
    truncated |= text.lines().nth(TEXT_MAX_LINES).is_some();
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
    Loaded::Text(lines, truncated)
}

// ── Drawing (layer 1 / modal) ────────────────────────────────────────────────

/// Draw the overlay. Returns the texture draw for the modal texture batch,
/// if an image is ready.
#[allow(clippy::too_many_arguments)]
pub fn draw_quick_look<'a>(
    ql: &'a QuickLook,
    thumbnail: Option<&'a GpuTexture>,
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
    if ql.is_video && !ql.thumb_only {
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
        // Rows that fit. When there is more text than rows (or more file
        // than was loaded) the last row is given to the "… more" footer
        // instead of having the footer drawn on top of a line of text.
        let fit = ((panel.h - pad * 2.0) / line_h).floor().max(1.0) as usize;
        let overflow = lines.len() > fit || ql.text_truncated;
        let max_lines = if overflow {
            fit.saturating_sub(1).min(lines.len())
        } else {
            lines.len()
        };
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
        if overflow {
            // The loaded lines are capped, so past the cap the real count is
            // unknown: say "more" without a number.
            let more = if ql.text_truncated {
                "… more".to_string()
            } else {
                format!("… {} more lines", lines.len() - max_lines)
            };
            TextLabel::new(&more, panel.x + pad, panel.y + pad + max_lines as f32 * line_h)
                .size(FontSize::Custom(line_font))
                .color(palette.muted)
                .draw(text, sw, sh);
        }
        return None;
    }

    // Loading / error state — centered message.
    let msg = ql.error.as_deref().unwrap_or("Loading…");
    let msg_font = 20.0 * s;
    let lines = crate::dialogs::wrap_lines(text, msg, msg_font, screen_w - margin * 2.0);
    let line_h = msg_font * 1.4;
    let block_h = line_h * lines.len() as f32;
    // The listing's thumbnail, when nothing is loaded for this file: shown
    // above the note at twice its size at most (it is small, and blowing
    // it up to the window would only show its pixels).
    let mut msg_y = screen_h * 0.5 - msg_font;
    let mut stand_in = None;
    if let Some(tex) = thumbnail.filter(|_| ql.thumb_only) {
        let gap = msg_font;
        let side = (tex.width.max(tex.height) as f32 * 2.0).min(content.h - block_h - gap);
        if side > 0.0 {
            let top = content.y + (content.h - side - gap - block_h) * 0.5;
            let (x, y, w, h) =
                crate::icons::fit_in_box(tex, (screen_w - side) * 0.5, top, side, side);
            stand_in = Some(TextureDraw::new(tex, x, y, w, h));
            msg_y = top + side + gap;
        }
    }
    let color = if ql.error.is_some() {
        palette.text_secondary
    } else {
        palette.muted
    };
    for line in &lines {
        let line_w = text.measure_width(line, msg_font);
        TextLabel::new(line, (screen_w - line_w) * 0.5, msg_y)
            .size(FontSize::Custom(msg_font))
            .color(color)
            // Slack so the measured last glyph isn't clipped by the bound.
            .max_width(line_w + 8.0 * s)
            .draw(text, sw, sh);
        msg_y += line_h;
    }
    stand_in
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

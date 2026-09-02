//! Decoding images into GPU textures: raster (EXIF + orientation aware),
//! SVG (kept re-rasterizable), and animated GIF frames. Lives apart from
//! app.rs so the viewer state file stays about state, not codecs.

use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use image::codecs::gif::GifDecoder;
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat};
use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::app::{is_svg, GifAnimation, GifFrame, SvgImage};

/// A decoded raster image on the GPU plus the metadata the info overlay wants.
pub struct RasterLoad {
    pub tex: GpuTexture,
    pub width: u32,
    pub height: u32,
    pub format: Option<ImageFormat>,
    /// Raw EXIF blob (TIFF structure) when the container carried one.
    pub exif: Option<Vec<u8>>,
}

/// A decoded raster on the CPU, already rotated per its EXIF orientation.
pub struct DecodedRaster {
    pub image: DynamicImage,
    pub format: Option<ImageFormat>,
    pub exif: Option<Vec<u8>>,
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

pub(crate) fn svg_font_database() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

// ── Raster ──────────────────────────────────────────────────────────────────

/// Decode a raster file and honour its EXIF orientation, so phone photos come
/// up the right way round. The EXIF blob is kept for the info overlay.
pub fn decode_raster(path: &Path) -> Option<DecodedRaster> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let format = reader.format();
    let mut decoder = reader.into_decoder().ok()?;
    // Pull EXIF before decoding — the decoder hands it out once.
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = exif
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    Some(DecodedRaster {
        image,
        format,
        exif,
    })
}

pub fn load_raster_texture(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
) -> Option<RasterLoad> {
    let decoded = decode_raster(path)?;
    let rgba = decoded.image.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let tex = tex_pass.upload(gpu, &rgba, w, h);
    Some(RasterLoad {
        tex,
        width: w,
        height: h,
        format: decoded.format,
        exif: decoded.exif,
    })
}

// ── GIF ─────────────────────────────────────────────────────────────────────

pub fn load_gif_frames(path: &Path) -> Option<GifAnimation> {
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

// ── SVG ─────────────────────────────────────────────────────────────────────

/// Initial SVG load: rasterize at native size and keep the source around so it
/// can be re-rasterized larger on demand (see `App::maybe_rerender_svg`).
pub fn load_svg_texture(
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

/// Rasterize an SVG source string to RGBA pixels at `target_w × target_h`,
/// stretching the native box to fit (callers keep the aspect ratio).
pub fn rasterize_svg_rgba(
    source: &str,
    native_w: f32,
    native_h: f32,
    target_w: u32,
    target_h: u32,
) -> Option<(Vec<u8>, u32, u32)> {
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
    Some((pixmap.take(), render_w, render_h))
}

/// Rasterize an SVG source string to a GPU texture at `target_w × target_h`
/// pixels, preserving the native aspect ratio. Returns the texture and the
/// actual pixel size used.
pub fn rasterize_svg(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    source: &str,
    native_w: f32,
    native_h: f32,
    target_w: u32,
    target_h: u32,
) -> Option<(GpuTexture, u32, u32)> {
    let (rgba, w, h) = rasterize_svg_rgba(source, native_w, native_h, target_w, target_h)?;
    let tex = tex_pass.upload(gpu, &rgba, w, h);
    Some((tex, w, h))
}

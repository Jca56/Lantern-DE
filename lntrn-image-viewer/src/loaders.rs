//! Decoding images into GPU textures: raster (EXIF + orientation aware,
//! downscaled past the GPU's texture limit) and SVG (kept re-rasterizable).
//! Animated GIFs stream from gif.rs. Lives apart from app.rs so the viewer
//! state file stays about state, not codecs.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, Limits};
use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use crate::app::is_svg;

/// A decoded raster image on the GPU plus the metadata the info overlay wants.
pub struct RasterLoad {
    pub tex: GpuTexture,
    /// The picture's real size. The texture is smaller when the picture is
    /// bigger than the GPU allows.
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

/// A loaded SVG: its source (kept for re-rasterizing at the display size)
/// and native size in SVG user units.
pub struct SvgLoad {
    pub source: String,
    pub native_w: f32,
    pub native_h: f32,
}

/// The largest texture side this GPU takes. Anything bigger fails wgpu
/// validation, which aborts the process.
pub fn max_texture_dim(gpu: &GpuContext) -> u32 {
    gpu.device.limits().max_texture_dimension_2d
}

/// Read just the header of an image to get its dimensions without a full
/// decode, as it will be shown (a portrait phone photo stored sideways with
/// an EXIF rotation reports portrait). Picks the initial window size and the
/// size of images placed on a canvas.
pub fn peek_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    if is_svg(path) {
        let data = std::fs::read_to_string(path).ok()?;
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb = svg_font_database();
        let tree = resvg::usvg::Tree::from_str(&data, &opt).ok()?;
        let s = tree.size();
        Some((s.width().ceil() as u32, s.height().ceil() as u32))
    } else {
        let mut decoder = image::ImageReader::open(path)
            .ok()?
            .with_guessed_format()
            .ok()?
            .into_decoder()
            .ok()?;
        let (w, h) = decoder.dimensions();
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        Some(if swaps_axes(orientation) { (h, w) } else { (w, h) })
    }
}

/// Orientations that turn the picture a quarter, trading width for height.
fn swaps_axes(o: Orientation) -> bool {
    matches!(
        o,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
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
/// `limits` caps what a decode may allocate (the image crate's defaults
/// otherwise). Every raster path in the app goes through here so the viewer,
/// the browser thumbnails and the canvas all agree on which way is up.
pub fn decode_oriented(path: &Path, limits: Option<Limits>) -> Option<DecodedRaster> {
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    if let Some(limits) = limits {
        reader.limits(limits);
    }
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

pub fn decode_raster(path: &Path) -> Option<DecodedRaster> {
    decode_oriented(path, None)
}

pub fn load_raster_texture(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
) -> Option<RasterLoad> {
    let DecodedRaster {
        image,
        format,
        exif,
    } = decode_raster(path)?;
    let (width, height) = (image.width(), image.height());
    let rgba = fit_texture(image, max_texture_dim(gpu)).to_rgba8();
    let tex = tex_pass.upload(gpu, &rgba, rgba.width(), rgba.height());
    Some(RasterLoad {
        tex,
        width,
        height,
        format,
        exif,
    })
}

/// Shrink a picture that's past the GPU's texture limit (panoramas, big
/// scans) to fit it, aspect kept. Layout still uses the real size, so only
/// deep zoom loses detail. Anything that fits is returned untouched.
fn fit_texture(image: DynamicImage, max_dim: u32) -> DynamicImage {
    if image.width() > max_dim || image.height() > max_dim {
        image.resize(max_dim, max_dim, FilterType::Triangle)
    } else {
        image
    }
}

// ── SVG ─────────────────────────────────────────────────────────────────────

/// Pixel size to rasterize an SVG at: `scale` × its native size, shrunk
/// uniformly to fit `max_dim` on both axes so the aspect never changes.
pub fn svg_raster_size(native_w: f32, native_h: f32, scale: f32, max_dim: u32) -> (u32, u32) {
    let (nw, nh) = (native_w.max(1e-3), native_h.max(1e-3));
    let max = max_dim as f32;
    let k = scale.min(max / nw).min(max / nh);
    let w = (nw * k).round().clamp(1.0, max) as u32;
    let h = (nh * k).round().clamp(1.0, max) as u32;
    (w, h)
}

/// Initial SVG load: rasterize at native size (capped) and keep the source
/// around so it can be re-rasterized larger on demand (see
/// `App::maybe_rerender_svg`).
pub fn load_svg_texture(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    path: &Path,
) -> Option<(GpuTexture, SvgLoad)> {
    let source = std::fs::read_to_string(path).ok()?;
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb = svg_font_database();
    let tree = resvg::usvg::Tree::from_str(&source, &opt).ok()?;
    let size = tree.size();
    let (native_w, native_h) = (size.width(), size.height());

    let (w, h) = svg_raster_size(native_w, native_h, 1.0, max_texture_dim(gpu));
    let tex = rasterize_svg(gpu, tex_pass, &source, native_w, native_h, w, h)?;
    let svg = SvgLoad {
        source,
        native_w,
        native_h,
    };
    Some((tex, svg))
}

/// Rasterize an SVG source string to RGBA pixels at `target_w × target_h`,
/// stretching the native box to fit (callers keep the aspect ratio, see
/// `svg_raster_size`).
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

    let (w, h) = (target_w.max(1), target_h.max(1));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        w as f32 / native_w.max(1e-3),
        h as f32 / native_h.max(1e-3),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some((pixmap.take(), w, h))
}

/// Rasterize an SVG source string straight to a GPU texture.
pub fn rasterize_svg(
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    source: &str,
    native_w: f32,
    native_h: f32,
    target_w: u32,
    target_h: u32,
) -> Option<GpuTexture> {
    let (rgba, w, h) = rasterize_svg_rgba(source, native_w, native_h, target_w, target_h)?;
    Some(tex_pass.upload(gpu, &rgba, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_rasters_shrink_to_the_texture_limit() {
        let wide = DynamicImage::new_rgb8(9000, 900);
        let fitted = fit_texture(wide, 8192);
        assert_eq!((fitted.width(), fitted.height()), (8192, 819));
        let tall = fit_texture(DynamicImage::new_rgb8(100, 10_000), 8192);
        assert_eq!((tall.width(), tall.height()), (82, 8192));
        let small = fit_texture(DynamicImage::new_rgb8(640, 480), 8192);
        assert_eq!((small.width(), small.height()), (640, 480));
    }

    #[test]
    fn svg_size_keeps_aspect_under_the_cap() {
        // Wider than the GPU allows: shrunk uniformly, not squashed per axis.
        assert_eq!(svg_raster_size(10_000.0, 100.0, 1.0, 8192), (8192, 82));
        // Zoomed far into a 2:1 icon: both axes stop together at the cap.
        assert_eq!(svg_raster_size(512.0, 256.0, 40.0, 8192), (8192, 4096));
        assert_eq!(svg_raster_size(512.0, 256.0, 2.0, 8192), (1024, 512));
        // Degenerate sizes still give a drawable texture.
        assert_eq!(svg_raster_size(0.2, 50_000.0, 1.0, 8192), (1, 8192));
    }
}

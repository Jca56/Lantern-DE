//! GPU texture cache for canvas items, keyed by source path.
//!
//! Two items with the same source share one texture (`TextureDraw` borrows).
//! Decodes are capped at `IMPORT_MAX_DIM` on the longest axis — the document
//! keeps natural dimensions, so a future full-res export can re-decode.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

const IMPORT_MAX_DIM: u32 = 2048;

pub enum TexEntry {
    Loaded(GpuTexture),
    /// Source file unreadable — rendered as a placeholder.
    Missing,
}

#[derive(Default)]
pub struct CanvasTexCache {
    map: HashMap<String, TexEntry>,
}

impl CanvasTexCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_load(
        &mut self,
        path: &str,
        gpu: &GpuContext,
        tex_pass: &TexturePass,
    ) -> &TexEntry {
        self.map.entry(path.to_string()).or_insert_with(|| {
            match load_capped(Path::new(path), gpu, tex_pass) {
                Some(tex) => TexEntry::Loaded(tex),
                None => TexEntry::Missing,
            }
        })
    }

    /// Immutable lookup — call `get_or_load` for every needed path first,
    /// then borrow entries for `TextureDraw`s through this.
    pub fn get(&self, path: &str) -> Option<&TexEntry> {
        self.map.get(path)
    }

    /// Drop textures for paths no longer referenced by any item.
    pub fn evict_not_in(&mut self, active: &HashSet<&str>) {
        self.map.retain(|k, _| active.contains(k.as_str()));
    }

    /// Forget everything (document switch).
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// Decode an image (raster or SVG; GIFs yield their first frame) downscaled to
/// at most `IMPORT_MAX_DIM` on the longest axis, and upload it.
fn load_capped(path: &Path, gpu: &GpuContext, tex_pass: &TexturePass) -> Option<GpuTexture> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));

    let (rgba, w, h) = if is_svg {
        rasterize_svg_capped(path)?
    } else {
        let mut reader = image::ImageReader::open(path)
            .ok()?
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        let img = reader.decode().ok()?;
        let img = if img.width() > IMPORT_MAX_DIM || img.height() > IMPORT_MAX_DIM {
            img.thumbnail(IMPORT_MAX_DIM, IMPORT_MAX_DIM)
        } else {
            img
        };
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        (rgba.into_raw(), w, h)
    };

    Some(tex_pass.upload(gpu, &rgba, w, h))
}

fn rasterize_svg_capped(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let data = std::fs::read_to_string(path).ok()?;
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb = crate::app::svg_font_database();
    let tree = resvg::usvg::Tree::from_str(&data, &opt).ok()?;
    let size = tree.size();
    let (nw, nh) = (size.width().max(1.0), size.height().max(1.0));
    let scale = (IMPORT_MAX_DIM as f32 / nw)
        .min(IMPORT_MAX_DIM as f32 / nh)
        .min(1.0)
        .max(0.01);
    // Render at native size when it fits, else capped — items scale on canvas.
    let w = ((nw * scale).ceil() as u32).max(1);
    let h = ((nh * scale).ceil() as u32).max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(w as f32 / nw, h as f32 / nh);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some((pixmap.take(), w, h))
}

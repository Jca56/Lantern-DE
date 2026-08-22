//! `COLR` v0 + `CPAL` — layered color glyphs.
//!
//! A base glyph maps to a run of (layer glyph, palette color) records; each
//! layer rasterizes through the normal outline path and composites bottom-up
//! into an RGBA bitmap. COLRv1 paint graphs (gradients/transforms) are not
//! supported — v1-only faces are excluded at scan time, matching the old
//! stack's eviction of faces it couldn't rasterize.

use super::sfnt::{read_u16_at, read_u32_at};
use super::Font;
use crate::raster::{self, RasterRgba};

pub(crate) struct Colr {
    colr: usize,
    cpal: usize,
}

impl Colr {
    /// Only v0 layer lists are consumed (a v1 table's v0-compatible records,
    /// when present, still work).
    pub fn parse(d: &[u8], colr: usize, cpal: usize) -> Option<Colr> {
        read_u16_at(d, colr).ok()?; // header sanity
        read_u16_at(d, cpal).ok()?;
        Some(Colr { colr, cpal })
    }

    /// The (layer gid, RGBA straight color) list for `gid`, bottom-up.
    fn layers(&self, d: &[u8], gid: u16) -> Option<Vec<(u16, [u8; 4])>> {
        let num_base = read_u16_at(d, self.colr + 2).ok()?;
        let base_off = self.colr + read_u32_at(d, self.colr + 4).ok()? as usize;
        let layers_off = self.colr + read_u32_at(d, self.colr + 8).ok()? as usize;

        let (mut lo, mut hi) = (0usize, num_base as usize);
        let mut hit = None;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let g = read_u16_at(d, base_off + mid * 6).ok()?;
            match g.cmp(&gid) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    hit = Some(mid);
                    break;
                }
            }
        }
        let rec = base_off + hit? * 6;
        let first = read_u16_at(d, rec + 2).ok()? as usize;
        let count = read_u16_at(d, rec + 4).ok()? as usize;

        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let layer = layers_off + (first + i) * 4;
            let layer_gid = read_u16_at(d, layer).ok()?;
            let palette_index = read_u16_at(d, layer + 2).ok()?;
            out.push((layer_gid, self.color(d, palette_index)));
        }
        Some(out)
    }

    /// Palette 0 lookup; 0xFFFF = "current text color" → white (color quads
    /// aren't tinted by the text color).
    fn color(&self, d: &[u8], index: u16) -> [u8; 4] {
        if index == 0xFFFF {
            return [255, 255, 255, 255];
        }
        let read = || -> Option<[u8; 4]> {
            let records = self.cpal + read_u32_at(d, self.cpal + 8).ok()? as usize;
            let first = read_u16_at(d, self.cpal + 12).ok()?; // palette 0 start index
            let rec = records + (first + index) as usize * 4;
            let b = super::sfnt::read_u8_at(d, rec).ok()?;
            let g = super::sfnt::read_u8_at(d, rec + 1).ok()?;
            let r = super::sfnt::read_u8_at(d, rec + 2).ok()?;
            let a = super::sfnt::read_u8_at(d, rec + 3).ok()?;
            Some([r, g, b, a])
        };
        read().unwrap_or([255, 255, 255, 255])
    }
}

/// Rasterize a COLRv0 glyph: union the layer bounds, rasterize each layer's
/// outline, composite tinted coverage bottom-up.
pub(crate) fn rasterize(font: &Font, gid: u16, px: f32) -> Option<RasterRgba> {
    let colr = font.colr.as_ref()?;
    let layers = colr.layers(&font.data, gid)?;
    if layers.is_empty() {
        return None;
    }
    let scale = font.scale(px);
    let mut rendered: Vec<(raster::RasterGlyph, [u8; 4])> = Vec::new();
    for (layer_gid, color) in layers {
        let Ok(outline) = font.outline(layer_gid) else {
            continue;
        };
        if let Some(glyph) = raster::rasterize(&outline, scale, 0.0) {
            rendered.push((glyph, color));
        }
    }
    if rendered.is_empty() {
        return None;
    }

    // Union frame in "left/top bearing" space.
    let left = rendered.iter().map(|(g, _)| g.left).min()?;
    let top = rendered.iter().map(|(g, _)| g.top).max()?;
    let right = rendered
        .iter()
        .map(|(g, _)| g.left + g.width as i32)
        .max()?;
    let bottom = rendered
        .iter()
        .map(|(g, _)| g.top - g.height as i32)
        .min()?;
    let width = (right - left).max(1) as u32;
    let height = (top - bottom).max(1) as u32;

    let placed: Vec<(raster::RasterGlyph, [u8; 4], i32, i32)> = rendered
        .into_iter()
        .map(|(g, c)| {
            let ox = g.left - left;
            let oy = top - g.top;
            (g, c, ox, oy)
        })
        .collect();
    let rgba = raster::composite_layers(width, height, &placed);
    Some(RasterRgba {
        width,
        height,
        left,
        top,
        rgba,
    })
}

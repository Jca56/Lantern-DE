//! `CBLC` + `CBDT` — color bitmap (PNG) emoji strikes.
//!
//! CBLC indexes fixed-ppem "strikes"; CBDT holds the glyph images. Google's
//! emoji fonts use index formats 1/2/3 with PNG image formats 17/18/19 — all
//! implemented here. The best strike for a target pixel size is decoded,
//! then downscaled by the caller.

use super::sfnt::{read_u16_at, read_u32_at, read_u8_at};
use crate::raster::{self, png, RasterRgba};

pub(crate) struct Cbdt {
    cbdt: usize,
    strikes: Vec<Strike>,
}

struct Strike {
    /// Absolute offset of the IndexSubTableArray.
    index_array: usize,
    subtable_count: u32,
    ppem: u8,
}

struct GlyphImage {
    /// Absolute byte range of the image data in CBDT.
    range: (usize, usize),
    image_format: u16,
    /// Metrics from the index (formats 2/5), when per-glyph metrics are
    /// absent from the image data.
    index_metrics: Option<Metrics>,
}

/// Placement metrics; PNG dimensions drive the bitmap size, so the
/// declared width/height are informational.
#[allow(dead_code)]
#[derive(Clone, Copy)]
struct Metrics {
    width: u8,
    height: u8,
    bearing_x: i8,
    bearing_y: i8,
}

impl Cbdt {
    pub fn parse(d: &[u8], cblc: usize, cbdt: usize) -> Option<Cbdt> {
        let num_sizes = read_u32_at(d, cblc + 4).ok()? as usize;
        let mut strikes = Vec::with_capacity(num_sizes);
        for i in 0..num_sizes.min(32) {
            let rec = cblc + 8 + i * 48;
            let index_array = cblc + read_u32_at(d, rec).ok()? as usize;
            let subtable_count = read_u32_at(d, rec + 8).ok()?;
            let ppem = read_u8_at(d, rec + 45).ok()?; // ppemY
            strikes.push(Strike {
                index_array,
                subtable_count,
                ppem,
            });
        }
        if strikes.is_empty() {
            return None;
        }
        Some(Cbdt { cbdt, strikes })
    }

    /// Decode `gid` at the best strike for `px`, scaled to the target size.
    pub fn glyph(&self, d: &[u8], gid: u16, px: f32) -> Option<RasterRgba> {
        // Smallest strike that's >= the target (sharpest downscale), else
        // the largest available.
        let strike = self
            .strikes
            .iter()
            .filter(|s| s.ppem as f32 >= px)
            .min_by_key(|s| s.ppem)
            .or_else(|| self.strikes.iter().max_by_key(|s| s.ppem))?;

        let image = self.find_glyph(d, strike, gid)?;
        let (start, end) = image.range;
        let data = d.get(start..end)?;

        // Image-format header: metrics + PNG payload.
        let (metrics, png_off) = match image.image_format {
            17 => {
                let m = Metrics {
                    height: read_u8_at(data, 0).ok()?,
                    width: read_u8_at(data, 1).ok()?,
                    bearing_x: read_u8_at(data, 2).ok()? as i8,
                    bearing_y: read_u8_at(data, 3).ok()? as i8,
                };
                (Some(m), 9) // smallGlyphMetrics(5) + dataLen(4)
            }
            18 => {
                let m = Metrics {
                    height: read_u8_at(data, 0).ok()?,
                    width: read_u8_at(data, 1).ok()?,
                    bearing_x: read_u8_at(data, 2).ok()? as i8,
                    bearing_y: read_u8_at(data, 3).ok()? as i8,
                };
                (Some(m), 12) // bigGlyphMetrics(8) + dataLen(4)
            }
            19 => (image.index_metrics, 4), // dataLen(4)
            _ => return None,
        };
        let img = png::decode(data.get(png_off..)?)?;
        let metrics = metrics.unwrap_or(Metrics {
            width: img.width.min(255) as u8,
            height: img.height.min(255) as u8,
            bearing_x: 0,
            bearing_y: img.height.min(127) as i8,
        });

        // Scale strike space → target pixels.
        let scale = px / strike.ppem as f32;
        let out_w = ((img.width as f32 * scale).round() as u32).max(1);
        let out_h = ((img.height as f32 * scale).round() as u32).max(1);
        let rgba = if out_w == img.width && out_h == img.height {
            img.rgba.clone()
        } else {
            raster::resize_rgba(&img, out_w, out_h)
        };
        Some(RasterRgba {
            width: out_w,
            height: out_h,
            left: (metrics.bearing_x as f32 * scale).round() as i32,
            top: (metrics.bearing_y as f32 * scale).round() as i32,
            rgba,
        })
    }

    fn find_glyph(&self, d: &[u8], strike: &Strike, gid: u16) -> Option<GlyphImage> {
        for i in 0..strike.subtable_count as usize {
            let rec = strike.index_array + i * 8;
            let first = read_u16_at(d, rec).ok()?;
            let last = read_u16_at(d, rec + 2).ok()?;
            if gid < first || gid > last {
                continue;
            }
            let sub = strike.index_array + read_u32_at(d, rec + 4).ok()? as usize;
            let index_format = read_u16_at(d, sub).ok()?;
            let image_format = read_u16_at(d, sub + 2).ok()?;
            let image_data = self.cbdt + read_u32_at(d, sub + 4).ok()? as usize;
            let rel = (gid - first) as usize;
            return match index_format {
                1 => {
                    let a = read_u32_at(d, sub + 8 + rel * 4).ok()? as usize;
                    let b = read_u32_at(d, sub + 8 + rel * 4 + 4).ok()? as usize;
                    (a < b).then(|| GlyphImage {
                        range: (image_data + a, image_data + b),
                        image_format,
                        index_metrics: None,
                    })
                }
                2 => {
                    let size = read_u32_at(d, sub + 8).ok()? as usize;
                    let metrics = big_metrics(d, sub + 12)?;
                    Some(GlyphImage {
                        range: (image_data + rel * size, image_data + (rel + 1) * size),
                        image_format,
                        index_metrics: Some(metrics),
                    })
                }
                3 => {
                    let a = read_u16_at(d, sub + 8 + rel * 2).ok()? as usize;
                    let b = read_u16_at(d, sub + 8 + rel * 2 + 2).ok()? as usize;
                    (a < b).then(|| GlyphImage {
                        range: (image_data + a, image_data + b),
                        image_format,
                        index_metrics: None,
                    })
                }
                _ => None,
            };
        }
        None
    }
}

fn big_metrics(d: &[u8], pos: usize) -> Option<Metrics> {
    Some(Metrics {
        height: read_u8_at(d, pos).ok()?,
        width: read_u8_at(d, pos + 1).ok()?,
        bearing_x: read_u8_at(d, pos + 2).ok()? as i8,
        bearing_y: read_u8_at(d, pos + 3).ok()? as i8,
    })
}

#[cfg(test)]
mod tests {
    use crate::font::Font;

    /// Decode a real emoji from the bundled CBDT font: PNG decoder + CBLC
    /// index + strike scaling, end to end. Skips if the font is absent.
    #[test]
    fn decodes_bundled_emoji() {
        let home = std::env::var("HOME").unwrap_or_default();
        let Ok(data) = std::fs::read(format!("{home}/.lantern/fonts/NotoColorEmoji.ttf")) else {
            eprintln!("no bundled emoji font — skipping CBDT test");
            return;
        };
        let font = Font::parse(data, 0).expect("CBDT font should parse");
        let gid = font.glyph_index('🦊');
        assert_ne!(gid, 0, "fox emoji should map");
        let glyph = font
            .color_glyph(gid, 32.0)
            .expect("CBDT strike should decode");
        assert!(
            glyph.width >= 24 && glyph.width <= 48,
            "unexpected size {}",
            glyph.width
        );
        // Must be chromatic (the fox is orange) with real alpha coverage.
        let chroma = glyph
            .rgba
            .chunks_exact(4)
            .filter(|px| px[3] > 128 && (px[0] as i32 - px[2] as i32).abs() > 30)
            .count();
        assert!(
            chroma > 50,
            "fox should be orange, got {chroma} chromatic px"
        );
    }
}

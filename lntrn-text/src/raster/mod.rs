//! Rasterization: outline → anti-aliased coverage bitmap, plus the color
//! (RGBA) glyph helpers for emoji — PNG decoding, downscaling, and COLR
//! layer compositing.

pub mod outline;
pub(crate) mod png;
mod scanline;

use outline::{Affine, Outline, PathCmd};
use scanline::Accumulator;

/// A color (RGBA, straight-alpha) glyph bitmap with placement metrics.
pub(crate) struct RasterRgba {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub rgba: Vec<u8>,
}

/// Area-average downscale (box filter) — emoji strikes are 128px+ and render
/// at ~16–48px, where a box filter beats bilinear.
pub(crate) fn resize_rgba(src: &png::Image, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (dw, dh) = (dst_w.max(1) as usize, dst_h.max(1) as usize);
    let mut out = vec![0u8; dw * dh * 4];
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    for dy in 0..dh {
        let y0 = (dy as f32 * fy) as usize;
        let y1 = (((dy + 1) as f32 * fy).ceil() as usize).clamp(y0 + 1, sh);
        for dx in 0..dw {
            let x0 = (dx as f32 * fx) as usize;
            let x1 = (((dx + 1) as f32 * fx).ceil() as usize).clamp(x0 + 1, sw);
            // Alpha-weighted color average avoids dark fringes at edges.
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = (sy * sw + sx) * 4;
                    let pa = src.rgba[p + 3] as u32;
                    r += src.rgba[p] as u32 * pa;
                    g += src.rgba[p + 1] as u32 * pa;
                    b += src.rgba[p + 2] as u32 * pa;
                    a += pa;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (dy * dw + dx) * 4;
            if let (Some(cr), Some(cg), Some(cb)) =
                (r.checked_div(a), g.checked_div(a), b.checked_div(a))
            {
                out[o] = cr as u8;
                out[o + 1] = cg as u8;
                out[o + 2] = cb as u8;
                out[o + 3] = (a / n) as u8;
            }
        }
    }
    out
}

/// Composite COLR layers (coverage + straight-alpha color each) bottom-up
/// into one straight-alpha RGBA bitmap. Layers share the same bitmap frame.
pub(crate) fn composite_layers(
    width: u32,
    height: u32,
    layers: &[(RasterGlyph, [u8; 4], i32, i32)],
) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    // Accumulate premultiplied, output straight.
    let mut acc = vec![0.0f32; w * h * 4];
    for (glyph, color, ox, oy) in layers {
        let (cr, cg, cb) = (
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
        );
        let ca = color[3] as f32 / 255.0;
        for gy in 0..glyph.height as usize {
            let ty = gy as i32 + oy;
            if ty < 0 || ty >= h as i32 {
                continue;
            }
            for gx in 0..glyph.width as usize {
                let tx = gx as i32 + ox;
                if tx < 0 || tx >= w as i32 {
                    continue;
                }
                let cov = glyph.coverage[gy * glyph.width as usize + gx] as f32 / 255.0;
                let sa = cov * ca;
                if sa <= 0.0 {
                    continue;
                }
                let o = (ty as usize * w + tx as usize) * 4;
                let inv = 1.0 - sa;
                acc[o] = cr * sa + acc[o] * inv;
                acc[o + 1] = cg * sa + acc[o + 1] * inv;
                acc[o + 2] = cb * sa + acc[o + 2] * inv;
                acc[o + 3] = sa + acc[o + 3] * inv;
            }
        }
    }
    let mut out = vec![0u8; w * h * 4];
    for (dst, src) in out.chunks_exact_mut(4).zip(acc.chunks_exact(4)) {
        let a = src[3];
        if a > 0.0 {
            dst[0] = (src[0] / a * 255.0 + 0.5).min(255.0) as u8;
            dst[1] = (src[1] / a * 255.0 + 0.5).min(255.0) as u8;
            dst[2] = (src[2] / a * 255.0 + 0.5).min(255.0) as u8;
            dst[3] = (a * 255.0 + 0.5).min(255.0) as u8;
        }
    }
    out
}

/// A rasterized glyph ready for atlas insertion.
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    /// Pixels from the pen origin to the bitmap's left edge.
    pub left: i32,
    /// Pixels from the baseline up to the bitmap's top edge.
    pub top: i32,
    pub coverage: Vec<u8>,
}

/// Safety valve against corrupt outlines producing absurd bitmaps.
const MAX_GLYPH_PX: f32 = 4096.0;

/// Rasterize `outline` (font units, y up) at `scale` px-per-unit into a tight
/// coverage bitmap. `x_offset` (0..1 px) bakes a subpixel horizontal position
/// into the coverage — the engine quantizes pen positions to quarter-pixel
/// bins so proportional text keeps even spacing instead of snapping each
/// glyph to whole pixels. Returns `None` for empty/degenerate outlines.
pub fn rasterize(outline: &Outline, scale: f32, x_offset: f32) -> Option<RasterGlyph> {
    if outline.cmds.is_empty() || scale <= 0.0 {
        return None;
    }

    // Pixel-space bounds from the transformed control points. Béziers stay
    // inside their control hull, so this is conservative and tight enough.
    let mut min = [f32::MAX, f32::MAX];
    let mut max = [f32::MIN, f32::MIN];
    {
        let mut see = |p: &[f32; 2]| {
            let x = p[0] * scale + x_offset;
            let y = -p[1] * scale;
            min[0] = min[0].min(x);
            min[1] = min[1].min(y);
            max[0] = max[0].max(x);
            max[1] = max[1].max(y);
        };
        for cmd in &outline.cmds {
            match cmd {
                PathCmd::Move(p) | PathCmd::Line(p) => see(p),
                PathCmd::Quad(c, p) => {
                    see(c);
                    see(p);
                }
                PathCmd::Cubic(c1, c2, p) => {
                    see(c1);
                    see(c2);
                    see(p);
                }
            }
        }
    }
    if !(min[0].is_finite() && min[1].is_finite() && max[0].is_finite() && max[1].is_finite()) {
        return None;
    }

    let x0 = min[0].floor();
    let y0 = min[1].floor();
    let w = (max[0].ceil() - x0).max(0.0);
    let h = (max[1].ceil() - y0).max(0.0);
    if w < 1.0 || h < 1.0 {
        return None; // zero-area ink (degenerate outline)
    }
    if w > MAX_GLYPH_PX || h > MAX_GLYPH_PX {
        eprintln!("[lntrn-text] refusing to rasterize {w}x{h}px glyph (corrupt outline?)");
        return None;
    }
    let (w, h) = (w as usize, h as usize);

    // Scale + y-flip + subpixel shift + translate into bitmap space.
    let t = Affine {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: -scale,
        e: x_offset - x0,
        f: -y0,
    };
    let mut acc = Accumulator::new(w, h);
    outline::flatten(&outline.cmds, &t, |p0, p1| acc.line(p0, p1));

    Some(RasterGlyph {
        width: w as u32,
        height: h as u32,
        left: x0 as i32,
        top: -y0 as i32,
        coverage: acc.finish(),
    })
}

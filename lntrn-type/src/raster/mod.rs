//! Rasterization: outline → anti-aliased coverage bitmap.

pub mod outline;
mod scanline;

use outline::{Affine, Outline, PathCmd};
use scanline::Accumulator;

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
/// coverage bitmap. Returns `None` for empty/degenerate outlines (e.g. space).
pub fn rasterize(outline: &Outline, scale: f32) -> Option<RasterGlyph> {
    if outline.cmds.is_empty() || scale <= 0.0 {
        return None;
    }

    // Pixel-space bounds from the transformed control points. Béziers stay
    // inside their control hull, so this is conservative and tight enough.
    let mut min = [f32::MAX, f32::MAX];
    let mut max = [f32::MIN, f32::MIN];
    {
        let mut see = |p: &[f32; 2]| {
            let x = p[0] * scale;
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
        eprintln!("[lntrn-type] refusing to rasterize {w}x{h}px glyph (corrupt outline?)");
        return None;
    }
    let (w, h) = (w as usize, h as usize);

    // Scale + y-flip + translate into bitmap space in one transform.
    let t = Affine {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: -scale,
        e: -x0,
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

//! `glyf` + `loca` — TrueType glyph outlines.
//!
//! Decodes simple glyphs (flag-compressed point arrays with implied on-curve
//! midpoints) and composite glyphs (nested components with F2.14 affine
//! transforms) into [`Outline`] path commands in font units.

use super::sfnt::{read_u16_at, read_u32_at, Reader};
use super::{Font, FontError};
use crate::raster::outline::{Affine, Outline, PathCmd};

/// Composite glyphs referencing composites; 8 levels is far beyond real fonts.
const MAX_DEPTH: u8 = 8;

// Simple-glyph point flags.
const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME_OR_POS: u8 = 0x10;
const Y_SAME_OR_POS: u8 = 0x20;

// Composite component flags.
const ARGS_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY: u16 = 0x0002;
const HAS_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const HAS_XY_SCALE: u16 = 0x0040;
const HAS_2X2: u16 = 0x0080;

pub(crate) fn outline(font: &Font, gid: u16) -> Result<Outline, FontError> {
    let mut out = Outline::default();
    emit_glyph(font, gid, Affine::IDENTITY, 0, &mut out)?;
    Ok(out)
}

/// `loca` lookup → absolute glyph data range, or `None` for empty glyphs.
fn glyph_range(font: &Font, gid: u16) -> Result<Option<(usize, usize)>, FontError> {
    if gid >= font.num_glyphs {
        return Err(FontError::BadGlyph(gid));
    }
    let (loca_off, loca_len) = font.loca;
    let loca = font
        .data
        .get(loca_off..loca_off + loca_len)
        .ok_or(FontError::Truncated)?;
    let i = gid as usize;
    let (start, end) = if font.long_loca {
        (
            read_u32_at(loca, i * 4)? as usize,
            read_u32_at(loca, i * 4 + 4)? as usize,
        )
    } else {
        (
            read_u16_at(loca, i * 2)? as usize * 2,
            read_u16_at(loca, i * 2 + 2)? as usize * 2,
        )
    };
    if start >= end {
        return Ok(None); // no outline (space and friends)
    }
    let (glyf_off, glyf_len) = font.glyf;
    if end > glyf_len {
        return Err(FontError::Truncated);
    }
    Ok(Some((glyf_off + start, glyf_off + end)))
}

fn emit_glyph(
    font: &Font,
    gid: u16,
    t: Affine,
    depth: u8,
    out: &mut Outline,
) -> Result<(), FontError> {
    if depth > MAX_DEPTH {
        return Err(FontError::Unsupported("composite glyph nested too deep"));
    }
    let Some((start, end)) = glyph_range(font, gid)? else {
        return Ok(());
    };
    let mut r = Reader::new(&font.data[start..end]);
    let n_contours = r.i16()?;
    r.skip(8)?; // bounding box — recomputed from actual points at raster time
    if n_contours >= 0 {
        emit_simple(&mut r, n_contours as usize, &t, out)
    } else {
        emit_composite(font, &mut r, &t, depth, out)
    }
}

fn emit_simple(
    r: &mut Reader,
    n_contours: usize,
    t: &Affine,
    out: &mut Outline,
) -> Result<(), FontError> {
    let mut end_pts = Vec::with_capacity(n_contours);
    for _ in 0..n_contours {
        end_pts.push(r.u16()?);
    }
    let n_points = match end_pts.last() {
        Some(&e) => e as usize + 1,
        None => return Ok(()),
    };
    let instruction_len = r.u16()? as usize;
    r.skip(instruction_len)?; // hinting bytecode — Phase 4 territory

    // Flags, with run-length repeats.
    let mut flags = Vec::with_capacity(n_points);
    while flags.len() < n_points {
        let f = r.u8()?;
        flags.push(f);
        if f & REPEAT != 0 {
            let n = r.u8()?;
            for _ in 0..n {
                if flags.len() < n_points {
                    flags.push(f);
                }
            }
        }
    }

    // Delta-compressed coordinates: one full x pass, then one full y pass.
    let mut xs = Vec::with_capacity(n_points);
    let mut x = 0i32;
    for &f in &flags {
        if f & X_SHORT != 0 {
            let d = r.u8()? as i32;
            x += if f & X_SAME_OR_POS != 0 { d } else { -d };
        } else if f & X_SAME_OR_POS == 0 {
            x += r.i16()? as i32;
        }
        xs.push(x);
    }
    let mut ys = Vec::with_capacity(n_points);
    let mut y = 0i32;
    for &f in &flags {
        if f & Y_SHORT != 0 {
            let d = r.u8()? as i32;
            y += if f & Y_SAME_OR_POS != 0 { d } else { -d };
        } else if f & Y_SAME_OR_POS == 0 {
            y += r.i16()? as i32;
        }
        ys.push(y);
    }

    let mut begin = 0usize;
    for &e in &end_pts {
        let end = e as usize + 1;
        if end > n_points || begin >= end {
            return Err(FontError::Truncated); // endPts must ascend
        }
        emit_contour(&flags[begin..end], &xs[begin..end], &ys[begin..end], t, out);
        begin = end;
    }
    Ok(())
}

/// One closed contour → path commands. Off-curve runs imply on-curve midpoints
/// between consecutive control points; a contour may even start off-curve, in
/// which case the start point is borrowed from the end or synthesized.
fn emit_contour(flags: &[u8], xs: &[i32], ys: &[i32], t: &Affine, out: &mut Outline) {
    let n = flags.len();
    if n < 2 {
        return;
    }
    let on = |i: usize| flags[i] & ON_CURVE != 0;
    let pt = |i: usize| (xs[i] as f32, ys[i] as f32);
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);

    let (start, lo, hi) = if on(0) {
        (pt(0), 1, n)
    } else if on(n - 1) {
        (pt(n - 1), 0, n - 1)
    } else {
        (mid(pt(0), pt(n - 1)), 0, n)
    };
    out.cmds.push(PathCmd::Move(t.apply(start)));

    let mut ctrl: Option<(f32, f32)> = None;
    for i in lo..hi {
        let p = pt(i);
        if on(i) {
            match ctrl.take() {
                Some(c) => out.cmds.push(PathCmd::Quad(t.apply(c), t.apply(p))),
                None => out.cmds.push(PathCmd::Line(t.apply(p))),
            }
        } else if let Some(c) = ctrl.replace(p) {
            out.cmds.push(PathCmd::Quad(t.apply(c), t.apply(mid(c, p))));
        }
    }
    match ctrl {
        Some(c) => out.cmds.push(PathCmd::Quad(t.apply(c), t.apply(start))),
        None => out.cmds.push(PathCmd::Line(t.apply(start))),
    }
}

fn emit_composite(
    font: &Font,
    r: &mut Reader,
    parent: &Affine,
    depth: u8,
    out: &mut Outline,
) -> Result<(), FontError> {
    loop {
        let flags = r.u16()?;
        let child = r.u16()?;
        let (a1, a2) = if flags & ARGS_ARE_WORDS != 0 {
            (r.i16()? as i32, r.i16()? as i32)
        } else {
            (r.u8()? as i8 as i32, r.u8()? as i8 as i32)
        };
        let (mut a, mut b, mut c, mut d) = (1.0f32, 0.0f32, 0.0f32, 1.0f32);
        if flags & HAS_SCALE != 0 {
            let s = r.f2dot14()?;
            a = s;
            d = s;
        } else if flags & HAS_XY_SCALE != 0 {
            a = r.f2dot14()?;
            d = r.f2dot14()?;
        } else if flags & HAS_2X2 != 0 {
            a = r.f2dot14()?;
            b = r.f2dot14()?;
            c = r.f2dot14()?;
            d = r.f2dot14()?;
        }
        if flags & ARGS_ARE_XY != 0 {
            // Offsets are unscaled font units by default (the MS convention;
            // SCALED_COMPONENT_OFFSET is vanishingly rare in real fonts).
            let local = Affine { a, b, c, d, e: a1 as f32, f: a2 as f32 };
            emit_glyph(font, child, local.then(parent), depth + 1, out)?;
        } else {
            // Point-matching anchors: essentially unused by modern fonts.
            eprintln!(
                "[lntrn-type] skipping point-matched composite component (glyph {child})"
            );
        }
        if flags & MORE_COMPONENTS == 0 {
            return Ok(());
        }
    }
}

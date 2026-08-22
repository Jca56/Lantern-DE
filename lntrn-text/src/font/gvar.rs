//! `gvar` — TrueType outline variation deltas.
//!
//! For a glyph at a normalized design-space position, sums the scaled deltas
//! of every applicable tuple-variation region onto the glyph's points:
//! shared/embedded peak tuples, intermediate regions, packed point numbers,
//! packed deltas, and IUP (interpolation of untouched points) per contour —
//! applied per tuple before accumulation, as the spec requires. Composite
//! glyphs get their component offsets varied the same way.

use super::sfnt::{read_i16_at, read_u16_at, read_u32_at, read_u8_at};
use super::variations::axis_scalar;

pub(crate) struct Gvar {
    pub axis_count: usize,
    shared_count: u16,
    shared_tuples: usize,
    glyph_count: u16,
    long_offsets: bool,
    /// Absolute offset of the per-glyph offset array.
    offsets: usize,
    /// Absolute offset of the glyph variation data array.
    data: usize,
}

impl Gvar {
    /// `off` = absolute offset of the gvar table.
    pub fn parse(d: &[u8], off: usize) -> Option<Gvar> {
        let axis_count = read_u16_at(d, off + 4).ok()? as usize;
        let shared_count = read_u16_at(d, off + 6).ok()?;
        let shared_tuples = off + read_u32_at(d, off + 8).ok()? as usize;
        let glyph_count = read_u16_at(d, off + 12).ok()?;
        let flags = read_u16_at(d, off + 14).ok()?;
        let data = off + read_u32_at(d, off + 16).ok()? as usize;
        Some(Gvar {
            axis_count,
            shared_count,
            shared_tuples,
            glyph_count,
            long_offsets: flags & 1 != 0,
            offsets: off + 20,
            data,
        })
    }

    /// Byte range of `gid`'s variation data (None = no variation).
    fn glyph_range(&self, d: &[u8], gid: u16) -> Option<(usize, usize)> {
        if gid >= self.glyph_count {
            return None;
        }
        let (a, b) = if self.long_offsets {
            (
                read_u32_at(d, self.offsets + gid as usize * 4).ok()? as usize,
                read_u32_at(d, self.offsets + gid as usize * 4 + 4).ok()? as usize,
            )
        } else {
            (
                read_u16_at(d, self.offsets + gid as usize * 2).ok()? as usize * 2,
                read_u16_at(d, self.offsets + gid as usize * 2 + 2).ok()? as usize * 2,
            )
        };
        (a < b).then(|| (self.data + a, self.data + b))
    }
}

fn f2dot14(d: &[u8], pos: usize) -> Option<f32> {
    read_i16_at(d, pos).ok().map(|v| v as f32 / 16384.0)
}

/// Apply `gid`'s deltas at `coords` to `points` (which must include the four
/// phantom points at the end). `contour_ends` enables IUP for simple glyphs;
/// pass `None` for composites (unreferenced components shift by zero).
pub(crate) fn apply(
    d: &[u8],
    gvar: &Gvar,
    gid: u16,
    coords: &[f32],
    points: &mut [(f32, f32)],
    contour_ends: Option<&[u16]>,
) {
    let Some((start, _end)) = gvar.glyph_range(d, gid) else {
        return;
    };
    let _ = apply_inner(d, gvar, coords, points, contour_ends, start);
}

fn apply_inner(
    d: &[u8],
    gvar: &Gvar,
    coords: &[f32],
    points: &mut [(f32, f32)],
    contour_ends: Option<&[u16]>,
    start: usize,
) -> Option<()> {
    let n = points.len();
    let original: Vec<(f32, f32)> = points.to_vec();
    let tuple_info = read_u16_at(d, start).ok()?;
    let tuple_count = (tuple_info & 0x0FFF) as usize;
    let serialized = start + read_u16_at(d, start + 2).ok()? as usize;

    // Shared point numbers (used by tuples without private points).
    let mut ser_pos = serialized;
    let shared_points: Option<Option<Vec<u16>>> = if tuple_info & 0x8000 != 0 {
        Some(read_packed_points(d, &mut ser_pos)?)
    } else {
        None
    };

    let mut header = start + 4;
    let mut acc: Vec<(f32, f32)> = vec![(0.0, 0.0); n];

    for _ in 0..tuple_count {
        let data_size = read_u16_at(d, header).ok()? as usize;
        let tuple_index = read_u16_at(d, header + 2).ok()?;
        header += 4;

        // Peak tuple: embedded or shared.
        let mut peaks = Vec::with_capacity(gvar.axis_count);
        if tuple_index & 0x8000 != 0 {
            for a in 0..gvar.axis_count {
                peaks.push(f2dot14(d, header + a * 2)?);
            }
            header += gvar.axis_count * 2;
        } else {
            let idx = (tuple_index & 0x0FFF).min(gvar.shared_count.saturating_sub(1)) as usize;
            let base = gvar.shared_tuples + idx * gvar.axis_count * 2;
            for a in 0..gvar.axis_count {
                peaks.push(f2dot14(d, base + a * 2)?);
            }
        }
        // Intermediate region.
        let (mut starts, mut ends) = (Vec::new(), Vec::new());
        if tuple_index & 0x4000 != 0 {
            for a in 0..gvar.axis_count {
                starts.push(f2dot14(d, header + a * 2)?);
            }
            header += gvar.axis_count * 2;
            for a in 0..gvar.axis_count {
                ends.push(f2dot14(d, header + a * 2)?);
            }
            header += gvar.axis_count * 2;
        }

        // Region scalar at coords.
        let mut scalar = 1.0f32;
        for a in 0..gvar.axis_count {
            let peak = peaks[a];
            let coord = coords.get(a).copied().unwrap_or(0.0);
            let (s, e) = if tuple_index & 0x4000 != 0 {
                (starts[a], ends[a])
            } else {
                (peak.min(0.0), peak.max(0.0))
            };
            scalar *= axis_scalar(coord, s, peak, e);
            if scalar == 0.0 {
                break;
            }
        }

        let tuple_data_start = ser_pos;
        ser_pos += data_size;
        if scalar == 0.0 {
            continue;
        }

        // Point list: private, shared, or all points.
        let mut p = tuple_data_start;
        let point_list: Option<Vec<u16>> = if tuple_index & 0x2000 != 0 {
            read_packed_points(d, &mut p)?
        } else {
            match &shared_points {
                Some(sp) => sp.clone(),
                None => None,
            }
        };
        let count = point_list.as_ref().map_or(n, |l| l.len());
        let dx = read_packed_deltas(d, &mut p, count)?;
        let dy = read_packed_deltas(d, &mut p, count)?;

        match &point_list {
            None => {
                // All points explicitly referenced.
                for (i, a) in acc.iter_mut().enumerate().take(count.min(n)) {
                    a.0 += scalar * dx[i] as f32;
                    a.1 += scalar * dy[i] as f32;
                }
            }
            Some(list) => {
                // Sparse: fill via IUP per contour (simple glyphs), zero
                // elsewhere (composites/phantoms).
                let mut tup_dx = vec![0.0f32; n];
                let mut tup_dy = vec![0.0f32; n];
                let mut touched = vec![false; n];
                for (k, &pt) in list.iter().enumerate() {
                    let pt = pt as usize;
                    if pt < n {
                        tup_dx[pt] = dx[k] as f32;
                        tup_dy[pt] = dy[k] as f32;
                        touched[pt] = true;
                    }
                }
                if let Some(ends_list) = contour_ends {
                    iup(&original, ends_list, &touched, &mut tup_dx, true);
                    iup(&original, ends_list, &touched, &mut tup_dy, false);
                }
                for i in 0..n {
                    acc[i].0 += scalar * tup_dx[i];
                    acc[i].1 += scalar * tup_dy[i];
                }
            }
        }
    }

    for (pt, delta) in points.iter_mut().zip(acc) {
        pt.0 += delta.0;
        pt.1 += delta.1;
    }
    Some(())
}

/// Packed point numbers. `None` = "all points".
fn read_packed_points(d: &[u8], pos: &mut usize) -> Option<Option<Vec<u16>>> {
    let b0 = read_u8_at(d, *pos).ok()?;
    *pos += 1;
    let count = if b0 == 0 {
        return Some(None);
    } else if b0 & 0x80 != 0 {
        let b1 = read_u8_at(d, *pos).ok()?;
        *pos += 1;
        (((b0 & 0x7F) as usize) << 8) | b1 as usize
    } else {
        b0 as usize
    };
    let mut out = Vec::with_capacity(count);
    let mut current = 0u16;
    while out.len() < count {
        let control = read_u8_at(d, *pos).ok()?;
        *pos += 1;
        let run = (control & 0x7F) as usize + 1;
        for _ in 0..run.min(count - out.len()) {
            let delta = if control & 0x80 != 0 {
                let v = read_u16_at(d, *pos).ok()?;
                *pos += 2;
                v
            } else {
                let v = read_u8_at(d, *pos).ok()? as u16;
                *pos += 1;
                v
            };
            current = current.wrapping_add(delta);
            out.push(current);
        }
    }
    Some(Some(out))
}

/// Packed deltas: zero runs, byte runs, word runs.
fn read_packed_deltas(d: &[u8], pos: &mut usize, count: usize) -> Option<Vec<i32>> {
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let control = read_u8_at(d, *pos).ok()?;
        *pos += 1;
        let run = (control & 0x3F) as usize + 1;
        for _ in 0..run.min(count - out.len()) {
            if control & 0x80 != 0 {
                out.push(0);
            } else if control & 0x40 != 0 {
                out.push(read_i16_at(d, *pos).ok()? as i32);
                *pos += 2;
            } else {
                out.push(read_u8_at(d, *pos).ok()? as i8 as i32);
                *pos += 1;
            }
        }
    }
    Some(out)
}

/// IUP: interpolate untouched points from touched neighbors within each
/// contour, on one axis (`x_axis` selects coordinate + delta channel).
fn iup(
    original: &[(f32, f32)],
    contour_ends: &[u16],
    touched: &[bool],
    deltas: &mut [f32],
    x_axis: bool,
) {
    let coord = |i: usize| if x_axis { original[i].0 } else { original[i].1 };
    let mut start = 0usize;
    for &e in contour_ends {
        let end = e as usize;
        if end >= touched.len() {
            break;
        }
        let len = end - start + 1;
        let touched_idx: Vec<usize> = (start..=end).filter(|&i| touched[i]).collect();
        match touched_idx.len() {
            0 => {}
            1 => {
                let dv = deltas[touched_idx[0]];
                for i in start..=end {
                    if !touched[i] {
                        deltas[i] = dv;
                    }
                }
            }
            _ => {
                for k in 0..len {
                    let i = start + k;
                    if touched[i] {
                        continue;
                    }
                    // Nearest touched neighbors, cyclically.
                    let prev = (1..=len)
                        .map(|s| start + (k + len - s) % len)
                        .find(|&j| touched[j])
                        .unwrap();
                    let next = (1..=len)
                        .map(|s| start + (k + s) % len)
                        .find(|&j| touched[j])
                        .unwrap();
                    let (c, c1, c2) = (coord(i), coord(prev), coord(next));
                    let (d1, d2) = (deltas[prev], deltas[next]);
                    deltas[i] = if (c1 - c2).abs() < f32::EPSILON {
                        if (d1 - d2).abs() < f32::EPSILON {
                            d1
                        } else {
                            0.0
                        }
                    } else {
                        let (lo_c, lo_d, hi_c, hi_d) = if c1 < c2 {
                            (c1, d1, c2, d2)
                        } else {
                            (c2, d2, c1, d1)
                        };
                        if c <= lo_c {
                            lo_d
                        } else if c >= hi_c {
                            hi_d
                        } else {
                            lo_d + (c - lo_c) / (hi_c - lo_c) * (hi_d - lo_d)
                        }
                    };
                }
            }
        }
        start = end + 1;
    }
}

#[cfg(test)]
mod tests {
    use crate::font::Font;
    use crate::raster;

    /// Instance a real single-file variable font at two weights and verify
    /// the outlines actually differ (gvar deltas applied). Skips when the
    /// machine has no variable font to test with.
    #[test]
    fn variable_font_weights_differ() {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{home}/.local/share/fonts/Orbitron-VariableFont_wght.ttf");
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("no variable font found — skipping gvar test");
            return;
        };

        let mut thin = Font::parse(data.clone(), 0).expect("parse");
        thin.set_instance(&[(*b"wght", 400.0)]);
        let mut black = Font::parse(data, 0).expect("parse");
        black.set_instance(&[(*b"wght", 900.0)]);

        let gid = thin.glyph_index('O');
        assert_ne!(gid, 0);
        let o_thin = thin.outline(gid).expect("outline");
        let o_black = black.outline(gid).expect("outline");
        assert!(!o_thin.cmds.is_empty());

        // The heavier instance must produce genuinely different geometry.
        let differs = o_thin.cmds.len() != o_black.cmds.len()
            || format!("{:?}", o_thin.cmds) != format!("{:?}", o_black.cmds);
        assert!(differs, "wght 400 and 900 outlines should differ");

        // Heavier weight = more ink at the same size.
        let scale = 32.0 / 1000.0;
        let ink = |o: &crate::raster::outline::Outline| -> u32 {
            raster::rasterize(o, scale, 0.0)
                .map(|g| g.coverage.iter().map(|&c| c as u32).sum())
                .unwrap_or(0)
        };
        let (ink_thin, ink_black) = (ink(&o_thin), ink(&o_black));
        assert!(
            ink_black as f32 > ink_thin as f32 * 1.15,
            "wght 900 should have more ink: {ink_thin} vs {ink_black}"
        );
    }
}

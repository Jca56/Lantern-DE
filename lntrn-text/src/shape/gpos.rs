//! GPOS application: single adjustments, pair kerning, and mark attachment.
//!
//! Operates on a run of glyphs from one font, in font units; the shaper
//! scales results to pixels. Phase 5 scope: lookup types 1 (single), 2 (pair,
//! both formats), 4 (mark-to-base), 6 (mark-to-mark), with type 9 extensions
//! pre-resolved at plan time. Cursive attachment (3) and contextual
//! positioning (7/8) are Phase 8 territory.

use super::gtab::{coverage_index, glyph_class, GposPlan, LookupRef};
use crate::font::sfnt::{read_i16_at, read_u16_at};

/// One glyph's positioning state, in font units.
pub(crate) struct GlyphPos {
    pub gid: u16,
    /// Advance including hmtx base + kern deltas.
    pub x_adv: i32,
    /// Draw offset from the pen position (doesn't move the pen).
    pub x_off: i32,
    /// Draw offset, y-up (font convention; the shaper flips for screen).
    pub y_off: i32,
}

#[derive(Clone, Copy, Default)]
struct ValueAdj {
    x_place: i32,
    y_place: i32,
    x_adv: i32,
}

/// Bytes one ValueRecord occupies for `format` (2 per set bit, device
/// entries included in size but their contents ignored).
fn value_size(format: u16) -> usize {
    (format & 0xFF).count_ones() as usize * 2
}

fn read_value(d: &[u8], mut pos: usize, format: u16) -> ValueAdj {
    let mut v = ValueAdj::default();
    if format & 0x1 != 0 {
        v.x_place = read_i16_at(d, pos).unwrap_or(0) as i32;
        pos += 2;
    }
    if format & 0x2 != 0 {
        v.y_place = read_i16_at(d, pos).unwrap_or(0) as i32;
        pos += 2;
    }
    if format & 0x4 != 0 {
        v.x_adv = read_i16_at(d, pos).unwrap_or(0) as i32;
    }
    v
}

fn add(g: &mut GlyphPos, v: ValueAdj) {
    g.x_off += v.x_place;
    g.y_off += v.y_place;
    g.x_adv += v.x_adv;
}

/// Lookup flag: skip glyphs GDEF classifies as marks when matching.
const IGNORE_MARKS: u16 = 0x0008;

/// Apply the plan's kern + mark lookups to a same-font glyph run.
/// `marks[i]` = GDEF class 3, honored for IgnoreMarks pair kerning (so e.g.
/// Arabic base-to-base kerning works across vowel marks).
pub(crate) fn apply(d: &[u8], plan: &GposPlan, glyphs: &mut [GlyphPos], marks: &[bool]) {
    for lookup in &plan.kern {
        for i in 0..glyphs.len() {
            apply_kern_at(d, lookup, glyphs, marks, i);
        }
    }
    if !plan.mark.is_empty() {
        apply_marks(d, plan, glyphs);
    }
}

/// Within a lookup, the first subtable that matches at a position wins.
fn apply_kern_at(d: &[u8], lookup: &LookupRef, glyphs: &mut [GlyphPos], marks: &[bool], i: usize) {
    let ignore_marks = lookup.flag & IGNORE_MARKS != 0;
    if ignore_marks && marks.get(i).copied().unwrap_or(false) {
        return;
    }
    // Pair partner: next glyph, skipping marks when the lookup asks to.
    let next = if ignore_marks {
        (i + 1..glyphs.len()).find(|&j| !marks[j])
    } else {
        (i + 1 < glyphs.len()).then_some(i + 1)
    };
    for &(kind, sub) in &lookup.subtables {
        let applied = match kind {
            1 => apply_single(d, sub, glyphs, i),
            2 => match next {
                Some(j) => apply_pair(d, sub, glyphs, i, j),
                None => false,
            },
            _ => false,
        };
        if applied {
            return;
        }
    }
}

fn apply_single(d: &[u8], sub: usize, glyphs: &mut [GlyphPos], i: usize) -> bool {
    let Ok(format) = read_u16_at(d, sub) else {
        return false;
    };
    let Ok(cov_rel) = read_u16_at(d, sub + 2) else {
        return false;
    };
    let Some(ci) = coverage_index(d, sub + cov_rel as usize, glyphs[i].gid) else {
        return false;
    };
    let vf = read_u16_at(d, sub + 4).unwrap_or(0);
    let v = match format {
        1 => read_value(d, sub + 6, vf),
        2 => {
            let n = read_u16_at(d, sub + 6).unwrap_or(0);
            if ci >= n {
                return false;
            }
            read_value(d, sub + 8 + ci as usize * value_size(vf), vf)
        }
        _ => return false,
    };
    add(&mut glyphs[i], v);
    true
}

fn apply_pair(d: &[u8], sub: usize, glyphs: &mut [GlyphPos], i: usize, j: usize) -> bool {
    let (left, right) = (glyphs[i].gid, glyphs[j].gid);
    let Ok(format) = read_u16_at(d, sub) else {
        return false;
    };
    let Ok(cov_rel) = read_u16_at(d, sub + 2) else {
        return false;
    };
    let Some(ci) = coverage_index(d, sub + cov_rel as usize, left) else {
        return false;
    };
    let vf1 = read_u16_at(d, sub + 4).unwrap_or(0);
    let vf2 = read_u16_at(d, sub + 6).unwrap_or(0);
    let (s1, s2) = (value_size(vf1), value_size(vf2));

    let pair = match format {
        1 => {
            let n_sets = read_u16_at(d, sub + 8).unwrap_or(0);
            if ci >= n_sets {
                return false;
            }
            let Ok(set_rel) = read_u16_at(d, sub + 10 + 2 * ci as usize) else {
                return false;
            };
            let set = sub + set_rel as usize;
            let n = read_u16_at(d, set).unwrap_or(0) as usize;
            let rec_size = 2 + s1 + s2;
            let (mut lo, mut hi) = (0usize, n);
            let mut hit = None;
            while lo < hi {
                let mid = (lo + hi) / 2;
                let rec = set + 2 + mid * rec_size;
                let second = read_u16_at(d, rec).unwrap_or(0);
                match second.cmp(&right) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => {
                        hit = Some((
                            read_value(d, rec + 2, vf1),
                            read_value(d, rec + 2 + s1, vf2),
                        ));
                        break;
                    }
                }
            }
            match hit {
                Some(p) => p,
                None => return false,
            }
        }
        2 => {
            let cd1 = sub + read_u16_at(d, sub + 8).unwrap_or(0) as usize;
            let cd2 = sub + read_u16_at(d, sub + 10).unwrap_or(0) as usize;
            let c1n = read_u16_at(d, sub + 12).unwrap_or(0);
            let c2n = read_u16_at(d, sub + 14).unwrap_or(0);
            let c1 = glyph_class(d, cd1, left);
            let c2 = glyph_class(d, cd2, right);
            if c1 >= c1n || c2 >= c2n {
                return false;
            }
            let rec = sub + 16 + (c1 as usize * c2n as usize + c2 as usize) * (s1 + s2);
            // A class hit "matches" even with zero values, per spec — later
            // subtables in this lookup are not consulted.
            (read_value(d, rec, vf1), read_value(d, rec + s1, vf2))
        }
        _ => return false,
    };
    add(&mut glyphs[i], pair.0);
    add(&mut glyphs[j], pair.1);
    true
}

// ── Mark attachment ─────────────────────────────────────────────────────────

/// Anchor point: (x, y) in font units (formats 2/3 share this prefix; their
/// contour-point / device refinements are hinting-era detail we skip).
fn anchor(d: &[u8], off: usize) -> (i32, i32) {
    (
        read_i16_at(d, off + 2).unwrap_or(0) as i32,
        read_i16_at(d, off + 4).unwrap_or(0) as i32,
    )
}

/// MarkArray record → (mark class, anchor).
fn mark_anchor(d: &[u8], array: usize, index: u16) -> Option<(u16, (i32, i32))> {
    let count = read_u16_at(d, array).ok()?;
    if index >= count {
        return None;
    }
    let rec = array + 2 + index as usize * 4;
    let class = read_u16_at(d, rec).ok()?;
    let a_rel = read_u16_at(d, rec + 2).ok()?;
    Some((class, anchor(d, array + a_rel as usize)))
}

fn apply_marks(d: &[u8], plan: &GposPlan, glyphs: &mut [GlyphPos]) {
    let n = glyphs.len();
    let mut attached = vec![false; n];
    for i in 1..n {
        'lookups: for lookup in &plan.mark {
            for &(kind, sub) in &lookup.subtables {
                let done = match kind {
                    4 => attach_mark_to_base(d, sub, glyphs, &mut attached, i),
                    6 => attach_mark_to_mark(d, sub, glyphs, &attached, i),
                    _ => false,
                };
                if done {
                    if kind == 6 {
                        attached[i] = true;
                    }
                    break 'lookups;
                }
            }
        }
    }
}

/// Position mark `i` on the nearest preceding base glyph via anchor pairs:
/// mark origin = base pen + (base anchor − mark anchor).
fn attach_mark_to_base(
    d: &[u8],
    sub: usize,
    glyphs: &mut [GlyphPos],
    attached: &mut [bool],
    i: usize,
) -> bool {
    let Ok(mark_cov) = read_u16_at(d, sub + 2) else {
        return false;
    };
    let Some(mi) = coverage_index(d, sub + mark_cov as usize, glyphs[i].gid) else {
        return false;
    };
    let Some(b) = (0..i).rev().find(|&j| !attached[j]) else {
        return false;
    };
    let Ok(base_cov) = read_u16_at(d, sub + 4) else {
        return false;
    };
    let Some(bi) = coverage_index(d, sub + base_cov as usize, glyphs[b].gid) else {
        return false;
    };
    let class_count = read_u16_at(d, sub + 6).unwrap_or(0);
    let mark_array = sub + read_u16_at(d, sub + 8).unwrap_or(0) as usize;
    let base_array = sub + read_u16_at(d, sub + 10).unwrap_or(0) as usize;
    let Some((class, ma)) = mark_anchor(d, mark_array, mi) else {
        return false;
    };
    if class >= class_count {
        return false;
    }
    let slot = base_array + 2 + (bi as usize * class_count as usize + class as usize) * 2;
    let ba_rel = read_u16_at(d, slot).unwrap_or(0);
    if ba_rel == 0 {
        return false;
    }
    let ba = anchor(d, base_array + ba_rel as usize);
    let advance_between: i32 = glyphs[b..i].iter().map(|g| g.x_adv).sum();
    glyphs[i].x_off = glyphs[b].x_off + ba.0 - ma.0 - advance_between;
    glyphs[i].y_off = glyphs[b].y_off + ba.1 - ma.1;
    attached[i] = true;
    true
}

/// Stack mark `i` on the immediately preceding attached mark.
fn attach_mark_to_mark(
    d: &[u8],
    sub: usize,
    glyphs: &mut [GlyphPos],
    attached: &[bool],
    i: usize,
) -> bool {
    if !attached[i - 1] {
        return false;
    }
    let Ok(m1_cov) = read_u16_at(d, sub + 2) else {
        return false;
    };
    let Some(mi) = coverage_index(d, sub + m1_cov as usize, glyphs[i].gid) else {
        return false;
    };
    let Ok(m2_cov) = read_u16_at(d, sub + 4) else {
        return false;
    };
    let Some(bi) = coverage_index(d, sub + m2_cov as usize, glyphs[i - 1].gid) else {
        return false;
    };
    let class_count = read_u16_at(d, sub + 6).unwrap_or(0);
    let mark1_array = sub + read_u16_at(d, sub + 8).unwrap_or(0) as usize;
    let mark2_array = sub + read_u16_at(d, sub + 10).unwrap_or(0) as usize;
    let Some((class, ma)) = mark_anchor(d, mark1_array, mi) else {
        return false;
    };
    if class >= class_count {
        return false;
    }
    let slot = mark2_array + 2 + (bi as usize * class_count as usize + class as usize) * 2;
    let ba_rel = read_u16_at(d, slot).unwrap_or(0);
    if ba_rel == 0 {
        return false;
    }
    let ba = anchor(d, mark2_array + ba_rel as usize);
    let j = i - 1;
    glyphs[i].x_off = glyphs[j].x_off + ba.0 - ma.0 - glyphs[j].x_adv;
    glyphs[i].y_off = glyphs[j].y_off + ba.1 - ma.1;
    true
}

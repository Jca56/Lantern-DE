//! Legacy `kern` table (version 0, format 0) — the pre-OpenType pair-kerning
//! path, used only when a font has no GPOS `kern` feature. Apple's `kern`
//! version 1 (u32 header) and the exotic formats are skipped.

use crate::font::sfnt::{read_i16_at, read_u16_at, read_u32_at};

/// Summed horizontal kerning for the pair, in font units. `t` is the kern
/// table's byte range.
pub(crate) fn kern_pair(t: &[u8], left: u16, right: u16) -> i32 {
    let Ok(version) = read_u16_at(t, 0) else {
        return 0;
    };
    if version != 0 {
        return 0; // Apple 'kern' v1 — not seen in the fonts we target
    }
    let n_tables = read_u16_at(t, 2).unwrap_or(0);
    let mut off = 4usize;
    let mut total = 0i32;
    for _ in 0..n_tables {
        let Ok(length) = read_u16_at(t, off + 2) else {
            break;
        };
        let coverage = read_u16_at(t, off + 4).unwrap_or(0);
        let horizontal = coverage & 0x1 != 0;
        let format = coverage >> 8;
        if horizontal && format == 0 {
            total += format0_pair(t, off + 6, left, right);
        }
        if length == 0 {
            break; // malformed / >64k subtable — length wrapped
        }
        off += length as usize;
    }
    total
}

fn format0_pair(t: &[u8], sub: usize, left: u16, right: u16) -> i32 {
    let Ok(n_pairs) = read_u16_at(t, sub) else {
        return 0;
    };
    let pairs = sub + 8; // skip nPairs + searchRange + entrySelector + rangeShift
    let key = ((left as u32) << 16) | right as u32;
    let (mut lo, mut hi) = (0usize, n_pairs as usize);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let rec = pairs + mid * 6;
        let Ok(pair_key) = read_u32_at(t, rec) else {
            return 0;
        };
        match pair_key.cmp(&key) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => return read_i16_at(t, rec + 4).unwrap_or(0) as i32,
        }
    }
    0
}

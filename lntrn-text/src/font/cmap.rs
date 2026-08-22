//! `cmap` — character → glyph index mapping.
//!
//! Supports subtable formats 0 (byte table), 4 (BMP segments), 6 (trimmed
//! range), and 12 (full-Unicode groups). Lookups stay zero-copy: the chosen
//! subtable is binary-searched in place in the font data on every call, so no
//! decoded map is built or stored. Format 14 (variation selectors) is a
//! shaping concern and lands with Phase 7/8.

use super::sfnt::{read_u16_at, read_u32_at};
use super::FontError;

pub(crate) struct Cmap {
    /// Absolute offset of the chosen subtable in the font data.
    off: usize,
    format: u16,
}

impl Cmap {
    pub fn parse(data: &[u8], cmap_off: usize) -> Result<Self, FontError> {
        let num_tables = read_u16_at(data, cmap_off + 2)? as usize;
        let mut best: Option<(u32, usize, u16)> = None;
        for i in 0..num_tables {
            let rec = cmap_off + 4 + 8 * i;
            let platform = read_u16_at(data, rec)?;
            let encoding = read_u16_at(data, rec + 2)?;
            let off = cmap_off + read_u32_at(data, rec + 4)? as usize;
            let Ok(format) = read_u16_at(data, off) else {
                continue;
            };
            if !matches!(format, 0 | 4 | 6 | 12) {
                continue;
            }
            // Prefer full-Unicode subtables, then BMP Unicode, then legacy.
            let score = match (platform, encoding) {
                (3, 10) | (0, 4) | (0, 6) => 100,
                (3, 1) | (0, 3) => 80,
                (0, _) => 60,
                (3, 0) => 40, // symbol
                (1, 0) => 20, // legacy Mac
                _ => 10,
            } + u32::from(format == 12);
            if best.is_none_or(|(s, _, _)| score > s) {
                best = Some((score, off, format));
            }
        }
        let (_, off, format) = best.ok_or(FontError::Unsupported(
            "no usable cmap subtable (formats 0/4/6/12)",
        ))?;
        Ok(Cmap { off, format })
    }

    /// The subtable's mapped codepoint ranges (sorted, merged, inclusive).
    /// A coarse coverage summary for the font database: segment bounds are
    /// reported as-is, so a range may include a few codepoints that still map
    /// to `.notdef` — callers must treat this as a hint and confirm with
    /// [`Cmap::glyph_index`] once the face is actually loaded.
    pub fn coverage(&self, data: &[u8]) -> Vec<(u32, u32)> {
        let s = self.off;
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        match self.format {
            0 => {
                // 256-byte table: compress non-zero runs.
                let mut run: Option<(u32, u32)> = None;
                for c in 0..=0xFFu32 {
                    let mapped = data.get(s + 6 + c as usize).is_some_and(|&g| g != 0);
                    match (mapped, run) {
                        (true, Some((st, _))) => run = Some((st, c)),
                        (true, None) => run = Some((c, c)),
                        (false, Some(r)) => {
                            ranges.push(r);
                            run = None;
                        }
                        (false, None) => {}
                    }
                }
                if let Some(r) = run {
                    ranges.push(r);
                }
            }
            4 => {
                let seg_x2 = read_u16_at(data, s + 6).unwrap_or(0) as usize;
                let end_base = s + 14;
                let start_base = end_base + seg_x2 + 2;
                for i in 0..seg_x2 / 2 {
                    let (Ok(end), Ok(start)) = (
                        read_u16_at(data, end_base + 2 * i),
                        read_u16_at(data, start_base + 2 * i),
                    ) else {
                        break;
                    };
                    if start == 0xFFFF && end == 0xFFFF {
                        continue; // the required terminal sentinel segment
                    }
                    if start <= end {
                        ranges.push((start as u32, end as u32));
                    }
                }
            }
            6 => {
                let first = read_u16_at(data, s + 6).unwrap_or(0) as u32;
                let count = read_u16_at(data, s + 8).unwrap_or(0) as u32;
                if count > 0 {
                    ranges.push((first, first + count - 1));
                }
            }
            12 => {
                let n = read_u32_at(data, s + 12).unwrap_or(0) as usize;
                for i in 0..n.min(100_000) {
                    let (Ok(start), Ok(end)) = (
                        read_u32_at(data, s + 16 + 12 * i),
                        read_u32_at(data, s + 16 + 12 * i + 4),
                    ) else {
                        break;
                    };
                    if start <= end {
                        ranges.push((start, end));
                    }
                }
            }
            _ => {}
        }
        ranges.sort_unstable();
        // Merge touching/overlapping ranges.
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            match merged.last_mut() {
                Some((_, pe)) if start <= pe.saturating_add(1) => *pe = (*pe).max(end),
                _ => merged.push((start, end)),
            }
        }
        merged
    }

    /// Map a Unicode scalar to a glyph index; 0 (`.notdef`) when unmapped.
    pub fn glyph_index(&self, data: &[u8], c: u32) -> u16 {
        let gid = match self.format {
            0 => lookup_f0(data, self.off, c),
            4 => lookup_f4(data, self.off, c),
            6 => lookup_f6(data, self.off, c),
            12 => lookup_f12(data, self.off, c),
            _ => None,
        };
        gid.unwrap_or(0)
    }
}

/// Format 0: 256-entry byte table.
fn lookup_f0(d: &[u8], s: usize, c: u32) -> Option<u16> {
    if c > 0xFF {
        return None;
    }
    let gid = *d.get(s + 6 + c as usize)? as u16;
    (gid != 0).then_some(gid)
}

/// Format 4: BMP segment mapping (endCode/startCode/idDelta/idRangeOffset).
fn lookup_f4(d: &[u8], s: usize, c: u32) -> Option<u16> {
    if c > 0xFFFF {
        return None;
    }
    let c = c as u16;
    let seg_x2 = read_u16_at(d, s + 6).ok()? as usize;
    let seg_count = seg_x2 / 2;
    if seg_count == 0 {
        return None;
    }
    let end_base = s + 14;
    let start_base = end_base + seg_x2 + 2; // +2 skips reservedPad
    let delta_base = start_base + seg_x2;
    let range_base = delta_base + seg_x2;

    // First segment whose endCode >= c.
    let (mut lo, mut hi) = (0usize, seg_count);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if read_u16_at(d, end_base + 2 * mid).ok()? < c {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= seg_count {
        return None;
    }
    let start = read_u16_at(d, start_base + 2 * lo).ok()?;
    if c < start {
        return None;
    }
    let delta = read_u16_at(d, delta_base + 2 * lo).ok()?;
    let range_off = read_u16_at(d, range_base + 2 * lo).ok()?;
    let gid = if range_off == 0 {
        c.wrapping_add(delta)
    } else {
        // idRangeOffset is relative to its own position in the table.
        let addr = range_base + 2 * lo + range_off as usize + 2 * (c - start) as usize;
        let raw = read_u16_at(d, addr).ok()?;
        if raw == 0 {
            return None;
        }
        raw.wrapping_add(delta)
    };
    (gid != 0).then_some(gid)
}

/// Format 6: trimmed contiguous range.
fn lookup_f6(d: &[u8], s: usize, c: u32) -> Option<u16> {
    let first = read_u16_at(d, s + 6).ok()? as u32;
    let count = read_u16_at(d, s + 8).ok()? as u32;
    if c < first || c >= first + count {
        return None;
    }
    let gid = read_u16_at(d, s + 10 + 2 * (c - first) as usize).ok()?;
    (gid != 0).then_some(gid)
}

/// Format 12: sequential map groups (full Unicode range).
fn lookup_f12(d: &[u8], s: usize, c: u32) -> Option<u16> {
    let num_groups = read_u32_at(d, s + 12).ok()? as usize;
    let groups = s + 16;
    let (mut lo, mut hi) = (0usize, num_groups);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if read_u32_at(d, groups + 12 * mid + 4).ok()? < c {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= num_groups {
        return None;
    }
    let start = read_u32_at(d, groups + 12 * lo).ok()?;
    if c < start {
        return None;
    }
    let gid = read_u32_at(d, groups + 12 * lo + 8).ok()? + (c - start);
    // Glyph ids are u16 in every other table; larger values are corrupt.
    u16::try_from(gid).ok().filter(|&g| g != 0)
}

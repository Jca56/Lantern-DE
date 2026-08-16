//! Metric tables: `head`, `hhea`, `maxp`, `hmtx`.

use crate::font::sfnt::{read_i16_at, read_u16_at, read_u32_at};
use crate::font::FontError;

const HEAD_MAGIC: u32 = 0x5F0F_3CF5;

pub(crate) struct Head {
    pub units_per_em: u16,
    /// `indexToLocFormat`: false = u16 offsets ("short"), true = u32 ("long").
    pub long_loca: bool,
}

pub(crate) fn parse_head(t: &[u8]) -> Result<Head, FontError> {
    if read_u32_at(t, 12)? != HEAD_MAGIC {
        return Err(FontError::BadMagic(read_u32_at(t, 12)?));
    }
    let units_per_em = read_u16_at(t, 18)?;
    if units_per_em == 0 {
        return Err(FontError::Unsupported("unitsPerEm = 0"));
    }
    Ok(Head {
        units_per_em,
        long_loca: read_i16_at(t, 50)? != 0,
    })
}

pub(crate) struct Hhea {
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub num_h_metrics: u16,
}

pub(crate) fn parse_hhea(t: &[u8]) -> Result<Hhea, FontError> {
    Ok(Hhea {
        ascender: read_i16_at(t, 4)?,
        descender: read_i16_at(t, 6)?,
        line_gap: read_i16_at(t, 8)?,
        num_h_metrics: read_u16_at(t, 34)?,
    })
}

/// `maxp` → glyph count.
pub(crate) fn parse_maxp(t: &[u8]) -> Result<u16, FontError> {
    read_u16_at(t, 4)
}

/// Advance width in font units for `gid`. Glyphs past `numberOfHMetrics`
/// repeat the last advance (the monospace tail-compression scheme).
pub(crate) fn hmtx_advance(hmtx: &[u8], num_h_metrics: u16, gid: u16) -> u16 {
    let n = num_h_metrics.max(1);
    let i = gid.min(n - 1) as usize;
    read_u16_at(hmtx, i * 4).unwrap_or(0)
}

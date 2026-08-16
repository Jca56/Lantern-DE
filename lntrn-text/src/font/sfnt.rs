//! sfnt container parsing: byte readers, the table directory, and TrueType
//! Collection (`ttcf`) indirection.

use super::FontError;

// ── Bounds-checked primitive reads ───────────────────────────────────────────

pub(crate) fn read_u8_at(d: &[u8], off: usize) -> Result<u8, FontError> {
    d.get(off).copied().ok_or(FontError::Truncated)
}

pub(crate) fn read_u16_at(d: &[u8], off: usize) -> Result<u16, FontError> {
    match d.get(off..off + 2) {
        Some(b) => Ok(u16::from_be_bytes([b[0], b[1]])),
        None => Err(FontError::Truncated),
    }
}

pub(crate) fn read_i16_at(d: &[u8], off: usize) -> Result<i16, FontError> {
    read_u16_at(d, off).map(|v| v as i16)
}

pub(crate) fn read_u32_at(d: &[u8], off: usize) -> Result<u32, FontError> {
    match d.get(off..off + 4) {
        Some(b) => Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
        None => Err(FontError::Truncated),
    }
}

/// Sequential byte cursor for record-structured tables (glyf, composites).
pub(crate) struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }

    pub fn u8(&mut self) -> Result<u8, FontError> {
        let v = read_u8_at(self.d, self.pos)?;
        self.pos += 1;
        Ok(v)
    }

    pub fn u16(&mut self) -> Result<u16, FontError> {
        let v = read_u16_at(self.d, self.pos)?;
        self.pos += 2;
        Ok(v)
    }

    pub fn i16(&mut self) -> Result<i16, FontError> {
        self.u16().map(|v| v as i16)
    }

    /// Fixed-point 2.14 → f32 (composite glyph scale factors).
    pub fn f2dot14(&mut self) -> Result<f32, FontError> {
        self.i16().map(|v| v as f32 / 16384.0)
    }

    pub fn skip(&mut self, n: usize) -> Result<(), FontError> {
        if self.pos + n > self.d.len() {
            return Err(FontError::Truncated);
        }
        self.pos += n;
        Ok(())
    }
}

// ── Table directory ──────────────────────────────────────────────────────────

pub(crate) struct TableDir {
    entries: Vec<([u8; 4], usize, usize)>,
}

impl TableDir {
    /// Look up a table by tag → `(offset, length)` into the font data.
    /// Offsets are validated in-bounds at parse time.
    pub fn find(&self, tag: &[u8; 4]) -> Option<(usize, usize)> {
        self.entries
            .iter()
            .find(|(t, _, _)| t == tag)
            .map(|&(_, off, len)| (off, len))
    }
}

const SFNT_TRUETYPE: u32 = 0x0001_0000;
const SFNT_TRUE: u32 = 0x7472_7565; // 'true' (legacy Apple)
const SFNT_OTTO: u32 = 0x4F54_544F; // 'OTTO' (CFF outlines)
const SFNT_TTCF: u32 = 0x7474_6366; // 'ttcf' (collection)

/// Parse the container and return the table directory for face `index`
/// (only collections have more than one face; `index` is ignored otherwise).
pub(crate) fn parse(data: &[u8], index: u32) -> Result<TableDir, FontError> {
    match read_u32_at(data, 0)? {
        SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO => parse_dir(data, 0),
        SFNT_TTCF => {
            let num_fonts = read_u32_at(data, 8)?;
            if index >= num_fonts {
                return Err(FontError::BadIndex(index));
            }
            let off = read_u32_at(data, 12 + 4 * index as usize)? as usize;
            match read_u32_at(data, off)? {
                SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO => parse_dir(data, off),
                other => Err(FontError::BadMagic(other)),
            }
        }
        other => Err(FontError::BadMagic(other)),
    }
}

fn parse_dir(data: &[u8], off: usize) -> Result<TableDir, FontError> {
    let num_tables = read_u16_at(data, off + 4)? as usize;
    let mut entries = Vec::with_capacity(num_tables);
    for i in 0..num_tables {
        let rec = off + 12 + 16 * i;
        let tag = read_u32_at(data, rec)?.to_be_bytes();
        let t_off = read_u32_at(data, rec + 8)? as usize;
        let t_len = read_u32_at(data, rec + 12)? as usize;
        if t_off.checked_add(t_len).is_none_or(|end| end > data.len()) {
            return Err(FontError::Truncated);
        }
        entries.push((tag, t_off, t_len));
    }
    Ok(TableDir { entries })
}

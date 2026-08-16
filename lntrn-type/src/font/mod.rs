//! Font parsing: sfnt/ttc containers, metric tables, character mapping, and
//! glyph outlines.
//!
//! Phase 1 scope: TrueType (`glyf`) outline fonts. CFF/CFF2 (Phase 9),
//! variations (Phase 10), and color tables (Phase 11) come later; discovery,
//! matching, and fallback (`db.rs`) land in Phase 2.

mod cmap;
mod glyf;
mod sfnt;
mod tables;

use std::fmt;

use crate::raster::outline::Outline;

#[derive(Clone, Copy, Debug)]
pub enum FontError {
    /// A read ran past the end of the data (or a table past its bounds).
    Truncated,
    /// Unrecognized sfnt version or table magic.
    BadMagic(u32),
    /// Valid font, but needs a later phase's machinery.
    Unsupported(&'static str),
    MissingTable([u8; 4]),
    /// Glyph id out of range for this font.
    BadGlyph(u16),
    /// Collection face index out of range.
    BadIndex(u32),
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "font data truncated"),
            Self::BadMagic(v) => write!(f, "unrecognized font magic {v:#010x}"),
            Self::Unsupported(what) => write!(f, "unsupported: {what}"),
            Self::MissingTable(tag) => {
                write!(f, "missing table `{}`", String::from_utf8_lossy(tag))
            }
            Self::BadGlyph(gid) => write!(f, "glyph id {gid} out of range"),
            Self::BadIndex(i) => write!(f, "collection face index {i} out of range"),
        }
    }
}

impl std::error::Error for FontError {}

/// A parsed font face. Owns its raw data; tables are referenced by validated
/// offset and read in place (zero-copy).
pub(crate) struct Font {
    pub(crate) data: Vec<u8>,
    units_per_em: f32,
    ascender: i16,
    /// Negative (below baseline). Used by ink measurement in Phase 3.
    #[allow(dead_code)]
    descender: i16,
    #[allow(dead_code)]
    line_gap: i16,
    num_h_metrics: u16,
    pub(crate) num_glyphs: u16,
    pub(crate) long_loca: bool,
    cmap: cmap::Cmap,
    /// (offset, length) into `data`.
    pub(crate) loca: (usize, usize),
    pub(crate) glyf: (usize, usize),
    hmtx: (usize, usize),
}

impl Font {
    /// Parse face `index` (for `.ttc` collections; 0 for single-face files).
    pub fn parse(data: Vec<u8>, index: u32) -> Result<Self, FontError> {
        let dir = sfnt::parse(&data, index)?;
        let need = |tag: &[u8; 4]| dir.find(tag).ok_or(FontError::MissingTable(*tag));
        let table = |range: (usize, usize)| {
            data.get(range.0..range.0 + range.1).ok_or(FontError::Truncated)
        };

        let head = tables::parse_head(table(need(b"head")?)?)?;
        let hhea = tables::parse_hhea(table(need(b"hhea")?)?)?;
        let num_glyphs = tables::parse_maxp(table(need(b"maxp")?)?)?;
        let cmap = cmap::Cmap::parse(&data, need(b"cmap")?.0)?;
        let glyf = dir
            .find(b"glyf")
            .ok_or(FontError::Unsupported("no `glyf` outlines (CFF lands in Phase 9)"))?;
        let loca = need(b"loca")?;
        let hmtx = need(b"hmtx")?;

        Ok(Font {
            data,
            units_per_em: head.units_per_em as f32,
            ascender: hhea.ascender,
            descender: hhea.descender,
            line_gap: hhea.line_gap,
            num_h_metrics: hhea.num_h_metrics,
            num_glyphs,
            long_loca: head.long_loca,
            cmap,
            loca,
            glyf,
            hmtx,
        })
    }

    /// Pixels per font unit at `px` pixels-per-em.
    pub fn scale(&self, px: f32) -> f32 {
        px / self.units_per_em
    }

    /// Baseline offset from the top of the line box, in pixels.
    pub fn ascender_px(&self, px: f32) -> f32 {
        self.ascender as f32 * self.scale(px)
    }

    /// Character → glyph index; 0 (`.notdef`) when unmapped.
    pub fn glyph_index(&self, ch: char) -> u16 {
        let gid = self.cmap.glyph_index(&self.data, ch as u32);
        if gid < self.num_glyphs {
            gid
        } else {
            0
        }
    }

    /// Advance width in font units.
    pub fn advance_units(&self, gid: u16) -> u16 {
        let (off, len) = self.hmtx;
        match self.data.get(off..off + len) {
            Some(hmtx) => tables::hmtx_advance(hmtx, self.num_h_metrics, gid),
            None => 0,
        }
    }

    /// Decode the glyph's outline (composites pre-flattened into one path).
    pub fn outline(&self, gid: u16) -> Result<Outline, FontError> {
        glyf::outline(self, gid)
    }
}

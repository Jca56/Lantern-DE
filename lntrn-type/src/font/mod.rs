//! Font parsing + the font database: sfnt/ttc containers, metric/naming
//! tables, character mapping, glyph outlines, discovery, and fallback.
//!
//! Phase 1+2 scope: TrueType (`glyf`) outline fonts, discovered at runtime
//! and matched by family/weight/style with per-glyph fallback. CFF/CFF2
//! (Phase 9), variations (Phase 10), and color tables (Phase 11) come later.

mod cmap;
pub(crate) mod db;
mod glyf;
mod scan;
pub(crate) mod sfnt;
mod tables;

use std::fmt;

use crate::raster::outline::Outline;
use crate::shape::gpos::{self, GlyphPos};
use crate::shape::gsub;
use crate::shape::gtab::{GposPlan, GsubPlan};
use crate::shape::kern;

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
    /// GPOS kern/mark lookups per script tag, gathered at parse.
    gpos_plans: Vec<([u8; 4], GposPlan)>,
    /// GSUB substitution lookups per script tag, gathered at parse.
    gsub_plans: Vec<([u8; 4], GsubPlan)>,
    /// Legacy `kern` table, used only when GPOS has no kern feature.
    kern: Option<(usize, usize)>,
    /// GDEF glyph-class definition (absolute offset); class 3 = mark.
    gdef_classes: Option<usize>,
}

/// Pick the plan for a shaping run's script: exact tag, else DFLT, else
/// latn, else whatever the font has.
fn plan_for<T>(plans: &[([u8; 4], T)], script: [u8; 4]) -> Option<&T> {
    plans
        .iter()
        .find(|(t, _)| *t == script)
        .or_else(|| plans.iter().find(|(t, _)| t == b"DFLT"))
        .or_else(|| plans.iter().find(|(t, _)| t == b"latn"))
        .or_else(|| plans.first())
        .map(|(_, p)| p)
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
        // Build layout plans per script the tables declare (Arabic features
        // live under `arab`, not the latn/DFLT default a single plan would
        // pick). A shaping run selects by its script at runtime.
        let mut gpos_plans = Vec::new();
        if let Some((off, _)) = dir.find(b"GPOS") {
            for tag in crate::shape::gtab::script_tags(&data, off) {
                if !gpos_plans.iter().any(|(t, _)| *t == tag) {
                    gpos_plans.push((tag, GposPlan::build(&data, off, Some(tag))));
                }
            }
        }
        let mut gsub_plans = Vec::new();
        if let Some((off, _)) = dir.find(b"GSUB") {
            for tag in crate::shape::gtab::script_tags(&data, off) {
                if !gsub_plans.iter().any(|(t, _)| *t == tag) {
                    gsub_plans.push((tag, GsubPlan::build(&data, off, Some(tag))));
                }
            }
        }
        let kern = dir.find(b"kern");
        let gdef_classes = dir.find(b"GDEF").and_then(|(off, _)| {
            match crate::font::sfnt::read_u16_at(&data, off + 4) {
                Ok(rel) if rel != 0 => Some(off + rel as usize),
                _ => None,
            }
        });

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
            gpos_plans,
            gsub_plans,
            kern,
            gdef_classes,
        })
    }

    /// GDEF glyph class 3 = mark (used for lookup mark-filtering).
    fn is_mark_glyph(&self, gid: u16) -> bool {
        self.gdef_classes
            .is_some_and(|cd| crate::shape::gtab::glyph_class(&self.data, cd, gid) == 3)
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

    /// Apply GSUB substitution (ligatures, contextual alternates, Arabic
    /// positional forms) for `script` to a glyph run. Substituted ids are
    /// clamped to the glyph count defensively.
    pub fn substitute(&self, glyphs: &mut Vec<gsub::Glyph>, script: [u8; 4]) {
        if let Some(plan) = plan_for(&self.gsub_plans, script) {
            gsub::apply(&self.data, plan, glyphs);
            for g in glyphs.iter_mut() {
                if g.gid >= self.num_glyphs {
                    g.gid = 0;
                }
            }
        }
    }

    /// Apply positioning (GPOS kern/single/marks, or the legacy `kern` table
    /// when GPOS carries no kern feature) for `script`, in font units.
    pub fn position(&self, glyphs: &mut [GlyphPos], script: [u8; 4]) {
        let mut gpos_kerned = false;
        if let Some(plan) = plan_for(&self.gpos_plans, script) {
            let marks: Vec<bool> = glyphs.iter().map(|g| self.is_mark_glyph(g.gid)).collect();
            gpos::apply(&self.data, plan, glyphs, &marks);
            gpos_kerned = !plan.kern.is_empty();
        }
        if !gpos_kerned {
            if let Some((off, len)) = self.kern {
                if let Some(table) = self.data.get(off..off + len) {
                    for i in 0..glyphs.len().saturating_sub(1) {
                        glyphs[i].x_adv += kern::kern_pair(table, glyphs[i].gid, glyphs[i + 1].gid);
                    }
                }
            }
        }
    }
}

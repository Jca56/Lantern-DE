//! Font parsing + the font database: sfnt/ttc containers, metric/naming
//! tables, character mapping, glyph outlines, discovery, and fallback.
//!
//! Phase 1+2 scope: TrueType (`glyf`) outline fonts, discovered at runtime
//! and matched by family/weight/style with per-glyph fallback. CFF/CFF2
//! (Phase 9), variations (Phase 10), and color tables (Phase 11) come later.

mod cbdt;
mod cff;
mod cmap;
mod colr;
pub(crate) mod db;
mod glyf;
mod gvar;
mod scan;
pub(crate) mod sfnt;
mod tables;
pub(crate) mod variations;

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
    /// (offset, length) into `data` (zeroed for CFF-outline fonts).
    pub(crate) loca: (usize, usize),
    pub(crate) glyf: (usize, usize),
    /// CFF outline source; `None` = TrueType `glyf`.
    cff: Option<cff::Cff>,
    /// Color bitmap strikes (emoji).
    cbdt: Option<cbdt::Cbdt>,
    /// Layered color glyphs (COLRv0).
    pub(crate) colr: Option<colr::Colr>,
    hmtx: (usize, usize),
    /// GPOS kern/mark lookups per script tag, gathered at parse.
    gpos_plans: Vec<([u8; 4], GposPlan)>,
    /// GSUB substitution lookups per script tag, gathered at parse.
    gsub_plans: Vec<([u8; 4], GsubPlan)>,
    /// Legacy `kern` table, used only when GPOS has no kern feature.
    kern: Option<(usize, usize)>,
    /// GDEF glyph-class definition (absolute offset); class 3 = mark.
    gdef_classes: Option<usize>,
    /// Variation axes from `fvar` (empty = static font).
    axes: Vec<variations::Axis>,
    /// `avar` table range for normalization remapping.
    avar: Option<(usize, usize)>,
    gvar: Option<gvar::Gvar>,
    hvar: Option<variations::Hvar>,
    /// Normalized design-space position (empty = default instance).
    norm_coords: Vec<f32>,
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
        let cbdt = dir
            .find(b"CBLC")
            .zip(dir.find(b"CBDT"))
            .and_then(|((cblc, _), (cbdt_off, _))| cbdt::Cbdt::parse(&data, cblc, cbdt_off));
        let (glyf, loca, cff) = if let Some(glyf) = dir.find(b"glyf") {
            (glyf, need(b"loca")?, None)
        } else if let Some(range) = dir.find(b"CFF ") {
            ((0, 0), (0, 0), Some(cff::Cff::parse(&data, range)?))
        } else if cbdt.is_some() {
            ((0, 0), (0, 0), None) // bitmap-only color font (CBDT strikes)
        } else {
            return Err(FontError::Unsupported(
                "no `glyf`, `CFF `, or `CBDT` glyph source (CFF2 pending)",
            ));
        };
        let colr = dir
            .find(b"COLR")
            .zip(dir.find(b"CPAL"))
            .and_then(|((colr_off, _), (cpal_off, _))| colr::Colr::parse(&data, colr_off, cpal_off));
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
        let axes = dir
            .find(b"fvar")
            .and_then(|r| data.get(r.0..r.0 + r.1))
            .and_then(variations::parse_fvar)
            .map(|(axes, _)| axes)
            .unwrap_or_default();
        let avar = dir.find(b"avar");
        let gvar = dir.find(b"gvar").and_then(|(off, _)| gvar::Gvar::parse(&data, off));
        let hvar = dir.find(b"HVAR").and_then(|(off, _)| variations::Hvar::parse(&data, off));

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
            cff,
            cbdt,
            colr,
            hmtx,
            gpos_plans,
            gsub_plans,
            kern,
            gdef_classes,
            axes,
            avar,
            gvar,
            hvar,
            norm_coords: Vec::new(),
        })
    }

    /// Select a variable-font instance from user-space axis values (e.g.
    /// `("wght", 700.0)`). No-op for static fonts.
    pub fn set_instance(&mut self, user: &[([u8; 4], f32)]) {
        if self.axes.is_empty() || user.is_empty() {
            return;
        }
        let avar = self.avar.and_then(|(o, l)| self.data.get(o..o + l));
        let coords = variations::normalize(&self.axes, avar, user);
        // All-zero = the default instance; keep the static fast path.
        self.norm_coords = if coords.iter().all(|&c| c == 0.0) { Vec::new() } else { coords };
    }

    /// gvar + normalized coords, when this font is a non-default instance.
    pub(crate) fn variation(&self) -> Option<(&gvar::Gvar, &[f32])> {
        match &self.gvar {
            Some(g) if !self.norm_coords.is_empty() => Some((g, &self.norm_coords)),
            _ => None,
        }
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

    /// Advance width in font units (HVAR-adjusted for variable instances).
    pub fn advance_units(&self, gid: u16) -> i32 {
        let (off, len) = self.hmtx;
        let base = match self.data.get(off..off + len) {
            Some(hmtx) => tables::hmtx_advance(hmtx, self.num_h_metrics, gid) as i32,
            None => 0,
        };
        if !self.norm_coords.is_empty() {
            if let Some(hvar) = &self.hvar {
                return base
                    + hvar.advance_delta(&self.data, gid, &self.norm_coords).round() as i32;
            }
        }
        base
    }

    /// Decode the glyph's outline (composites pre-flattened into one path).
    pub fn outline(&self, gid: u16) -> Result<Outline, FontError> {
        match &self.cff {
            Some(cff) => {
                if gid >= self.num_glyphs {
                    return Err(FontError::BadGlyph(gid));
                }
                let mut out = Outline::default();
                cff::outline(&self.data, cff, gid, &mut out)?;
                Ok(out)
            }
            None if self.glyf.1 == 0 => Ok(Outline::default()), // bitmap-only face
            None => glyf::outline(self, gid),
        }
    }

    /// Color (RGBA) rendition of `gid` at `px`, when this face has one:
    /// CBDT bitmap strikes first, then COLRv0 layers.
    pub fn color_glyph(&self, gid: u16, px: f32) -> Option<crate::raster::RasterRgba> {
        if let Some(cbdt) = &self.cbdt {
            if let Some(glyph) = cbdt.glyph(&self.data, gid, px) {
                return Some(glyph);
            }
        }
        if self.colr.is_some() {
            return colr::rasterize(self, gid, px);
        }
        None
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

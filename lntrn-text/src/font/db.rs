//! Font database: face index, lazy loading, matching, and per-glyph fallback.
//!
//! Discovery scans metadata only (see [`super::scan`]); a face's full data is
//! read and parsed the first time it actually renders. Faces that fail to
//! parse at load time are disabled and matching re-resolves without them.
//!
//! Fallback has two tiers: the Lantern fallback family list (kept in sync
//! with the glyphon wrapper's `LanternFallback` order — emoji after Noto Sans
//! but ahead of DejaVu, mono and symbol families after), then a coverage
//! search across every known face. The coverage tier is what routes e.g.
//! katakana in a Latin UI font to UDEV Gothic / Noto CJK without any
//! configuration.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use super::scan::{self, FaceMeta};
use super::{Font, FontError};
use crate::{FontStyle, FontWeight};

/// Mirrors the glyphon wrapper's fallback order. Lowercased for matching.
/// "noto color emoji" is currently skipped at scan time (no `glyf` outlines
/// until Phase 11) but stays listed so it slots back in when color lands.
const FALLBACK_FAMILIES: &[&str] = &[
    "noto sans",
    "noto color emoji",
    "dejavu sans",
    "freesans",
    "noto sans mono",
    "dejavu sans mono",
    "freemono",
    "noto sans symbols",
    "noto sans symbols2",
];

/// Default-family chains, tried after the configured family. The theme
/// proportional default is Inter (bundled in ~/.lantern/fonts).
const SANS_DEFAULTS: &[&str] = &["inter", "noto sans", "dejavu sans", "freesans"];
const MONO_DEFAULTS: &[&str] = &["dejavu sans mono", "jetbrains mono", "freemono"];

/// (CSS weight, italic) for the public style enums.
pub(crate) fn style_params(weight: FontWeight, style: FontStyle) -> (u16, bool) {
    let w = match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
    };
    (w, matches!(style, FontStyle::Italic))
}

enum FaceSource {
    File(PathBuf),
    /// Loaded via `load_font_data`; parsed eagerly, so never re-read.
    Embedded,
}

struct FaceRecord {
    source: FaceSource,
    face_index: u32,
    families: Vec<String>,
    weight: u16,
    width: u16,
    italic: bool,
    monospace: bool,
    coverage: Vec<(u32, u32)>,
    /// Variable-font instance coordinates (empty = static face).
    var_coords: Vec<([u8; 4], f32)>,
}

impl FaceRecord {
    fn from_meta(meta: FaceMeta, source: FaceSource) -> Self {
        Self {
            source,
            face_index: meta.face_index,
            families: meta.families,
            weight: meta.weight,
            width: meta.width,
            italic: meta.italic,
            monospace: meta.monospace,
            coverage: meta.coverage,
            var_coords: meta.var_coords,
        }
    }

    fn covers(&self, c: u32) -> bool {
        self.coverage
            .binary_search_by(|&(start, end)| {
                if end < c {
                    Ordering::Less
                } else if start > c {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
            .is_ok()
    }
}

pub(crate) struct FontDb {
    records: Vec<FaceRecord>,
    /// Parallel to `records`; `None` = not loaded yet.
    fonts: Vec<Option<Font>>,
    /// Parallel to `records`; a face that failed to load stays disabled.
    dead: Vec<bool>,
    sans_family: String,
    mono_family: String,
    /// (family_lower, monospace, weight, italic) → face.
    resolve_cache: HashMap<(String, bool, u16, bool), Option<usize>>,
    /// (char, weight, italic) → fallback face for chars the primary lacks.
    fallback_cache: HashMap<(char, u16, bool), Option<usize>>,
}

impl FontDb {
    /// Scan the system + Lantern font directories. `sans_family` is the DE
    /// font from lantern.toml; `mono_family` the generic monospace default.
    pub fn discover(sans_family: &str, mono_family: &str) -> Self {
        let files = scan::collect_font_files(&scan::font_dirs());
        let mut records = Vec::new();
        let mut skipped = 0usize;
        for path in &files {
            let metas = scan::scan_file(path);
            if metas.is_empty() {
                skipped += 1;
                continue;
            }
            for meta in metas {
                records.push(FaceRecord::from_meta(meta, FaceSource::File(path.clone())));
            }
        }
        eprintln!(
            "[lntrn-text] font db: {} faces from {} files ({} skipped: CFF/bitmap/unreadable)",
            records.len(),
            files.len(),
            skipped
        );
        let n = records.len();
        Self {
            records,
            fonts: (0..n).map(|_| None).collect(),
            dead: vec![false; n],
            sans_family: sans_family.trim().to_ascii_lowercase(),
            mono_family: mono_family.trim().to_ascii_lowercase(),
            resolve_cache: HashMap::new(),
            fallback_cache: HashMap::new(),
        }
    }

    /// Register an embedded font (raw `.ttf`/`.ttc` bytes, face 0). Variable
    /// fonts contribute one face per instance.
    pub fn add_font_data(&mut self, data: Vec<u8>) -> Result<(), FontError> {
        let mut metas = scan::meta_from_slice(&data, 0);
        if metas.is_empty() {
            metas.push(FaceMeta {
                face_index: 0,
                families: Vec::new(),
                weight: 400,
                width: 5,
                italic: false,
                monospace: false,
                coverage: Vec::new(),
                var_coords: Vec::new(),
            });
        }
        for meta in metas {
            let mut font = Font::parse(data.clone(), 0)?;
            if !meta.var_coords.is_empty() {
                font.set_instance(&meta.var_coords);
            }
            self.records.push(FaceRecord::from_meta(meta, FaceSource::Embedded));
            self.fonts.push(Some(font));
            self.dead.push(false);
        }
        // New faces may satisfy earlier family/fallback misses.
        self.resolve_cache.clear();
        self.fallback_cache.clear();
        Ok(())
    }

    pub fn face_count(&self) -> usize {
        self.records.len()
    }

    /// Pick the face for a queue/measure call. `family: None` = the renderer
    /// default (monospace or the DE sans family); an unknown family falls back
    /// to that default, matching the glyphon wrapper's behavior.
    pub fn resolve(
        &mut self,
        family: Option<&str>,
        monospace: bool,
        weight: FontWeight,
        style: FontStyle,
    ) -> Option<usize> {
        let (w, italic) = style_params(weight, style);
        let fam = family
            .map(|f| f.trim().to_ascii_lowercase())
            .filter(|f| !f.is_empty());
        let key = (fam.clone().unwrap_or_default(), monospace, w, italic);
        if let Some(&hit) = self.resolve_cache.get(&key) {
            return hit;
        }
        let result = self.resolve_uncached(fam.as_deref(), monospace, w, italic);
        self.resolve_cache.insert(key, result);
        result
    }

    fn resolve_uncached(
        &self,
        fam: Option<&str>,
        monospace: bool,
        w: u16,
        italic: bool,
    ) -> Option<usize> {
        if let Some(f) = fam {
            if let Some(id) = self.best_in_family(f, w, italic) {
                return Some(id);
            }
        }
        let configured = if monospace { &self.mono_family } else { &self.sans_family };
        if let Some(id) = self.best_in_family(configured, w, italic) {
            return Some(id);
        }
        let chain = if monospace { MONO_DEFAULTS } else { SANS_DEFAULTS };
        for f in chain {
            if let Some(id) = self.best_in_family(f, w, italic) {
                return Some(id);
            }
        }
        // Any face of the right pitch, then anything at all.
        self.best_where(|r| r.monospace == monospace, w, italic)
            .or_else(|| self.best_where(|_| true, w, italic))
    }

    fn best_in_family(&self, fam_lower: &str, w: u16, italic: bool) -> Option<usize> {
        self.best_where(|r| r.families.iter().any(|f| f == fam_lower), w, italic)
    }

    fn best_where(&self, pred: impl Fn(&FaceRecord) -> bool, w: u16, italic: bool) -> Option<usize> {
        self.records
            .iter()
            .enumerate()
            .filter(|(i, r)| !self.dead[*i] && pred(r))
            .min_by_key(|(i, r)| Self::style_rank(r, w, italic, *i))
            .map(|(i, _)| i)
    }

    /// Lower ranks first: style match, then closest weight, normal width,
    /// embedded faces winning ties, stable by id.
    fn style_rank(r: &FaceRecord, w: u16, italic: bool, id: usize) -> (bool, u16, u16, bool, usize) {
        (
            r.italic != italic,
            r.weight.abs_diff(w),
            r.width.abs_diff(5),
            matches!(r.source, FaceSource::File(_)),
            id,
        )
    }

    /// The face, loading + parsing it on first use. `None` disables the face.
    pub fn font(&mut self, id: usize) -> Option<&Font> {
        if self.dead.get(id).copied().unwrap_or(true) {
            return None;
        }
        if self.fonts[id].is_none() {
            let rec = &self.records[id];
            let parsed = match &rec.source {
                FaceSource::File(path) => match std::fs::read(path) {
                    Ok(data) => match Font::parse(data, rec.face_index) {
                        Ok(mut f) => {
                            if !rec.var_coords.is_empty() {
                                f.set_instance(&rec.var_coords);
                            }
                            Some(f)
                        }
                        Err(e) => {
                            eprintln!("[lntrn-text] disabling face {}: {e}", path.display());
                            None
                        }
                    },
                    Err(e) => {
                        eprintln!("[lntrn-text] disabling face {}: {e}", path.display());
                        None
                    }
                },
                FaceSource::Embedded => None, // embedded faces are always pre-parsed
            };
            match parsed {
                Some(f) => self.fonts[id] = Some(f),
                None => {
                    self.dead[id] = true;
                    // Cached decisions may point at the dead face.
                    self.resolve_cache.clear();
                    self.fallback_cache.clear();
                    return None;
                }
            }
        }
        self.fonts[id].as_ref()
    }

    /// Resolve `ch` in `primary`, falling back through the fallback chain and
    /// finally any covering face. Returns `(face, glyph)`; glyph 0 means
    /// nothing anywhere maps it → render the primary's `.notdef`.
    pub fn glyph_for(&mut self, primary: usize, ch: char, w: u16, italic: bool) -> (usize, u16) {
        if let Some(f) = self.font(primary) {
            let gid = f.glyph_index(ch);
            if gid != 0 {
                return (primary, gid);
            }
        }
        let key = (ch, w, italic);
        if let Some(&cached) = self.fallback_cache.get(&key) {
            if let Some(fid) = cached {
                let gid = self.font(fid).map_or(0, |f| f.glyph_index(ch));
                if gid != 0 {
                    return (fid, gid);
                }
            }
            return (primary, 0);
        }

        let c = ch as u32;
        let mut chosen = None;
        for fam in FALLBACK_FAMILIES {
            let Some(fid) = self.best_in_family(fam, w, italic) else {
                continue;
            };
            if fid == primary || !self.records[fid].covers(c) {
                continue;
            }
            if self.font(fid).is_some_and(|f| f.glyph_index(ch) != 0) {
                chosen = Some(fid);
                break;
            }
        }
        if chosen.is_none() {
            // Coverage tier: any face that claims the char, best style first.
            let mut candidates: Vec<usize> = (0..self.records.len())
                .filter(|&i| !self.dead[i] && i != primary && self.records[i].covers(c))
                .collect();
            candidates.sort_by_key(|&i| Self::style_rank(&self.records[i], w, italic, i));
            for fid in candidates {
                if self.font(fid).is_some_and(|f| f.glyph_index(ch) != 0) {
                    chosen = Some(fid);
                    break;
                }
            }
        }
        self.fallback_cache.insert(key, chosen);
        match chosen {
            Some(fid) => (fid, self.font(fid).map_or(0, |f| f.glyph_index(ch))),
            None => (primary, 0),
        }
    }
}

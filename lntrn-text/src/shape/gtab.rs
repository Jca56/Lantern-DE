//! Shared OpenType layout-table plumbing: the script → language system →
//! feature → lookup navigation that GPOS and GSUB have in common, plus
//! coverage and class-definition lookups. Built for GPOS in Phase 5; GSUB
//! (Phase 6) reuses everything here.
//!
//! Script selection is `latn` → `DFLT` → first, using the default LangSys —
//! proper per-run script itemization arrives with Phase 7 (UAX#24).

use std::collections::BTreeSet;

use crate::font::sfnt::{read_u16_at, read_u32_at};

/// Extension lookup types wrap a real subtable behind a 32-bit offset.
/// GPOS uses type 9, GSUB type 7.
const EXTENSION_POS: u16 = 9;
const EXTENSION_SUB: u16 = 7;

pub(crate) struct LookupRef {
    /// Lookup flag. IgnoreMarks (0x8) is honored in GPOS pair kerning;
    /// the other ignore-* bits and mark-filtering sets are still pending.
    pub flag: u16,
    /// (resolved lookup type, absolute subtable offset); extensions unwrapped.
    pub subtables: Vec<(u16, usize)>,
}

/// The GPOS lookups this engine applies, gathered once at font parse.
pub(crate) struct GposPlan {
    /// `kern` feature lookups, in LookupList order.
    pub kern: Vec<LookupRef>,
    /// `mark` + `mkmk` feature lookups (mark attachment).
    pub mark: Vec<LookupRef>,
}

impl GposPlan {
    /// Malformed tables yield an empty plan rather than an error — positioning
    /// is an enhancement, never a reason to reject a font.
    pub fn build(data: &[u8], gpos_off: usize, script: Option<[u8; 4]>) -> GposPlan {
        let groups: &[&[[u8; 4]]] = &[&[*b"kern"], &[*b"mark", *b"mkmk"]];
        match gather_lookups(data, gpos_off, groups, EXTENSION_POS, script) {
            Some((mut sets, _)) => {
                let mark = sets.pop().unwrap_or_default();
                let kern = sets.pop().unwrap_or_default();
                GposPlan { kern, mark }
            }
            None => GposPlan {
                kern: Vec::new(),
                mark: Vec::new(),
            },
        }
    }
}

/// The GSUB lookups this engine applies, gathered once at font parse.
/// Composition first, then the Arabic-style positional forms (masked to
/// glyphs the joining analysis tagged), then the ligature/contextual set.
pub(crate) struct GsubPlan {
    /// `ccmp` — glyph de/composition, applied before everything.
    pub ccmp: Vec<LookupRef>,
    /// Positional forms in application order: isol, fina, medi, init.
    pub positional: [Vec<LookupRef>; 4],
    /// `rlig` + `liga` + `clig` + `calt`, in LookupList order.
    pub subst: Vec<LookupRef>,
    /// Absolute LookupList offset — contextual rules reference arbitrary
    /// lookups by index, resolved on demand via [`resolve_lookup`].
    pub lookup_list: usize,
}

impl GsubPlan {
    pub fn build(data: &[u8], gsub_off: usize, script: Option<[u8; 4]>) -> GsubPlan {
        let groups: &[&[[u8; 4]]] = &[
            &[*b"ccmp"],
            &[*b"isol"],
            &[*b"fina"],
            &[*b"medi"],
            &[*b"init"],
            &[*b"liga", *b"clig", *b"calt", *b"rlig"],
        ];
        match gather_lookups(data, gsub_off, groups, EXTENSION_SUB, script) {
            Some((mut sets, lookup_list)) => {
                let subst = sets.pop().unwrap_or_default();
                let init = sets.pop().unwrap_or_default();
                let medi = sets.pop().unwrap_or_default();
                let fina = sets.pop().unwrap_or_default();
                let isol = sets.pop().unwrap_or_default();
                let ccmp = sets.pop().unwrap_or_default();
                GsubPlan {
                    ccmp,
                    positional: [isol, fina, medi, init],
                    subst,
                    lookup_list,
                }
            }
            None => GsubPlan {
                ccmp: Vec::new(),
                positional: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                subst: Vec::new(),
                lookup_list: 0,
            },
        }
    }

    /// Resolve a lookup referenced by index from a contextual rule.
    pub fn nested(&self, data: &[u8], index: u16) -> Option<LookupRef> {
        if self.lookup_list == 0 {
            return None;
        }
        resolve_lookup(data, self.lookup_list, index, EXTENSION_SUB)
    }
}

/// The script tags a layout table declares (for per-script plan building).
pub(crate) fn script_tags(data: &[u8], t: usize) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    let Ok(list_rel) = read_u16_at(data, t + 4) else {
        return out;
    };
    let script_list = t + list_rel as usize;
    let Ok(count) = read_u16_at(data, script_list) else {
        return out;
    };
    for i in 0..count as usize {
        if let Ok(tag) = read_u32_at(data, script_list + 2 + 6 * i) {
            out.push(tag.to_be_bytes());
        }
    }
    out
}

/// Walk script → default LangSys → features, bucketing each wanted feature
/// tag's lookup indices into its group. Returns the resolved lookups per
/// group (LookupList order within each) plus the LookupList offset.
/// `script`: exact script to use (None = latn → DFLT → first).
fn gather_lookups(
    data: &[u8],
    t: usize,
    groups: &[&[[u8; 4]]],
    ext_kind: u16,
    script: Option<[u8; 4]>,
) -> Option<(Vec<Vec<LookupRef>>, usize)> {
    let script_list = t + read_u16_at(data, t + 4).ok()? as usize;
    let feature_list = t + read_u16_at(data, t + 6).ok()? as usize;
    let lookup_list = t + read_u16_at(data, t + 8).ok()? as usize;

    let script = pick_script(data, script_list, script)?;
    let langsys = default_langsys(data, script)?;

    let mut sets: Vec<BTreeSet<u16>> = groups.iter().map(|_| BTreeSet::new()).collect();
    let required = read_u16_at(data, langsys + 2).ok()?;
    let count = read_u16_at(data, langsys + 4).ok()? as usize;
    let mut features: Vec<u16> = (0..count)
        .filter_map(|i| read_u16_at(data, langsys + 6 + 2 * i).ok())
        .collect();
    if required != 0xFFFF {
        features.push(required);
    }
    for fi in features {
        let rec = feature_list + 2 + fi as usize * 6;
        let tag = read_u32_at(data, rec).ok()?.to_be_bytes();
        let Some(group) = groups.iter().position(|tags| tags.contains(&tag)) else {
            continue;
        };
        let feat = feature_list + read_u16_at(data, rec + 4).ok()? as usize;
        let n = read_u16_at(data, feat + 2).ok()? as usize;
        for i in 0..n {
            sets[group].insert(read_u16_at(data, feat + 4 + 2 * i).ok()?);
        }
    }

    let resolved = sets
        .iter()
        .map(|set| {
            set.iter()
                .filter_map(|&i| resolve_lookup(data, lookup_list, i, ext_kind))
                .collect()
        })
        .collect();
    Some((resolved, lookup_list))
}

fn pick_script(data: &[u8], script_list: usize, want: Option<[u8; 4]>) -> Option<usize> {
    let count = read_u16_at(data, script_list).ok()? as usize;
    let (mut latn, mut dflt, mut first) = (None, None, None);
    for i in 0..count {
        let rec = script_list + 2 + 6 * i;
        let tag = read_u32_at(data, rec).ok()?.to_be_bytes();
        let off = script_list + read_u16_at(data, rec + 4).ok()? as usize;
        if want == Some(tag) {
            return Some(off);
        }
        if first.is_none() {
            first = Some(off);
        }
        match &tag {
            b"latn" => latn = Some(off),
            b"DFLT" => dflt = Some(off),
            _ => {}
        }
    }
    if want.is_some() {
        return None; // exact-script build: absent means no plan for it
    }
    latn.or(dflt).or(first)
}

fn default_langsys(data: &[u8], script: usize) -> Option<usize> {
    let default = read_u16_at(data, script).ok()? as usize;
    if default != 0 {
        return Some(script + default);
    }
    // No default LangSys: fall back to the first language-specific one.
    let count = read_u16_at(data, script + 2).ok()?;
    if count == 0 {
        return None;
    }
    Some(script + read_u16_at(data, script + 8).ok()? as usize)
}

pub(crate) fn resolve_lookup(
    data: &[u8],
    lookup_list: usize,
    idx: u16,
    ext_kind: u16,
) -> Option<LookupRef> {
    let off = lookup_list + read_u16_at(data, lookup_list + 2 + 2 * idx as usize).ok()? as usize;
    let kind = read_u16_at(data, off).ok()?;
    let flag = read_u16_at(data, off + 2).ok()?;
    let n = read_u16_at(data, off + 4).ok()? as usize;
    let mut subtables = Vec::with_capacity(n);
    for i in 0..n {
        let sub = off + read_u16_at(data, off + 6 + 2 * i).ok()? as usize;
        if kind == ext_kind {
            let real = read_u16_at(data, sub + 2).ok()?;
            let abs = sub + read_u32_at(data, sub + 4).ok()? as usize;
            subtables.push((real, abs));
        } else {
            subtables.push((kind, sub));
        }
    }
    Some(LookupRef { flag, subtables })
}

/// Coverage table lookup → coverage index of `gid`, if covered.
pub(crate) fn coverage_index(d: &[u8], cov: usize, gid: u16) -> Option<u16> {
    match read_u16_at(d, cov).ok()? {
        1 => {
            let n = read_u16_at(d, cov + 2).ok()? as usize;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = read_u16_at(d, cov + 4 + 2 * mid).ok()?;
                match g.cmp(&gid) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => return Some(mid as u16),
                }
            }
            None
        }
        2 => {
            let n = read_u16_at(d, cov + 2).ok()? as usize;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let rec = cov + 4 + 6 * mid;
                let start = read_u16_at(d, rec).ok()?;
                let end = read_u16_at(d, rec + 2).ok()?;
                if end < gid {
                    lo = mid + 1;
                } else if start > gid {
                    hi = mid;
                } else {
                    let base = read_u16_at(d, rec + 4).ok()?;
                    return Some(base + (gid - start));
                }
            }
            None
        }
        _ => None,
    }
}

/// ClassDef lookup → glyph's class (0 when unlisted, per spec).
pub(crate) fn glyph_class(d: &[u8], cd: usize, gid: u16) -> u16 {
    let Ok(format) = read_u16_at(d, cd) else {
        return 0;
    };
    match format {
        1 => {
            let start = read_u16_at(d, cd + 2).unwrap_or(0);
            let count = read_u16_at(d, cd + 4).unwrap_or(0);
            if gid >= start && gid < start.saturating_add(count) {
                read_u16_at(d, cd + 6 + 2 * (gid - start) as usize).unwrap_or(0)
            } else {
                0
            }
        }
        2 => {
            let n = read_u16_at(d, cd + 2).unwrap_or(0) as usize;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let rec = cd + 4 + 6 * mid;
                let start = read_u16_at(d, rec).unwrap_or(0);
                let end = read_u16_at(d, rec + 2).unwrap_or(0);
                if end < gid {
                    lo = mid + 1;
                } else if start > gid {
                    hi = mid;
                } else {
                    return read_u16_at(d, rec + 4).unwrap_or(0);
                }
            }
            0
        }
        _ => 0,
    }
}

//! GSUB application: glyph substitution on a same-font glyph run, before
//! positioning.
//!
//! Phase 6 scope: lookup types 1 (single), 2 (multiple), 4 (ligature),
//! 5 (context, formats 1–3), 6 (chained context, formats 1–3), with type 7
//! extensions pre-resolved at plan time. Contextual rules apply nested
//! lookups by index — this is how modern programming fonts (JetBrains Mono,
//! Inter) implement `calt` ligatures like `=>` and `!=`. Type 3 (alternate)
//! needs a selection UI and type 8 (reverse chained) is Phase 8 material.
//!
//! Every glyph carries its **cluster** (source byte offset). Substitutions
//! preserve it: single/context keep it, multiple copies it, ligatures keep
//! the first component's — so the layout engine can map glyphs back to text
//! positions (Phase 7 uses this to drop line-break opportunities that a
//! ligature swallowed).
//!
//! Simplification (documented): nested sequence-lookup records assume earlier
//! records in the same rule don't shift later sequence indices — true for the
//! length-preserving per-position substitutions real `calt` features use.

use super::arabic::Form;
use super::gtab::{coverage_index, glyph_class, GsubPlan, LookupRef};
use crate::font::sfnt::read_u16_at;

/// A glyph id + the byte offset of the source character it (still)
/// represents + its Arabic positional form tag.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Glyph {
    pub gid: u16,
    pub cluster: u32,
    pub form: Form,
}

/// Contextual rules can nest; real fonts stay shallow.
const MAX_NESTING: u8 = 4;

/// Apply the plan's substitution lookups to `glyphs`: composition, then the
/// positional forms (each masked to glyphs tagged with that form), then the
/// ligature/contextual set.
pub(crate) fn apply(d: &[u8], plan: &GsubPlan, glyphs: &mut Vec<Glyph>) {
    for lookup in &plan.ccmp {
        apply_lookup(d, plan, lookup, glyphs, None, 0);
    }
    let forms = [Form::Isol, Form::Fina, Form::Medi, Form::Init];
    for (bucket, form) in plan.positional.iter().zip(forms) {
        for lookup in bucket {
            apply_lookup(d, plan, lookup, glyphs, Some(form), 0);
        }
    }
    for lookup in &plan.subst {
        apply_lookup(d, plan, lookup, glyphs, None, 0);
    }
}

fn apply_lookup(
    d: &[u8],
    plan: &GsubPlan,
    lookup: &LookupRef,
    glyphs: &mut Vec<Glyph>,
    only_form: Option<Form>,
    depth: u8,
) {
    let mut i = 0;
    while i < glyphs.len() {
        if only_form.is_some_and(|f| glyphs[i].form != f) {
            i += 1;
            continue;
        }
        match apply_at(d, plan, lookup, glyphs, i, depth) {
            Some(consumed) => i += consumed.max(1),
            None => i += 1,
        }
    }
}

/// Try each subtable at position `i`; first match wins. Returns the number of
/// output glyphs the match covers (the caller skips past them).
fn apply_at(
    d: &[u8],
    plan: &GsubPlan,
    lookup: &LookupRef,
    glyphs: &mut Vec<Glyph>,
    i: usize,
    depth: u8,
) -> Option<usize> {
    for &(kind, sub) in &lookup.subtables {
        let result = match kind {
            1 => single(d, sub, glyphs, i),
            2 => multiple(d, sub, glyphs, i),
            4 => ligature(d, sub, glyphs, i),
            5 => context(d, plan, sub, glyphs, i, depth),
            6 => chained(d, plan, sub, glyphs, i, depth),
            _ => None,
        };
        if result.is_some() {
            return result;
        }
    }
    None
}

fn single(d: &[u8], sub: usize, glyphs: &mut [Glyph], i: usize) -> Option<usize> {
    let format = read_u16_at(d, sub).ok()?;
    let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
    let ci = coverage_index(d, cov, glyphs[i].gid)?;
    match format {
        1 => {
            let delta = read_u16_at(d, sub + 4).ok()?;
            glyphs[i].gid = glyphs[i].gid.wrapping_add(delta);
            Some(1)
        }
        2 => {
            let n = read_u16_at(d, sub + 4).ok()?;
            if ci >= n {
                return None;
            }
            glyphs[i].gid = read_u16_at(d, sub + 6 + 2 * ci as usize).ok()?;
            Some(1)
        }
        _ => None,
    }
}

fn multiple(d: &[u8], sub: usize, glyphs: &mut Vec<Glyph>, i: usize) -> Option<usize> {
    if read_u16_at(d, sub).ok()? != 1 {
        return None;
    }
    let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
    let ci = coverage_index(d, cov, glyphs[i].gid)?;
    let n = read_u16_at(d, sub + 4).ok()?;
    if ci >= n {
        return None;
    }
    let seq = sub + read_u16_at(d, sub + 6 + 2 * ci as usize).ok()? as usize;
    let count = read_u16_at(d, seq).ok()? as usize;
    if count == 0 {
        // Deletion — discouraged by the spec but legal.
        glyphs.remove(i);
        return Some(0);
    }
    let (cluster, form) = (glyphs[i].cluster, glyphs[i].form);
    glyphs[i].gid = read_u16_at(d, seq + 2).ok()?;
    for k in 1..count {
        let gid = read_u16_at(d, seq + 2 + 2 * k).ok()?;
        glyphs.insert(i + k, Glyph { gid, cluster, form });
    }
    Some(count)
}

fn ligature(d: &[u8], sub: usize, glyphs: &mut Vec<Glyph>, i: usize) -> Option<usize> {
    if read_u16_at(d, sub).ok()? != 1 {
        return None;
    }
    let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
    let ci = coverage_index(d, cov, glyphs[i].gid)?;
    let set_count = read_u16_at(d, sub + 4).ok()?;
    if ci >= set_count {
        return None;
    }
    let set = sub + read_u16_at(d, sub + 6 + 2 * ci as usize).ok()? as usize;
    let lig_count = read_u16_at(d, set).ok()? as usize;
    for l in 0..lig_count {
        let lig = set + read_u16_at(d, set + 2 + 2 * l).ok()? as usize;
        let lig_glyph = read_u16_at(d, lig).ok()?;
        let comp_count = read_u16_at(d, lig + 2).ok()? as usize;
        if comp_count == 0 || i + comp_count > glyphs.len() {
            continue;
        }
        let tail_matches = (1..comp_count)
            .all(|c| read_u16_at(d, lig + 4 + 2 * (c - 1)).is_ok_and(|g| g == glyphs[i + c].gid));
        if tail_matches {
            // The ligature keeps the first component's cluster — the whole
            // source span now maps to one glyph.
            glyphs[i].gid = lig_glyph;
            glyphs.drain(i + 1..i + comp_count);
            return Some(1);
        }
    }
    None
}

// ── Contextual substitution ─────────────────────────────────────────────────

/// How a rule's sequence values are matched against glyphs.
enum Matcher {
    /// Values are glyph ids.
    Glyph,
    /// Values are classes in this ClassDef.
    Class(usize),
    /// Values are coverage-table offsets relative to this subtable.
    Coverage(usize),
}

impl Matcher {
    fn matches(&self, d: &[u8], gid: u16, value: u16) -> bool {
        match *self {
            Matcher::Glyph => gid == value,
            Matcher::Class(cd) => glyph_class(d, cd, gid) == value,
            Matcher::Coverage(base) => coverage_index(d, base + value as usize, gid).is_some(),
        }
    }
}

/// Match `count` values at `values_off` against glyphs starting at `start`
/// (forward).
fn match_forward(
    d: &[u8],
    glyphs: &[Glyph],
    start: usize,
    values_off: usize,
    count: usize,
    m: &Matcher,
) -> bool {
    if start + count > glyphs.len() {
        return false;
    }
    (0..count).all(|k| {
        read_u16_at(d, values_off + 2 * k).is_ok_and(|v| m.matches(d, glyphs[start + k].gid, v))
    })
}

/// Match backtrack values against the glyphs before `i`, closest-first.
fn match_backtrack(
    d: &[u8],
    glyphs: &[Glyph],
    i: usize,
    values_off: usize,
    count: usize,
    m: &Matcher,
) -> bool {
    if count > i {
        return false;
    }
    (0..count).all(|k| {
        read_u16_at(d, values_off + 2 * k).is_ok_and(|v| m.matches(d, glyphs[i - 1 - k].gid, v))
    })
}

/// Apply a matched rule's sequence-lookup records (nested lookups at input
/// positions).
fn apply_records(
    d: &[u8],
    plan: &GsubPlan,
    glyphs: &mut Vec<Glyph>,
    i: usize,
    records_off: usize,
    count: usize,
    depth: u8,
) {
    for r in 0..count {
        let rec = records_off + 4 * r;
        let (Ok(seq_index), Ok(lookup_index)) = (read_u16_at(d, rec), read_u16_at(d, rec + 2))
        else {
            return;
        };
        let Some(lookup) = plan.nested(d, lookup_index) else {
            continue;
        };
        let pos = i + seq_index as usize;
        if pos < glyphs.len() {
            apply_at(d, plan, &lookup, glyphs, pos, depth + 1);
        }
    }
}

/// Rules in fmt 1/2 rule sets share a shape; chained rules add backtrack +
/// lookahead runs around the input. Returns consumed input length on match.
#[allow(clippy::too_many_arguments)]
fn try_rule_set(
    d: &[u8],
    plan: &GsubPlan,
    glyphs: &mut Vec<Glyph>,
    i: usize,
    set: usize,
    chained: bool,
    input_m: &Matcher,
    bt_m: &Matcher,
    la_m: &Matcher,
    depth: u8,
) -> Option<usize> {
    let rule_count = read_u16_at(d, set).ok()? as usize;
    'rules: for r in 0..rule_count {
        let rule = set + read_u16_at(d, set + 2 + 2 * r).ok()? as usize;
        let mut p = rule;
        let mut bt = (0usize, 0usize); // (offset, count)
        if chained {
            let n = read_u16_at(d, p).ok()? as usize;
            bt = (p + 2, n);
            p += 2 + 2 * n;
        }
        let input_count = read_u16_at(d, p).ok()? as usize;
        let (input_off, subst_info);
        if chained {
            let inputs = p + 2;
            p += 2 + 2 * (input_count.saturating_sub(1));
            let la_n = read_u16_at(d, p).ok()? as usize;
            let la_off = p + 2;
            p += 2 + 2 * la_n;
            let subst_count = read_u16_at(d, p).ok()? as usize;
            input_off = inputs;
            subst_info = (p + 2, subst_count);
            if !match_backtrack(d, glyphs, i, bt.0, bt.1, bt_m)
                || !match_forward(d, glyphs, i + input_count, la_off, la_n, la_m)
            {
                continue 'rules;
            }
        } else {
            let subst_count = read_u16_at(d, p + 2).ok()? as usize;
            input_off = p + 4;
            subst_info = (p + 4 + 2 * (input_count.saturating_sub(1)), subst_count);
        }
        if input_count == 0 || !match_forward(d, glyphs, i + 1, input_off, input_count - 1, input_m)
        {
            continue 'rules;
        }
        apply_records(d, plan, glyphs, i, subst_info.0, subst_info.1, depth);
        return Some(input_count);
    }
    None
}

/// Sequence context (type 5), formats 1–3.
fn context(
    d: &[u8],
    plan: &GsubPlan,
    sub: usize,
    glyphs: &mut Vec<Glyph>,
    i: usize,
    depth: u8,
) -> Option<usize> {
    if depth >= MAX_NESTING {
        return None;
    }
    match read_u16_at(d, sub).ok()? {
        1 => {
            let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
            let ci = coverage_index(d, cov, glyphs[i].gid)?;
            if ci >= read_u16_at(d, sub + 4).ok()? {
                return None;
            }
            let set = sub + read_u16_at(d, sub + 6 + 2 * ci as usize).ok()? as usize;
            let m = Matcher::Glyph;
            try_rule_set(d, plan, glyphs, i, set, false, &m, &m, &m, depth)
        }
        2 => {
            let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
            coverage_index(d, cov, glyphs[i].gid)?;
            let cd = sub + read_u16_at(d, sub + 4).ok()? as usize;
            let class = glyph_class(d, cd, glyphs[i].gid);
            if class >= read_u16_at(d, sub + 6).ok()? {
                return None;
            }
            let set_rel = read_u16_at(d, sub + 8 + 2 * class as usize).ok()?;
            if set_rel == 0 {
                return None;
            }
            let m = Matcher::Class(cd);
            try_rule_set(
                d,
                plan,
                glyphs,
                i,
                sub + set_rel as usize,
                false,
                &m,
                &m,
                &m,
                depth,
            )
        }
        3 => {
            let count = read_u16_at(d, sub + 2).ok()? as usize;
            let subst_count = read_u16_at(d, sub + 4).ok()? as usize;
            if count == 0 {
                return None;
            }
            let covs = sub + 6;
            let m = Matcher::Coverage(sub);
            if !match_forward(d, glyphs, i, covs, count, &m) {
                return None;
            }
            apply_records(d, plan, glyphs, i, covs + 2 * count, subst_count, depth);
            Some(count)
        }
        _ => None,
    }
}

/// Chained sequence context (type 6), formats 1–3.
fn chained(
    d: &[u8],
    plan: &GsubPlan,
    sub: usize,
    glyphs: &mut Vec<Glyph>,
    i: usize,
    depth: u8,
) -> Option<usize> {
    if depth >= MAX_NESTING {
        return None;
    }
    match read_u16_at(d, sub).ok()? {
        1 => {
            let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
            let ci = coverage_index(d, cov, glyphs[i].gid)?;
            if ci >= read_u16_at(d, sub + 4).ok()? {
                return None;
            }
            let set = sub + read_u16_at(d, sub + 6 + 2 * ci as usize).ok()? as usize;
            let m = Matcher::Glyph;
            try_rule_set(d, plan, glyphs, i, set, true, &m, &m, &m, depth)
        }
        2 => {
            let cov = sub + read_u16_at(d, sub + 2).ok()? as usize;
            coverage_index(d, cov, glyphs[i].gid)?;
            let bt_cd = sub + read_u16_at(d, sub + 4).ok()? as usize;
            let in_cd = sub + read_u16_at(d, sub + 6).ok()? as usize;
            let la_cd = sub + read_u16_at(d, sub + 8).ok()? as usize;
            let class = glyph_class(d, in_cd, glyphs[i].gid);
            if class >= read_u16_at(d, sub + 10).ok()? {
                return None;
            }
            let set_rel = read_u16_at(d, sub + 12 + 2 * class as usize).ok()?;
            if set_rel == 0 {
                return None;
            }
            try_rule_set(
                d,
                plan,
                glyphs,
                i,
                sub + set_rel as usize,
                true,
                &Matcher::Class(in_cd),
                &Matcher::Class(bt_cd),
                &Matcher::Class(la_cd),
                depth,
            )
        }
        3 => {
            let mut p = sub + 2;
            let bt_count = read_u16_at(d, p).ok()? as usize;
            let bt_covs = p + 2;
            p += 2 + 2 * bt_count;
            let in_count = read_u16_at(d, p).ok()? as usize;
            let in_covs = p + 2;
            p += 2 + 2 * in_count;
            let la_count = read_u16_at(d, p).ok()? as usize;
            let la_covs = p + 2;
            p += 2 + 2 * la_count;
            let subst_count = read_u16_at(d, p).ok()? as usize;
            let records = p + 2;
            if in_count == 0 {
                return None;
            }
            let m = Matcher::Coverage(sub);
            if !match_backtrack(d, glyphs, i, bt_covs, bt_count, &m)
                || !match_forward(d, glyphs, i, in_covs, in_count, &m)
                || !match_forward(d, glyphs, i + in_count, la_covs, la_count, &m)
            {
                return None;
            }
            apply_records(d, plan, glyphs, i, records, subst_count, depth);
            Some(in_count)
        }
        _ => None,
    }
}

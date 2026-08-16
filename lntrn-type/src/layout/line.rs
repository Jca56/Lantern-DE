//! Line building: fallback-aware glyph placement with greedy wrapping.
//!
//! Each source line shapes **whole** (so ligatures and kerning form across
//! word boundaries, like the HarfBuzz-based old stack), then wraps at UAX#14
//! break opportunities — but only those that survived shaping: an
//! opportunity is usable only where a glyph's cluster starts exactly at the
//! break's byte, so a ligature spanning a break point simply makes it
//! unbreakable. Japanese wraps between ideographs, hyphens break, NBSP
//! glues; units wider than the bound fall back to per-glyph breaks
//! (cosmic-text `Wrap::WordOrGlyph` behavior). Trailing whitespace may
//! overflow the bound (it never forces a wrap). `\n` always breaks.

use crate::font::db::{style_params, FontDb};
use crate::shape;
use crate::unicode;
use crate::{FontStyle, FontWeight};

/// One positioned glyph, relative to the layout origin. `y` is the baseline
/// offset (line index × line height + ascent).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedGlyph {
    pub face: u16,
    pub gid: u16,
    pub x: f32,
    pub y: f32,
}

/// A laid-out block of text, colorless and origin-relative (cacheable).
#[derive(Clone, Debug, Default)]
pub(crate) struct Layout {
    pub glyphs: Vec<PlacedGlyph>,
    /// Widest line's advance width — what `measure_width*` reports.
    pub width: f32,
}

/// Build a layout. `size` and `max_width` arrive pre-quantized. Returns an
/// empty layout when no font resolves at all.
#[allow(clippy::too_many_arguments)] // the full styling context, by design
pub(crate) fn build(
    db: &mut FontDb,
    monospace: bool,
    text: &str,
    size: f32,
    max_width: f32,
    weight: FontWeight,
    style: FontStyle,
    family: Option<&str>,
) -> Layout {
    let (w, italic) = style_params(weight, style);
    let Some(primary) = db.resolve(family, monospace, weight, style) else {
        return Layout::default();
    };
    let Some(ascent) = db.font(primary).map(|f| f.ascender_px(size)) else {
        return Layout::default();
    };
    let line_height = size * 1.2;

    let mut layout = Layout::default();
    let mut line_y = ascent;

    for src_line in text.split('\n') {
        let src_line = src_line.strip_suffix('\r').unwrap_or(src_line);

        // UAX#9: resolved levels (None = pure LTR fast path), then L4 —
        // mirrored characters swap before shaping so `(` renders as `)`
        // inside RTL runs (only when byte lengths match, keeping clusters
        // aligned; mirror pairs are same-length in practice).
        let bidi = unicode::bidi_resolve(src_line);
        let shaped_src: std::borrow::Cow<str> = match &bidi {
            Some(b) => {
                let mut s = String::with_capacity(src_line.len());
                for (&(_, level), c) in b.levels.iter().zip(src_line.chars()) {
                    match unicode::mirror(c) {
                        Some(m) if level % 2 == 1 && m.len_utf8() == c.len_utf8() => s.push(m),
                        _ => s.push(c),
                    }
                }
                std::borrow::Cow::Owned(s)
            }
            None => std::borrow::Cow::Borrowed(src_line),
        };

        // Shape the whole line once; wrap decisions and emission both come
        // from the shaped result, so they always agree.
        let (glyphs, _) = shape::shape_run(db, primary, &shaped_src, size, w, italic);
        let level_of = |cluster: u32| -> u8 {
            match &bidi {
                Some(b) => b
                    .levels
                    .binary_search_by_key(&(cluster as usize), |&(byte, _)| byte)
                    .map_or(0, |i| b.levels[i].1),
                None => 0,
            }
        };

        // Map UAX#14 break opportunities onto glyph indices. Usable only
        // where a glyph's cluster starts exactly at the break byte —
        // ligatures spanning a break swallow it.
        let mut segments: Vec<(usize, usize)> = Vec::new();
        let mut seg_start = 0usize;
        let mut gi = 0usize;
        for pos in unicode::break_opportunities(&shaped_src) {
            while gi < glyphs.len() && (glyphs[gi].cluster as usize) < pos {
                gi += 1;
            }
            if gi > seg_start && gi < glyphs.len() && glyphs[gi].cluster as usize == pos {
                segments.push((seg_start, gi));
                seg_start = gi;
            }
        }
        if seg_start < glyphs.len() {
            segments.push((seg_start, glyphs.len()));
        }

        // Greedy wrap in logical order: assign glyphs to rows.
        let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
        let mut pen = 0.0f32;
        for (start, end) in segments {
            // Trailing whitespace may overhang the bound (LB18 breaks after
            // spaces), so the wrap decision uses the segment's core width.
            let mut core_end = end;
            while core_end > start {
                let cl = glyphs[core_end - 1].cluster as usize;
                let ws = shaped_src[cl..].chars().next().is_some_and(char::is_whitespace);
                if ws {
                    core_end -= 1;
                } else {
                    break;
                }
            }
            let core_width: f32 = glyphs[start..core_end].iter().map(|g| g.advance).sum();
            if pen > 0.0 && pen + core_width > max_width {
                rows.push(Vec::new());
                pen = 0.0;
            }
            #[allow(clippy::needless_range_loop)] // k also feeds the core_end bound check
            for k in start..end {
                // Glyph-level break for segments wider than the whole bound
                // (never triggered by the overhanging trailing spaces).
                if k < core_end && pen > 0.0 && pen + glyphs[k].advance > max_width {
                    rows.push(Vec::new());
                    pen = 0.0;
                }
                rows.last_mut().unwrap().push(k);
                pen += glyphs[k].advance;
            }
        }

        // Emit each row: L2 reorder at *group* granularity (a base glyph plus
        // its zero-advance marks move as one unit, keeping mark offsets
        // valid), then assign pen positions in visual order.
        for row in rows {
            let mut groups: Vec<(Vec<usize>, u8)> = Vec::new();
            for &k in &row {
                let sg = &glyphs[k];
                if groups.is_empty() || sg.advance > 0.0 {
                    groups.push((vec![k], level_of(sg.cluster)));
                } else {
                    groups.last_mut().unwrap().0.push(k);
                }
            }
            let order: Vec<usize> = if bidi.is_some() {
                let levels: Vec<u8> = groups.iter().map(|&(_, l)| l).collect();
                unicode::reorder(&levels)
            } else {
                (0..groups.len()).collect()
            };
            let mut pen = 0.0f32;
            for &gidx in &order {
                for &k in &groups[gidx].0 {
                    let sg = &glyphs[k];
                    layout.glyphs.push(PlacedGlyph {
                        face: sg.face,
                        gid: sg.gid,
                        x: pen + sg.x_off,
                        y: line_y + sg.y_off,
                    });
                    pen += sg.advance;
                }
            }
            layout.width = layout.width.max(pen);
            line_y += line_height;
        }
    }
    layout
}

//! Shaping: characters → positioned glyphs.
//!
//! Phase 5 scope: the "simple with positioning" path — per-char cmap +
//! fallback resolution, then GPOS kerning / single adjustments / mark
//! attachment (or legacy `kern`) applied per same-font glyph run. GSUB
//! substitution (Phase 6) and proper script itemization (Phase 7) slot in
//! here later.
//!
//! Positioning applies within a token (word or whitespace run); pairs that
//! straddle token boundaries involve a space glyph, which real fonts don't
//! kern against.

pub(crate) mod gpos;
pub(crate) mod gsub;
pub(crate) mod gtab;
pub(crate) mod kern;

use crate::font::db::FontDb;

/// One positioned glyph in pixels, relative to the token's pen start.
pub(crate) struct ShapedGlyph {
    pub face: u16,
    pub gid: u16,
    /// Draw offset from the pen (kern placement / mark attachment).
    pub x_off: f32,
    /// Draw offset, screen-space (y down).
    pub y_off: f32,
    /// Pen advance.
    pub advance: f32,
}

/// Shape one token: resolve each char (with per-glyph fallback), then apply
/// positioning per same-font run. Returns the glyphs and the token's total
/// advance width.
pub(crate) fn shape_token(
    db: &mut FontDb,
    primary: usize,
    token: &str,
    size: f32,
    weight: u16,
    italic: bool,
) -> (Vec<ShapedGlyph>, f32) {
    let mut resolved: Vec<(usize, u16)> = Vec::new();
    for ch in token.chars() {
        if ch == '\r' {
            continue;
        }
        resolved.push(db.glyph_for(primary, ch, weight, italic));
    }

    let mut out = Vec::with_capacity(resolved.len());
    let mut total = 0.0f32;
    let mut i = 0;
    while i < resolved.len() {
        let fid = resolved[i].0;
        let mut j = i;
        while j < resolved.len() && resolved[j].0 == fid {
            j += 1;
        }
        let Some(font) = db.font(fid) else {
            i = j;
            continue;
        };
        let scale = font.scale(size);
        // GSUB first (ligatures may merge glyphs), then GPOS on the result.
        let mut gids: Vec<u16> = resolved[i..j].iter().map(|&(_, gid)| gid).collect();
        font.substitute(&mut gids);
        let mut run: Vec<gpos::GlyphPos> = gids
            .into_iter()
            .map(|gid| gpos::GlyphPos {
                gid,
                x_adv: font.advance_units(gid) as i32,
                x_off: 0,
                y_off: 0,
            })
            .collect();
        font.position(&mut run);
        for gp in &run {
            let advance = gp.x_adv as f32 * scale;
            out.push(ShapedGlyph {
                face: fid as u16,
                gid: gp.gid,
                x_off: gp.x_off as f32 * scale,
                // GPOS y is up; screen y is down.
                y_off: -(gp.y_off as f32) * scale,
                advance,
            });
            total += advance;
        }
        i = j;
    }
    (out, total)
}

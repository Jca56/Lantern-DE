//! Shaping: characters → positioned glyphs.
//!
//! Pipeline per same-font run: grapheme-cluster-aware fallback resolution →
//! GSUB substitution (ligatures/contextual, Phase 6) → advances → GPOS
//! positioning (kerning/marks, Phase 5). Text shapes as whole lines so
//! ligatures and kerning can form across word boundaries, exactly like the
//! HarfBuzz-based old stack; every output glyph carries its source byte
//! offset (**cluster**) so layout can map break opportunities back onto the
//! glyph stream. Proper per-script feature selection joins in Phase 8.

pub(crate) mod arabic;
pub(crate) mod gpos;
pub(crate) mod gsub;
pub(crate) mod gtab;
pub(crate) mod kern;

use crate::font::db::FontDb;
use crate::unicode;
use arabic::Form;

/// One positioned glyph in pixels, relative to the run's pen start.
pub(crate) struct ShapedGlyph {
    pub face: u16,
    pub gid: u16,
    /// Byte offset of the source character in the shaped text. Ligatures
    /// keep their first component's offset; clusters are non-decreasing.
    pub cluster: u32,
    /// Draw offset from the pen (kern placement / mark attachment).
    pub x_off: f32,
    /// Draw offset, screen-space (y down).
    pub y_off: f32,
    /// Pen advance.
    pub advance: f32,
}

/// OpenType script tag for a font run: the first character with a specific
/// script decides (Common/Inherited/Unknown keep scanning).
fn run_script(text: &str, run: &[(usize, gsub::Glyph)]) -> [u8; 4] {
    use crate::unicode::Script;
    for &(_, g) in run {
        let Some(c) = text[g.cluster as usize..].chars().next() else {
            continue;
        };
        let tag = match crate::unicode::script(c) {
            Script::Latin => *b"latn",
            Script::Arabic => *b"arab",
            Script::Hebrew => *b"hebr",
            Script::Han => *b"hani",
            Script::Hiragana | Script::Katakana => *b"kana",
            Script::Hangul => *b"hang",
            Script::Cyrillic => *b"cyrl",
            Script::Greek => *b"grek",
            Script::Thai => *b"thai",
            Script::Common | Script::Inherited | Script::Unknown => continue,
            _ => *b"DFLT",
        };
        return tag;
    }
    *b"DFLT"
}

/// Shape `text` (typically one line): resolve each grapheme cluster (with
/// per-glyph fallback), then substitute + position per same-font run.
/// Returns the glyphs and total advance width.
pub(crate) fn shape_run(
    db: &mut FontDb,
    primary: usize,
    text: &str,
    size: f32,
    weight: u16,
    italic: bool,
) -> (Vec<ShapedGlyph>, f32) {
    // Arabic-style positional forms (empty for non-cursive text).
    let forms = arabic::joining_forms(text);
    let form_at = |byte: u32| {
        forms
            .binary_search_by_key(&byte, |&(b, _)| b)
            .map_or(Form::None, |i| forms[i].1)
    };

    // Resolve per grapheme cluster: the base character picks the font and the
    // rest of the cluster (combining marks, ZWJ tails) tries that same font
    // first, so marks anchor to their base instead of landing in a different
    // fallback face. "Soft" characters (default-ignorable controls, joiners,
    // variation selectors) keep their glyph only when the cluster's font maps
    // one — emoji fonts map ZWJ/VS16 for their GSUB sequences, text fonts
    // usually don't — and never trigger a fallback search of their own.
    let soft = |c: char| unicode::is_default_ignorable(c) || matches!(c as u32, 0xFE00..=0xFE0F);
    let mut resolved: Vec<(usize, gsub::Glyph)> = Vec::new();
    let mut cluster_base = 0usize;
    for cluster in crate::unicode::graphemes(text) {
        let start = cluster_base;
        cluster_base += cluster.len();
        let mut chars = cluster.char_indices().filter(|&(_, c)| c != '\r');
        let Some((_, base)) = chars.next() else {
            continue;
        };
        let fid = if soft(base) {
            let gid = db.font(primary).map_or(0, |f| f.glyph_index(base));
            if gid != 0 {
                resolved.push((
                    primary,
                    gsub::Glyph {
                        gid,
                        cluster: start as u32,
                        form: form_at(start as u32),
                    },
                ));
            }
            primary
        } else {
            let (fid, gid) = db.glyph_for(primary, base, weight, italic);
            resolved.push((
                fid,
                gsub::Glyph {
                    gid,
                    cluster: start as u32,
                    form: form_at(start as u32),
                },
            ));
            fid
        };
        for (ci, c) in chars {
            let offset = (start + ci) as u32;
            if soft(c) {
                let gid = db.font(fid).map_or(0, |f| f.glyph_index(c));
                if gid != 0 {
                    resolved.push((
                        fid,
                        gsub::Glyph {
                            gid,
                            cluster: offset,
                            form: form_at(offset),
                        },
                    ));
                }
            } else {
                let (f, g) = db.glyph_for(fid, c, weight, italic);
                resolved.push((
                    f,
                    gsub::Glyph {
                        gid: g,
                        cluster: offset,
                        form: form_at(offset),
                    },
                ));
            }
        }
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
        // The run's OT script tag selects which per-script feature plan
        // applies (Arabic forms live under `arab`, not the default).
        let script = run_script(text, &resolved[i..j]);
        // GSUB first (ligatures may merge glyphs), then GPOS on the result.
        let mut glyphs: Vec<gsub::Glyph> = resolved[i..j].iter().map(|&(_, g)| g).collect();
        font.substitute(&mut glyphs, script);
        let mut run: Vec<gpos::GlyphPos> = glyphs
            .iter()
            .map(|g| gpos::GlyphPos {
                gid: g.gid,
                x_adv: font.advance_units(g.gid),
                x_off: 0,
                y_off: 0,
            })
            .collect();
        font.position(&mut run, script);
        for (g, gp) in glyphs.iter().zip(&run) {
            let advance = gp.x_adv as f32 * scale;
            out.push(ShapedGlyph {
                face: fid as u16,
                gid: gp.gid,
                cluster: g.cluster,
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

//! Line building: fallback-aware glyph placement with greedy wrapping.
//!
//! Wrap behavior matches cosmic-text's `Wrap::WordOrGlyph` default the
//! glyphon wrapper relied on: break lines at whitespace boundaries, and fall
//! back to per-glyph breaks for words wider than the wrap bound. Trailing
//! whitespace may overflow the bound (it never forces a wrap). `\n` always
//! breaks; `\r` is ignored.

use crate::font::db::{style_params, FontDb};
use crate::shape;
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
    let mut pen = 0.0f32;
    let mut line_y = ascent;

    for (li, src_line) in text.split('\n').enumerate() {
        if li > 0 {
            layout.width = layout.width.max(pen);
            pen = 0.0;
            line_y += line_height;
        }
        for token in tokens(src_line) {
            let is_space = token.chars().next().is_some_and(char::is_whitespace);
            // Each token shapes once (fallback + GPOS kerning/marks); the
            // shaped width drives both wrapping and emission so they agree.
            let (glyphs, token_width) = shape::shape_token(db, primary, token, size, w, italic);
            if !is_space && pen > 0.0 && pen + token_width > max_width {
                layout.width = layout.width.max(pen);
                pen = 0.0;
                line_y += line_height;
            }
            for sg in glyphs {
                // Glyph-level break for words wider than the whole bound.
                if !is_space && pen > 0.0 && pen + sg.advance > max_width {
                    layout.width = layout.width.max(pen);
                    pen = 0.0;
                    line_y += line_height;
                }
                layout.glyphs.push(PlacedGlyph {
                    face: sg.face,
                    gid: sg.gid,
                    x: pen + sg.x_off,
                    y: line_y + sg.y_off,
                });
                pen += sg.advance;
            }
        }
    }
    layout.width = layout.width.max(pen);
    layout
}

/// Split a line into alternating whitespace / non-whitespace runs.
fn tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space: Option<bool> = None;
    for (i, c) in s.char_indices() {
        let space = c.is_whitespace();
        match in_space {
            None => in_space = Some(space),
            Some(prev) if prev != space => {
                out.push(&s[start..i]);
                start = i;
                in_space = Some(space);
            }
            _ => {}
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn tokenizes_runs() {
        assert_eq!(tokens("ab  cd e"), vec!["ab", "  ", "cd", " ", "e"]);
        assert_eq!(tokens("  lead"), vec!["  ", "lead"]);
        assert_eq!(tokens(""), Vec::<&str>::new());
    }
}

//! UAX#29 extended grapheme cluster segmentation.
//!
//! Implements GB1–GB13 + GB999: CRLF, Hangul jamo composition, Extend/ZWJ
//! continuation, spacing marks, prepends, emoji ZWJ sequences (GB11), and
//! regional-indicator (flag) pairing. GB9c (Unicode 15.1 Indic conjunct
//! clusters) needs the Indic_Conjunct_Break property and is deferred to the
//! complex-scripts phase.

use super::{grapheme_break, is_extended_pictographic, GraphemeBreak as GB};

/// Byte index of the grapheme boundary after the cluster starting at `start`.
/// Returns `text.len()` when the cluster runs to the end. `start` must be a
/// char boundary.
pub fn next_grapheme_boundary(text: &str, start: usize) -> usize {
    let mut iter = text[start..].char_indices();
    let Some((_, first)) = iter.next() else {
        return text.len();
    };
    let mut prev = grapheme_break(first);
    // GB11 state: have we seen ExtPict (Extend)* — a ZWJ then keeps the
    // sequence eligible to absorb the next ExtPict.
    let mut emoji_pending = is_extended_pictographic(first);
    // GB12/13: count of consecutive regional indicators ending at prev.
    let mut ri_count = u32::from(prev == GB::RegionalIndicator);

    for (i, c) in iter {
        let cur = grapheme_break(c);
        let boundary = match (prev, cur) {
            (GB::CR, GB::LF) => false,                          // GB3
            (GB::Control | GB::CR | GB::LF, _) => true,         // GB4
            (_, GB::Control | GB::CR | GB::LF) => true,         // GB5
            (GB::L, GB::L | GB::V | GB::LV | GB::LVT) => false, // GB6
            (GB::LV | GB::V, GB::V | GB::T) => false,           // GB7
            (GB::LVT | GB::T, GB::T) => false,                  // GB8
            (_, GB::Extend | GB::ZWJ) => false,                 // GB9
            (_, GB::SpacingMark) => false,                      // GB9a
            (GB::Prepend, _) => false,                          // GB9b
            (GB::ZWJ, _) if emoji_pending && is_extended_pictographic(c) => false, // GB11
            (GB::RegionalIndicator, GB::RegionalIndicator) if ri_count % 2 == 1 => false, // GB12/13
            _ => true,                                          // GB999
        };
        if boundary {
            return start + i;
        }
        emoji_pending = if is_extended_pictographic(c) {
            true
        } else {
            emoji_pending && matches!(cur, GB::Extend | GB::ZWJ)
        };
        ri_count = if cur == GB::RegionalIndicator {
            ri_count + 1
        } else {
            0
        };
        prev = cur;
    }
    text.len()
}

/// Iterator over a string's extended grapheme clusters.
pub struct Graphemes<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Iterator for Graphemes<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.pos >= self.text.len() {
            return None;
        }
        let end = next_grapheme_boundary(self.text, self.pos);
        let cluster = &self.text[self.pos..end];
        self.pos = end;
        Some(cluster)
    }
}

/// Split `text` into extended grapheme clusters — the units a cursor and
/// editing operations should treat as single characters.
pub fn graphemes(text: &str) -> Graphemes<'_> {
    Graphemes { text, pos: 0 }
}

#[cfg(test)]
mod tests {
    use super::graphemes;

    fn clusters(s: &str) -> Vec<&str> {
        graphemes(s).collect()
    }

    #[test]
    fn ascii_and_combining() {
        assert_eq!(clusters("abc"), vec!["a", "b", "c"]);
        // e + combining acute = one cluster.
        assert_eq!(clusters("e\u{0301}x"), vec!["e\u{0301}", "x"]);
        // Stacked marks stay attached.
        assert_eq!(clusters("a\u{0301}\u{0302}"), vec!["a\u{0301}\u{0302}"]);
    }

    #[test]
    fn crlf_is_one_cluster() {
        assert_eq!(clusters("a\r\nb"), vec!["a", "\r\n", "b"]);
    }

    #[test]
    fn hangul_jamo_compose() {
        // L + V + T jamo sequence forms one syllable cluster.
        assert_eq!(
            clusters("\u{1100}\u{1161}\u{11A8}"),
            vec!["\u{1100}\u{1161}\u{11A8}"]
        );
    }

    #[test]
    fn emoji_zwj_sequence() {
        // Family: man ZWJ woman ZWJ girl — one cluster.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(clusters(family), vec![family]);
        // Skin-tone modifier (Extend) stays attached.
        assert_eq!(
            clusters("\u{1F44B}\u{1F3FD}!"),
            vec!["\u{1F44B}\u{1F3FD}", "!"]
        );
    }

    #[test]
    fn regional_indicator_pairs() {
        // Two flags = two clusters of two RIs each.
        let flags = "\u{1F1FA}\u{1F1F8}\u{1F1E9}\u{1F1EA}";
        assert_eq!(
            clusters(flags),
            vec!["\u{1F1FA}\u{1F1F8}", "\u{1F1E9}\u{1F1EA}"]
        );
    }
}

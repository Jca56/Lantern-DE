//! Case-insensitive text search with byte-accurate offset mapping.
//!
//! `str::to_lowercase()` can change a character's byte length (Turkish
//! `İ` → `i̇` grows 2→3 bytes, `ẞ` → `ß` shrinks 3→2), so finding matches in
//! a lowercased copy and reusing those byte offsets on the original line
//! skews every position after such a character — planting the cursor mid-char
//! (panic) or replacing the wrong bytes. Here the haystack is folded
//! char-by-char with a map from every folded byte back to the original
//! char's start offset, so returned ranges always land on real boundaries.

/// Case-fold a needle the same way `find_in_line` folds its haystack
/// (char-by-char), so both sides agree even where `str::to_lowercase`'s
/// context-sensitive rules (e.g. Greek final sigma) would differ.
pub fn fold(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

/// Append `needle`'s occurrences in `line` to `out` as byte ranges into the
/// ORIGINAL line. For case-insensitive searches the needle must already be
/// folded with [`fold`]. Every returned offset is a char boundary of `line`.
pub fn find_in_line(line: &str, needle: &str, case_sensitive: bool, out: &mut Vec<(usize, usize)>) {
    if needle.is_empty() {
        return;
    }

    if case_sensitive {
        let mut from = 0usize;
        while let Some(pos) = line[from..].find(needle) {
            let start = from + pos;
            out.push((start, start + needle.len()));
            from = start + needle.len();
        }
        return;
    }

    // Fold the haystack, remembering which original char every folded byte
    // came from. map.len() == folded.len() + 1 (sentinel = line.len()).
    let mut folded = String::new();
    let mut map: Vec<usize> = Vec::new();
    for (oi, ch) in line.char_indices() {
        let before = folded.len();
        for lc in ch.to_lowercase() {
            folded.push(lc);
        }
        for _ in before..folded.len() {
            map.push(oi);
        }
    }
    map.push(line.len());

    let mut from = 0usize;
    while let Some(pos) = folded[from..].find(needle) {
        let fs = from + pos;
        let fe = fs + needle.len();
        let start = map[fs];
        // A match ending inside one char's multi-byte fold expansion still
        // covers that whole original char (matching "i" inside "İ" selects
        // the İ) — otherwise the range would exclude a char it consumed.
        let mut end = if fe < folded.len() && map[fe] == map[fe - 1] {
            next_boundary(line, map[fe])
        } else {
            map[fe]
        };
        if end <= start {
            end = next_boundary(line, start);
        }
        out.push((start, end));
        from = fe;
    }
}

fn next_boundary(s: &str, i: usize) -> usize {
    let mut p = (i + 1).min(s.len());
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(line: &str, needle: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        find_in_line(line, &fold(needle), false, &mut out);
        out
    }

    /// Every returned range must slice the original line without panicking.
    fn assert_sliceable(line: &str, ranges: &[(usize, usize)]) {
        for &(s, e) in ranges {
            assert!(
                line.is_char_boundary(s) && line.is_char_boundary(e),
                "bad range {s}..{e} in {line:?}"
            );
            let _ = &line[s..e];
        }
    }

    #[test]
    fn ascii_case_insensitive() {
        let m = find("Hello hello HELLO", "hello");
        assert_eq!(m, vec![(0, 5), (6, 11), (12, 17)]);
    }

    /// İ (U+0130) folds 2→3 bytes; offsets after it must not skew.
    #[test]
    fn growing_fold_char_before_match() {
        let line = "aİb→c";
        let m = find(line, "→");
        assert_sliceable(line, &m);
        assert_eq!(&line[m[0].0..m[0].1], "→");
    }

    /// ẞ (U+1E9E) folds 3→2 bytes; offsets after it must not skew either.
    #[test]
    fn shrinking_fold_char_before_match() {
        let line = "aẞb→c";
        let m = find(line, "→");
        assert_sliceable(line, &m);
        assert_eq!(&line[m[0].0..m[0].1], "→");
    }

    /// Matching inside a fold expansion still yields boundary-safe ranges.
    #[test]
    fn match_inside_expansion() {
        let line = "aİb";
        let m = find(line, "i");
        assert_sliceable(line, &m);
        assert_eq!(&line[m[0].0..m[0].1], "İ");
    }

    #[test]
    fn case_sensitive_exact() {
        let mut out = Vec::new();
        find_in_line("aXbXc", "X", true, &mut out);
        assert_eq!(out, vec![(1, 2), (3, 4)]);
    }
}

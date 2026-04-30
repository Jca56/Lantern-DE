//! Subsequence-based fuzzy matcher.
//!
//! Scores how well a query matches a candidate string. Designed for app
//! launchers, file pickers, command palettes — short queries against
//! short candidate strings, where speed matters more than perfection.
//!
//! Algorithm (rough):
//! 1. Walk the query left-to-right. For each query char, find the next
//!    occurrence in the candidate (case-insensitive). If any char is
//!    missing, return `None` (no match).
//! 2. Score each match position with bonuses:
//!    - **Prefix match**: matches at the very start of the candidate are
//!      worth a lot.
//!    - **Word-boundary match**: a query char that lands on a candidate
//!      char immediately preceded by `_`, `-`, ` `, `.`, `/`, or a
//!      lowercase→uppercase transition (camelCase) earns a bonus.
//!    - **Consecutive match**: when the previous query char also matched
//!      the previous candidate char, earn a bonus (encourages "fox" to
//!      match "**fox**glove" much higher than "**f**iref**ox**").
//!    - **Gap penalty**: each unmatched candidate char between matches
//!      costs a small amount.
//! 3. Normalize the raw score into `[0.0, 1.0]` so different candidates
//!    are comparable.
//!
//! No allocation in the inner loop. Single pass over the candidate's bytes.

/// Score a query against a candidate string. Returns `None` if the query
/// is not a subsequence of the candidate (case-insensitive). Returns
/// `Some(score)` in `[0.0, 1.0]` otherwise.
///
/// An empty query returns `Some(0.0)` — caller decides what to show in
/// the no-query state. (Phase 2.4 wires up pinned + recents for that.)
pub fn score(query: &str, candidate: &str) -> Option<f32> {
    if query.is_empty() {
        return Some(0.0);
    }
    if candidate.is_empty() {
        return None;
    }

    let q = query.as_bytes();
    let c = candidate.as_bytes();

    let mut raw: f32 = 0.0;
    let mut last_match: Option<usize> = None;
    let mut ci: usize = 0;

    for (qi, &qc) in q.iter().enumerate() {
        let qc_lo = qc.to_ascii_lowercase();
        let mut found = None;

        while ci < c.len() {
            let cc = c[ci];
            if cc.to_ascii_lowercase() == qc_lo {
                found = Some(ci);
                break;
            }
            ci += 1;
        }

        let Some(pos) = found else {
            return None;
        };

        let mut delta: f32 = MATCH_SCORE;

        // Bonus: prefix match (very strong signal).
        if pos == 0 {
            delta += PREFIX_BONUS;
        }
        // Bonus: word-boundary match.
        else if is_word_boundary(c, pos) {
            delta += WORD_BOUNDARY_BONUS;
        }

        // Bonus: consecutive match.
        if let Some(last) = last_match {
            if pos == last + 1 {
                delta += CONSECUTIVE_BONUS;
            }
        }

        // Penalty: gap before this match (only counts past the first char).
        if qi > 0 {
            if let Some(last) = last_match {
                let gap = pos - last - 1;
                delta -= GAP_PENALTY * gap as f32;
            }
        }

        raw += delta;
        last_match = Some(pos);
        ci = pos + 1;
    }

    // Bonus: candidate is short relative to the query (tighter fit).
    let length_ratio = q.len() as f32 / c.len() as f32;
    raw += LENGTH_RATIO_BONUS * length_ratio;

    // Normalize to [0, 1]. Max comes from an exact-prefix match of length
    // q on a candidate of equal length:
    //   PREFIX (first char only) + (q-1)*(MATCH + CONSECUTIVE) + MATCH (first)
    //   + LENGTH_RATIO_BONUS
    let max = PREFIX_BONUS
        + MATCH_SCORE
        + (q.len() as f32 - 1.0).max(0.0) * (MATCH_SCORE + CONSECUTIVE_BONUS)
        + LENGTH_RATIO_BONUS;
    Some((raw / max).clamp(0.0, 1.0))
}

/// Score a query against multiple candidate strings (e.g., name, exec,
/// keywords). Returns the best score among them, or `None` if none match.
#[allow(dead_code)] // utility for Phase 2.7 providers (commands/files)
pub fn score_any<S: AsRef<str>>(query: &str, candidates: &[S]) -> Option<f32> {
    let mut best: Option<f32> = None;
    for cand in candidates {
        if let Some(s) = score(query, cand.as_ref()) {
            best = Some(best.map_or(s, |b| b.max(s)));
        }
    }
    best
}

// ── Tunables ────────────────────────────────────────────────────────────────

const MATCH_SCORE: f32 = 1.0;
const PREFIX_BONUS: f32 = 4.0;
const WORD_BOUNDARY_BONUS: f32 = 1.5;
/// Higher than WORD_BOUNDARY_BONUS so contiguous matches in the middle
/// of a string ("fox" in "fire**fox**") still beat scattered matches at
/// word boundaries ("**f**_**o**_**x**"). Empirically this is the most
/// important signal for app-launcher feel.
const CONSECUTIVE_BONUS: f32 = 3.0;
const GAP_PENALTY: f32 = 0.1;
const LENGTH_RATIO_BONUS: f32 = 0.5;

// ── Word-boundary detection ─────────────────────────────────────────────────

fn is_word_boundary(s: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = s[i - 1];
    let curr = s[i];
    matches!(prev, b'_' | b'-' | b' ' | b'.' | b'/' | b'\t')
        || (prev.is_ascii_lowercase() && curr.is_ascii_uppercase())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_scores_zero() {
        assert_eq!(score("", "anything"), Some(0.0));
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(score("xyz", "firefox"), None);
    }

    #[test]
    fn exact_prefix_beats_subsequence() {
        let prefix = score("fire", "firefox").unwrap();
        let subseq = score("fire", "configurer").unwrap();
        assert!(prefix > subseq, "prefix {prefix} should beat subseq {subseq}");
    }

    #[test]
    fn case_insensitive() {
        assert!(score("FIRE", "firefox").is_some());
        assert!(score("fire", "FIREFOX").is_some());
    }

    #[test]
    fn consecutive_run_beats_scattered() {
        let consec = score("fox", "foxglove").unwrap();
        let scattered = score("fox", "f_o_x").unwrap();
        assert!(consec > scattered);
    }

    #[test]
    fn word_boundary_beats_middle() {
        let boundary = score("fm", "file-manager").unwrap();
        let middle = score("fm", "fooomanager").unwrap();
        assert!(boundary > middle);
    }
}

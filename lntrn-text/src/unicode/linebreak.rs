//! UAX#14 line breaking: where a line may wrap.
//!
//! Implements the rule cascade LB2–LB31 over resolved break classes,
//! including the space-run rules (LB8/14/16/17), CM/ZWJ transparency
//! (LB9/10), Korean jamo (LB26/27), regional-indicator pairing (LB30a), and
//! the CJK classes that make Japanese/Chinese text wrap between ideographs.
//!
//! Documented simplifications (each errs toward *fewer* break points, which
//! only ever makes wrapping more conservative):
//! - LB1: SA (Southeast Asian, needs dictionary segmentation) → AL;
//!   CJ → NS (strict kinsoku).
//! - LB15a/b (Unicode 15.1 quote refinement) approximated by classic
//!   LB19 (`× QU`, `QU ×`) + `QU SP* × OP`.
//! - LB25 uses the spec's simplified number pattern; LB28a covers the core
//!   Brahmic pairs; LB30's East-Asian-width refinement is dropped.

use super::{line_break_class, LineBreakClass as LB};

/// LB1 class resolution.
fn resolved(c: char) -> LB {
    match line_break_class(c) {
        LB::AI | LB::SG | LB::XX | LB::SA => LB::AL,
        LB::CJ => LB::NS,
        other => other,
    }
}

/// Byte positions inside `text` where a line break is allowed (mandatory
/// breaks are included — callers pre-split on `\n`). Positions are
/// strictly between chars: 0 and `text.len()` are never returned.
pub fn break_opportunities(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut chars = text.char_indices();
    let Some((_, first)) = chars.next() else {
        return out;
    };

    // Effective class of the previous char (after LB9 folding / LB10).
    let mut a = match resolved(first) {
        LB::CM | LB::ZWJ => LB::AL, // LB10: lone combining mark
        other => other,
    };
    // Raw class of the previous char (LB8a needs unfolded ZWJ).
    let mut a_raw = resolved(first);
    // Class before `a` (LB21a).
    let mut prev2: Option<LB> = None;
    // Last non-space effective class before the current space run (LB8/14/16/17).
    let mut before_sp = a;
    // Consecutive regional indicators ending at `a` (LB30a).
    let mut ri_count = u32::from(a == LB::RI);

    for (i, c) in chars {
        let b = resolved(c);
        let no_break = 'rules: {
            // LB4/LB5: mandatory breaks (BK, CR, LF, NL) — CR × LF first.
            if a_raw == LB::CR && b == LB::LF {
                break 'rules true;
            }
            if matches!(a_raw, LB::BK | LB::CR | LB::LF | LB::NL) {
                break 'rules false;
            }
            // LB6/LB7.
            if matches!(b, LB::BK | LB::CR | LB::LF | LB::NL | LB::SP | LB::ZW) {
                break 'rules true;
            }
            // LB8: ZW SP* ÷.
            if before_sp == LB::ZW {
                break 'rules false;
            }
            // LB8a: ZWJ ×.
            if a_raw == LB::ZWJ {
                break 'rules true;
            }
            // LB9: X (CM|ZWJ)* is treated as X.
            if matches!(b, LB::CM | LB::ZWJ)
                && !matches!(a_raw, LB::BK | LB::CR | LB::LF | LB::NL | LB::SP | LB::ZW)
            {
                break 'rules true;
            }
            // (LB10 is handled where classes are assigned below.)
            // LB11: word joiner.
            if b == LB::WJ || a == LB::WJ {
                break 'rules true;
            }
            // LB12/LB12a: glue (no-break space and friends).
            if a == LB::GL {
                break 'rules true;
            }
            if b == LB::GL && !matches!(a, LB::SP | LB::BA | LB::HY) {
                break 'rules true;
            }
            // LB13: closing punctuation.
            if matches!(b, LB::CL | LB::CP | LB::EX | LB::IS | LB::SY) {
                break 'rules true;
            }
            // LB14: OP SP* ×.
            if before_sp == LB::OP {
                break 'rules true;
            }
            // LB15 (classic): QU SP* × OP.
            if before_sp == LB::QU && b == LB::OP {
                break 'rules true;
            }
            // LB16: (CL|CP) SP* × NS.
            if matches!(before_sp, LB::CL | LB::CP) && b == LB::NS {
                break 'rules true;
            }
            // LB17: B2 SP* × B2.
            if before_sp == LB::B2 && b == LB::B2 {
                break 'rules true;
            }
            // LB18: break after spaces.
            if a == LB::SP {
                break 'rules false;
            }
            // LB19: quotes.
            if b == LB::QU || a == LB::QU {
                break 'rules true;
            }
            // LB20: contingent breaks.
            if b == LB::CB || a == LB::CB {
                break 'rules false;
            }
            // LB21/21a/21b: hyphens, small kana, Hebrew.
            if matches!(b, LB::BA | LB::HY | LB::NS) || a == LB::BB {
                break 'rules true;
            }
            if prev2 == Some(LB::HL) && matches!(a, LB::HY | LB::BA) {
                break 'rules true;
            }
            if a == LB::SY && b == LB::HL {
                break 'rules true;
            }
            // LB22: ellipsis.
            if b == LB::IN {
                break 'rules true;
            }
            // LB23/23a: letters and numbers, prefixes around ideographs.
            if matches!(a, LB::AL | LB::HL) && b == LB::NU {
                break 'rules true;
            }
            if a == LB::NU && matches!(b, LB::AL | LB::HL) {
                break 'rules true;
            }
            if a == LB::PR && matches!(b, LB::ID | LB::EB | LB::EM) {
                break 'rules true;
            }
            if matches!(a, LB::ID | LB::EB | LB::EM) && b == LB::PO {
                break 'rules true;
            }
            // LB24.
            if matches!(a, LB::PR | LB::PO) && matches!(b, LB::AL | LB::HL) {
                break 'rules true;
            }
            if matches!(a, LB::AL | LB::HL) && matches!(b, LB::PR | LB::PO) {
                break 'rules true;
            }
            // LB25 (simplified numbers).
            if matches!(a, LB::CL | LB::CP | LB::NU) && matches!(b, LB::PO | LB::PR) {
                break 'rules true;
            }
            if matches!(a, LB::PO | LB::PR) && matches!(b, LB::OP | LB::NU) {
                break 'rules true;
            }
            if matches!(a, LB::HY | LB::IS | LB::NU | LB::SY) && b == LB::NU {
                break 'rules true;
            }
            // LB26/LB27: Korean syllable blocks.
            if a == LB::JL && matches!(b, LB::JL | LB::JV | LB::H2 | LB::H3) {
                break 'rules true;
            }
            if matches!(a, LB::JV | LB::H2) && matches!(b, LB::JV | LB::JT) {
                break 'rules true;
            }
            if matches!(a, LB::JT | LB::H3) && b == LB::JT {
                break 'rules true;
            }
            if matches!(a, LB::JL | LB::JV | LB::JT | LB::H2 | LB::H3) && b == LB::PO {
                break 'rules true;
            }
            if a == LB::PR && matches!(b, LB::JL | LB::JV | LB::JT | LB::H2 | LB::H3) {
                break 'rules true;
            }
            // LB28: letters.
            if matches!(a, LB::AL | LB::HL) && matches!(b, LB::AL | LB::HL) {
                break 'rules true;
            }
            // LB28a (core Brahmic pairs).
            if a == LB::AP && matches!(b, LB::AK | LB::AS) {
                break 'rules true;
            }
            if matches!(a, LB::AK | LB::AS) && matches!(b, LB::VF | LB::VI) {
                break 'rules true;
            }
            // LB29.
            if a == LB::IS && matches!(b, LB::AL | LB::HL) {
                break 'rules true;
            }
            // LB30 (without the East-Asian-width refinement).
            if matches!(a, LB::AL | LB::HL | LB::NU) && b == LB::OP {
                break 'rules true;
            }
            if matches!(a, LB::CL | LB::CP) && matches!(b, LB::AL | LB::HL | LB::NU) {
                break 'rules true;
            }
            // LB30a: regional-indicator pairs.
            if a == LB::RI && b == LB::RI && ri_count % 2 == 1 {
                break 'rules true;
            }
            // LB30b: emoji base × modifier.
            if a == LB::EB && b == LB::EM {
                break 'rules true;
            }
            // LB31: break everywhere else.
            false
        };

        if !no_break {
            out.push(i);
        }

        // Advance state. LB9: a CM/ZWJ that attached keeps the base's class;
        // LB10: an unattached CM/ZWJ becomes AL.
        let folded = matches!(b, LB::CM | LB::ZWJ)
            && !matches!(a_raw, LB::BK | LB::CR | LB::LF | LB::NL | LB::SP | LB::ZW);
        a_raw = b;
        if !folded {
            prev2 = Some(a);
            a = match b {
                LB::CM | LB::ZWJ => LB::AL,
                other => other,
            };
            if a != LB::SP {
                before_sp = a;
            }
            ri_count = if a == LB::RI { ri_count + 1 } else { 0 };
        }
    }
    out
}

/// Split `text` into its unbreakable units: the segments between line-break
/// opportunities. Trailing spaces belong to the preceding unit (LB18 breaks
/// *after* a space run), so wrapping measures units minus trailing
/// whitespace. (The layout engine itself maps [`break_opportunities`] onto
/// shaped glyph clusters instead — this convenience is for text-editing
/// callers.)
pub fn units(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for pos in break_opportunities(text) {
        out.push(&text[start..pos]);
        start = pos;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::units;

    #[test]
    fn spaces_break_after() {
        assert_eq!(units("foo bar  baz"), vec!["foo ", "bar  ", "baz"]);
    }

    #[test]
    fn nbsp_glues() {
        assert_eq!(units("12\u{00A0}km away"), vec!["12\u{00A0}km ", "away"]);
    }

    #[test]
    fn hyphen_breaks_after() {
        assert_eq!(units("well-known"), vec!["well-", "known"]);
    }

    #[test]
    fn punctuation_stays_attached() {
        assert_eq!(units("(hi) ok!"), vec!["(hi) ", "ok!"]);
        assert_eq!(units("3.14, e.g."), vec!["3.14, ", "e.g."]);
    }

    #[test]
    fn cjk_breaks_between_ideographs() {
        assert_eq!(
            units("日本語テキスト"),
            vec!["日", "本", "語", "テ", "キ", "ス", "ト"]
        );
        // ... but not before the prolonged-sound mark or small kana.
        assert_eq!(units("カーテン"), vec!["カー", "テ", "ン"]);
        assert_eq!(units("ちょっと"), vec!["ちょっ", "と"]);
    }

    #[test]
    fn combining_marks_are_transparent() {
        assert_eq!(units("ne\u{0301}e case"), vec!["ne\u{0301}e ", "case"]);
    }
}

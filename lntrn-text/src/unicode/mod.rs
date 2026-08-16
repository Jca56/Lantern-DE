//! Unicode segmentation, built on property tables code-generated from the
//! UCD data in `ucd/` (see `examples/gen_unicode.rs`) — no unicode-* crates.
//!
//! Public surface: [`graphemes`] (UAX#29 extended grapheme clusters — what a
//! cursor should treat as one character) and [`break_opportunities`] (UAX#14
//! line-break positions). Script itemization (UAX#24) is crate-internal
//! until complex-script shaping consumes it in Phase 8.

mod bidi;
mod grapheme;
mod linebreak;
mod script;
pub mod tables;

pub(crate) use bidi::{reorder, resolve as bidi_resolve};
pub use grapheme::{graphemes, next_grapheme_boundary, Graphemes};
pub use linebreak::{break_opportunities, units};
#[allow(unused_imports)] // consumed by per-script shaping in Phase 8
pub(crate) use script::{script_runs, ScriptRun};
pub use tables::{BidiClass, GraphemeBreak, JoiningType, LineBreakClass, Script};

/// Binary search a sorted, non-overlapping `(start, end, value)` range table.
fn lookup<T: Copy>(table: &[(u32, u32, T)], c: u32, default: T) -> T {
    let mut lo = 0usize;
    let mut hi = table.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (start, end, value) = table[mid];
        if end < c {
            lo = mid + 1;
        } else if start > c {
            hi = mid;
        } else {
            return value;
        }
    }
    default
}

pub fn grapheme_break(c: char) -> GraphemeBreak {
    lookup(tables::GRAPHEME_BREAK, c as u32, GraphemeBreak::Other)
}

pub fn is_extended_pictographic(c: char) -> bool {
    let c = c as u32;
    tables::EXTENDED_PICTOGRAPHIC
        .binary_search_by(|&(start, end)| {
            if end < c {
                std::cmp::Ordering::Less
            } else if start > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

pub fn script(c: char) -> Script {
    lookup(tables::SCRIPTS, c as u32, Script::Unknown)
}

pub fn line_break_class(c: char) -> LineBreakClass {
    lookup(tables::LINE_BREAK, c as u32, LineBreakClass::XX)
}

pub fn bidi_class(c: char) -> BidiClass {
    lookup(tables::BIDI_CLASS, c as u32, BidiClass::L)
}

/// The character's bidi-mirrored partner (`(` ↔ `)`), if it has one.
pub fn mirror(c: char) -> Option<char> {
    let c = c as u32;
    tables::MIRROR_PAIRS
        .binary_search_by_key(&c, |&(from, _)| from)
        .ok()
        .and_then(|i| char::from_u32(tables::MIRROR_PAIRS[i].1))
}

/// Arabic joining type. Unlisted characters default to Transparent when
/// combining (approximated via the grapheme Extend class) and Non-Joining
/// otherwise, per the ArabicShaping.txt header.
pub fn joining_type(c: char) -> JoiningType {
    let listed = lookup(tables::JOINING_TYPE, c as u32, JoiningType::U);
    if listed == JoiningType::U && grapheme_break(c) == GraphemeBreak::Extend {
        JoiningType::T
    } else {
        listed
    }
}

/// Default-ignorable code points that must not render a visible glyph
/// (directional controls, joiners, BOM). Kept for segmentation but skipped
/// at glyph resolution.
pub(crate) fn is_default_ignorable(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x061C | 0x180E | 0xFEFF
        | 0x200B..=0x200F
        | 0x202A..=0x202E
        | 0x2060..=0x2064
        | 0x2066..=0x206F
    )
}

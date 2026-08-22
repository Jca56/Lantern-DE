//! UAX#9 bidirectional algorithm: resolved embedding levels per character.
//!
//! Implements P2–P3 (paragraph direction), the X explicit-embedding stack
//! (RLE/LRE/RLO/LRO/PDF), W1–W7 (weak types), N1–N2 (neutrals), I1–I2
//! (implicit levels), and L1 (trailing-whitespace reset). The layout engine
//! performs L2 (run reversal) and L4 (mirroring) on the shaped glyphs.
//!
//! Documented simplifications:
//! - Directional *isolates* (RLI/LRI/FSI/PDI) are treated as neutral
//!   characters rather than isolating sequences; the strong marks LRM/RLM —
//!   the common manual control — work fully via their L/R classes.
//! - N0 (paired brackets) is skipped; brackets resolve via N1/N2.
//! - Level runs are processed independently with sos/eos from adjacent run
//!   levels (no isolating-run-sequence chaining).

use super::{bidi_class, tables::BidiClass as BC};

/// Per-char resolved data for one line of text.
pub(crate) struct BidiLine {
    /// (byte offset, resolved level), one entry per char, in logical order.
    pub levels: Vec<(usize, u8)>,
    /// Paragraph embedding level (0 = LTR base). Exposed for future
    /// alignment work; currently consumed by tests.
    #[allow(dead_code)]
    pub base_level: u8,
}

const MAX_DEPTH: u8 = 125;

/// Resolve embedding levels. Returns `None` for pure-LTR text (the fast path
/// — no reordering work needed at all).
pub(crate) fn resolve(text: &str) -> Option<BidiLine> {
    let mut types: Vec<(usize, BC)> = Vec::new();
    let mut any_rtl = false;
    for (i, c) in text.char_indices() {
        let t = bidi_class(c);
        any_rtl |= matches!(t, BC::R | BC::AL | BC::AN | BC::RLE | BC::RLO | BC::RLI);
        types.push((i, t));
    }
    if !any_rtl {
        return None;
    }

    // P2/P3: paragraph level from the first strong type.
    let base_level = types
        .iter()
        .find_map(|&(_, t)| match t {
            BC::L => Some(0),
            BC::R | BC::AL => Some(1),
            _ => None,
        })
        .unwrap_or(0);

    // X1–X8: explicit embeddings + overrides. Embedding initiators and PDF
    // become BN (removed from further analysis, X9).
    let mut levels: Vec<u8> = vec![base_level; types.len()];
    {
        let mut stack: Vec<(u8, Option<BC>)> = vec![(base_level, None)];
        let mut overflow = 0u32;
        for (idx, (_, t)) in types.iter_mut().enumerate() {
            match *t {
                BC::RLE | BC::LRE | BC::RLO | BC::LRO => {
                    let &(cur, _) = stack.last().unwrap();
                    let next = if matches!(*t, BC::RLE | BC::RLO) {
                        (cur + 1) | 1 // next odd
                    } else {
                        (cur + 2) & !1 // next even
                    };
                    if next <= MAX_DEPTH && overflow == 0 {
                        let ov = match *t {
                            BC::RLO => Some(BC::R),
                            BC::LRO => Some(BC::L),
                            _ => None,
                        };
                        stack.push((next, ov));
                    } else {
                        overflow += 1;
                    }
                    levels[idx] = stack.last().unwrap().0;
                    *t = BC::BN;
                }
                BC::PDF => {
                    if overflow > 0 {
                        overflow -= 1;
                    } else if stack.len() > 1 {
                        stack.pop();
                    }
                    levels[idx] = stack.last().unwrap().0;
                    *t = BC::BN;
                }
                _ => {
                    let &(cur, ov) = stack.last().unwrap();
                    levels[idx] = cur;
                    if let Some(forced) = ov {
                        *t = forced;
                    }
                }
            }
        }
    }

    // Split into level runs (skipping BN) and resolve W/N/I per run.
    let n = types.len();
    let mut i = 0;
    while i < n {
        if types[i].1 == BC::BN {
            i += 1;
            continue;
        }
        let level = levels[i];
        let mut j = i;
        let mut run: Vec<usize> = Vec::new();
        while j < n {
            if types[j].1 == BC::BN {
                j += 1;
                continue;
            }
            if levels[j] != level {
                break;
            }
            run.push(j);
            j += 1;
        }
        // sos/eos from the higher of the adjacent levels.
        let prev_level = (0..i)
            .rev()
            .find(|&k| types[k].1 != BC::BN)
            .map_or(base_level, |k| levels[k]);
        let next_level = (j..n)
            .find(|&k| types[k].1 != BC::BN)
            .map_or(base_level, |k| levels[k]);
        let sos = if prev_level.max(level) % 2 == 1 {
            BC::R
        } else {
            BC::L
        };
        let eos = if next_level.max(level) % 2 == 1 {
            BC::R
        } else {
            BC::L
        };
        resolve_run(&mut types, &mut levels, &run, level, sos, eos);
        i = j;
    }

    // L1: trailing whitespace/separators return to the base level.
    for (idx, (_, t)) in types.iter().enumerate().rev() {
        match t {
            BC::WS | BC::B | BC::S | BC::BN => levels[idx] = base_level,
            _ => break,
        }
    }

    Some(BidiLine {
        levels: types
            .iter()
            .zip(&levels)
            .map(|(&(b, _), &l)| (b, l))
            .collect(),
        base_level,
    })
}

/// W1–W7, N1–N2, I1–I2 over one level run (`run` holds indices into `types`).
fn resolve_run(
    types: &mut [(usize, BC)],
    levels: &mut [u8],
    run: &[usize],
    level: u8,
    sos: BC,
    eos: BC,
) {
    let get = |types: &[(usize, BC)], k: usize| types[run[k]].1;
    let len = run.len();

    // W1: NSM takes the type of the previous character (sos at run start).
    for k in 0..len {
        if get(types, k) == BC::NSM {
            types[run[k]].1 = if k == 0 { sos } else { get(types, k - 1) };
        }
    }
    // W2: EN → AN when the last strong type was AL.
    let mut strong = sos;
    for k in 0..len {
        match get(types, k) {
            BC::L | BC::R | BC::AL => strong = get(types, k),
            BC::EN if strong == BC::AL => types[run[k]].1 = BC::AN,
            _ => {}
        }
    }
    // W3: AL → R.
    for k in 0..len {
        if get(types, k) == BC::AL {
            types[run[k]].1 = BC::R;
        }
    }
    // W4: single ES between EN–EN → EN; single CS between same numbers → that.
    for k in 1..len.saturating_sub(1) {
        let (prev, cur, next) = (get(types, k - 1), get(types, k), get(types, k + 1));
        if cur == BC::ES && prev == BC::EN && next == BC::EN {
            types[run[k]].1 = BC::EN;
        } else if cur == BC::CS
            && ((prev == BC::EN && next == BC::EN) || (prev == BC::AN && next == BC::AN))
        {
            types[run[k]].1 = prev;
        }
    }
    // W5: ET runs adjacent to EN → EN.
    for k in 0..len {
        if get(types, k) == BC::ET {
            let before = (0..k)
                .rev()
                .take_while(|&m| get(types, m) == BC::ET)
                .count();
            let prev_is_en = k > before && get(types, k - before - 1) == BC::EN;
            let after = (k + 1..len)
                .take_while(|&m| get(types, m) == BC::ET)
                .count();
            let next_is_en = k + after + 1 < len && get(types, k + after + 1) == BC::EN;
            if prev_is_en || next_is_en {
                types[run[k]].1 = BC::EN;
            }
        }
    }
    // W6: leftover separators/terminators → ON.
    for k in 0..len {
        if matches!(get(types, k), BC::ET | BC::ES | BC::CS) {
            types[run[k]].1 = BC::ON;
        }
    }
    // W7: EN → L when the last strong type was L.
    strong = sos;
    for k in 0..len {
        match get(types, k) {
            BC::L | BC::R => strong = get(types, k),
            BC::EN if strong == BC::L => types[run[k]].1 = BC::L,
            _ => {}
        }
    }

    // N1/N2: neutrals (incl. isolate controls, simplified) take the
    // surrounding direction when it matches, else the embedding direction.
    let is_neutral = |t: BC| {
        matches!(
            t,
            BC::B | BC::S | BC::WS | BC::ON | BC::RLI | BC::LRI | BC::FSI | BC::PDI
        )
    };
    let strength = |t: BC| match t {
        BC::L => Some(BC::L),
        BC::R | BC::EN | BC::AN => Some(BC::R),
        _ => None,
    };
    let embedding = if level % 2 == 1 { BC::R } else { BC::L };
    let mut k = 0;
    while k < len {
        if !is_neutral(get(types, k)) {
            k += 1;
            continue;
        }
        let mut end = k;
        while end < len && is_neutral(get(types, end)) {
            end += 1;
        }
        let before = if k == 0 {
            Some(sos)
        } else {
            strength(get(types, k - 1))
        };
        let after = if end == len {
            Some(eos)
        } else {
            strength(get(types, end))
        };
        let fill = match (before, after) {
            (Some(a), Some(b)) if a == b => a,
            _ => embedding,
        };
        for m in k..end {
            types[run[m]].1 = fill;
        }
        k = end;
    }

    // I1/I2: implicit levels.
    #[allow(clippy::needless_range_loop, clippy::manual_is_multiple_of)]
    for k in 0..len {
        let idx = run[k];
        let t = types[idx].1;
        if level % 2 == 0 {
            match t {
                BC::R => levels[idx] = level + 1,
                BC::AN | BC::EN => levels[idx] = level + 2,
                _ => {}
            }
        } else if matches!(t, BC::L | BC::AN | BC::EN) {
            levels[idx] = level + 1;
        }
    }
}

/// L2: visual reorder of per-glyph levels — reverse maximal runs from the
/// highest level down to the lowest odd level. Returns visual-order indices.
pub(crate) fn reorder(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&max) = levels.iter().max() else {
        return order;
    };
    let min_odd = levels
        .iter()
        .copied()
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(max + 1);
    let mut lvl = max;
    while lvl >= min_odd && lvl >= 1 {
        let mut k = 0;
        while k < order.len() {
            if levels[order[k]] >= lvl {
                let mut end = k;
                while end < order.len() && levels[order[end]] >= lvl {
                    end += 1;
                }
                order[k..end].reverse();
                k = end;
            } else {
                k += 1;
            }
        }
        if lvl == 0 {
            break;
        }
        lvl -= 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::{reorder, resolve};

    #[test]
    fn pure_ltr_is_fast_path() {
        assert!(resolve("hello world").is_none());
    }

    #[test]
    fn pure_rtl() {
        let bidi = resolve("שלום").unwrap();
        assert_eq!(bidi.base_level, 1);
        assert!(bidi.levels.iter().all(|&(_, l)| l == 1));
    }

    #[test]
    fn ltr_with_embedded_rtl() {
        let bidi = resolve("abc שלום xyz").unwrap();
        assert_eq!(bidi.base_level, 0);
        let levels: Vec<u8> = bidi.levels.iter().map(|&(_, l)| l).collect();
        // Latin at 0, Hebrew at 1, separating spaces resolve outward.
        assert_eq!(levels[0], 0);
        assert_eq!(levels[4], 1, "Hebrew run should be level 1");
        assert_eq!(*levels.last().unwrap(), 0);
    }

    #[test]
    fn numbers_in_rtl_run() {
        let bidi = resolve("אבג 123").unwrap();
        assert_eq!(bidi.base_level, 1);
        let levels: Vec<u8> = bidi.levels.iter().map(|&(_, l)| l).collect();
        // European numbers inside an RTL paragraph sit at level 2.
        assert_eq!(levels[4], 2);
    }

    #[test]
    fn reorder_reverses_rtl_runs() {
        // L L R R L → visual: 0 1 3 2 4.
        assert_eq!(reorder(&[0, 0, 1, 1, 0]), vec![0, 1, 3, 2, 4]);
        // Pure RTL reverses fully.
        assert_eq!(reorder(&[1, 1, 1]), vec![2, 1, 0]);
        // Number inside RTL (level 2 inside 1): whole reversed, number LTR.
        assert_eq!(reorder(&[1, 1, 2, 2]), vec![2, 3, 1, 0]);
    }
}

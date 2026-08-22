//! Wrap-aware visual-line layout for the notes body editor.
//!
//! The editor model is hard lines ('\n'-separated); the body displays
//! soft-wrapped *visual* lines. This module is the single source of
//! truth for mapping byte offsets ↔ visual lines, so rendering, click
//! hit-testing, caret drawing, and scrolling all agree on where every
//! character sits. Any of those doing their own layout math is exactly
//! the clicks-land-in-the-wrong-place bug.

use lntrn_render::TextRenderer;

/// Width passed to `queue()` for pre-wrapped lines. Matches the
/// unbounded width `measure_width` uses internally so the queued
/// layout shares the measuring layout's cache entry — and the engine
/// never re-wraps (its wrap bound quantizes down, which would clip the
/// last glyph of an exact-width line).
pub const NO_WRAP: f32 = 10000.0;

/// One displayed line: a byte range into the editor buffer. Coverage
/// is contiguous — line N+1 starts where line N ends (plus the '\n'
/// after a hard break) — so every cursor byte maps to exactly one line.
#[derive(Debug, Clone, Copy)]
pub struct VisualLine {
    pub start: usize,
    /// Exclusive; excludes the trailing '\n' on hard breaks.
    pub end: usize,
    /// True when the break is a real '\n' (or end of buffer) rather
    /// than a soft wrap.
    pub hard_break: bool,
}

/// Body editor font: ~12% over the global text size so prose reads
/// comfortably without inflating every other label on the page.
pub fn body_font(text_size: f32, scale: f32) -> f32 {
    (text_size * scale * 1.12).max(16.0)
}

/// Greedy word-wrap of the whole buffer at `max_w`. Always returns at
/// least one line (an empty buffer is one empty hard line).
pub fn layout(buf: &str, font: f32, max_w: f32, text: &mut TextRenderer) -> Vec<VisualLine> {
    let max_w = max_w.max(1.0);
    let mut out = Vec::new();
    let mut start = 0usize;
    loop {
        let hard_end = buf[start..].find('\n').map(|i| start + i);
        let end = hard_end.unwrap_or(buf.len());
        wrap_hard_line(buf, start, end, font, max_w, text, &mut out);
        match hard_end {
            Some(e) => start = e + 1,
            None => return out,
        }
    }
}

/// Visual line index the caret occupies. At a soft-wrap boundary the
/// caret belongs to the continuation line — that's where typing
/// visibly inserts.
pub fn caret_vline(vlines: &[VisualLine], cursor: usize) -> usize {
    for (i, vl) in vlines.iter().enumerate() {
        if cursor < vl.end || (cursor == vl.end && vl.hard_break) {
            return i;
        }
    }
    vlines.len().saturating_sub(1)
}

fn wrap_hard_line(
    buf: &str,
    start: usize,
    end: usize,
    font: f32,
    max_w: f32,
    text: &mut TextRenderer,
    out: &mut Vec<VisualLine>,
) {
    let mut seg = start;
    loop {
        let line = &buf[seg..end];
        if line.is_empty() || text.measure_width(line, font) <= max_w {
            out.push(VisualLine {
                start: seg,
                end,
                hard_break: true,
            });
            return;
        }
        let fit = longest_fit(line, font, max_w, text);
        // Snap back to the last space so words stay whole. The space
        // itself stays on this line (its ink is empty, so hanging a
        // hair past the edge is invisible) and the continuation starts
        // flush on a word.
        let brk = match line[..fit].rfind(' ') {
            Some(sp) if sp + 1 < line.len() => sp + 1,
            _ => fit,
        };
        if brk >= line.len() {
            out.push(VisualLine {
                start: seg,
                end,
                hard_break: true,
            });
            return;
        }
        out.push(VisualLine {
            start: seg,
            end: seg + brk,
            hard_break: false,
        });
        seg += brk;
    }
}

/// Longest prefix of `line` (on a char boundary) measuring within
/// `max_w`, found by binary search. Returns at least one char so
/// wrapping always makes progress even when a single glyph overflows.
/// Only called when the full line is known not to fit.
fn longest_fit(line: &str, font: f32, max_w: f32, text: &mut TextRenderer) -> usize {
    let starts: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
    if starts.len() < 2 {
        return line.len();
    }
    // Candidate k = prefix of the first k chars, k in [1, len-1].
    let (mut lo, mut hi, mut best) = (1usize, starts.len() - 1, 1usize);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        if text.measure_width(&line[..starts[mid]], font) <= max_w {
            best = mid;
            lo = mid + 1;
        } else if mid <= 1 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    starts[best]
}

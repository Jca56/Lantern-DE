//! Word-wrap computation. Resolves each document line into the byte offsets
//! where its visual rows begin, honouring per-span font metrics and the
//! paragraph's first-line indent. Kept apart from `render.rs` so the frame
//! renderer stays about painting, not measuring.
//!
//! Wraps are cached per line: each line's content + width-relevant formatting
//! is hashed into a signature, and `compute` only re-wraps lines whose
//! signature changed since the previous frame. The old recompute-everything
//! path made every frame O(document) in text-measure calls, which pinned the
//! main thread after a large paste. Hashing is self-healing — no edit path
//! has to remember to invalidate anything.

use lntrn_render::TextRenderer;

use crate::editor::Editor;
use crate::render::{span_family, span_rendering};

/// Recompute stale word-wrap info and store it on the editor.
pub fn compute(
    text: &mut TextRenderer,
    editor: &mut Editor,
    max_width: f32,
    scale: f32,
    default_font_size: f32,
) {
    let key = (max_width.to_bits(), scale.to_bits(), default_font_size.to_bits());
    let n = editor.lines.len();
    let full = editor.wrap_key != Some(key)
        || editor.wrap_rows.len() != n
        || editor.wrap_sigs.len() != n;
    if full {
        // A computed row list is never empty (always at least [0]), so an
        // empty vec marks the line as needing recompute below.
        editor.wrap_rows.clear();
        editor.wrap_rows.resize(n, Vec::new());
        editor.wrap_sigs.clear();
        editor.wrap_sigs.resize(n, 0);
        editor.wrap_key = Some(key);
    }

    for i in 0..n {
        let sig = line_signature(editor, i);
        if editor.wrap_sigs[i] == sig && !editor.wrap_rows[i].is_empty() {
            continue;
        }
        let para = editor.formats.get(i).para;
        let indent_px = para.first_indent * scale;
        // Bullet paragraphs lose width to the hanging indent on every row.
        let line_w = if para.bullet {
            (max_width - crate::editor::BULLET_INDENT * scale).max(10.0)
        } else {
            max_width
        };
        editor.wrap_rows[i] =
            line_wraps(text, editor, i, line_w, indent_px, default_font_size);
        editor.wrap_sigs[i] = sig;
    }
}

// ── Line signatures (FNV-1a, u64-chunked) ───────────────────────────────────

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv_u64(h: &mut u64, v: u64) {
    *h ^= v;
    *h = h.wrapping_mul(FNV_PRIME);
}

fn fnv_bytes(h: &mut u64, bytes: &[u8]) {
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        fnv_u64(h, u64::from_le_bytes(c.try_into().unwrap()));
    }
    let rem = chunks.remainder();
    let mut last = [0u8; 8];
    last[..rem.len()].copy_from_slice(rem);
    // Mix the remainder length so "ab" and "ab\0" can't collide.
    fnv_u64(h, u64::from_le_bytes(last) ^ ((rem.len() as u64 + 1) << 56));
}

/// Hash everything that affects a line's wrap result: its text plus the
/// width-relevant parts of its formatting (span bounds, size, weight, style,
/// family, first indent, bullet). Colors and decorations are excluded — they
/// don't change measurement.
fn line_signature(editor: &Editor, line: usize) -> u64 {
    let mut h = FNV_OFFSET;
    fnv_bytes(&mut h, editor.lines[line].as_bytes());
    let lf = editor.formats.get(line);
    for span in lf.spans() {
        fnv_u64(&mut h, span.start as u64);
        fnv_u64(&mut h, span.end as u64);
        fnv_u64(&mut h, span.attrs.font_size.map_or(0, |f| f.to_bits() as u64 + 1));
        fnv_u64(
            &mut h,
            (span.attrs.bold as u64)
                | (span.attrs.italic as u64) << 1
                | (span.attrs.font.map_or(0, |f| f as u64 + 1)) << 2,
        );
    }
    fnv_u64(&mut h, lf.para.first_indent.to_bits() as u64);
    fnv_u64(&mut h, lf.para.bullet as u64);
    h
}

/// Compute word-wrap break points for a single document line.
/// Returns byte offsets where each visual row starts (first is always 0).
/// `first_indent_px` reduces the first row's available width.
fn line_wraps(
    text: &mut TextRenderer,
    editor: &Editor,
    line_idx: usize,
    max_width: f32,
    first_indent_px: f32,
    default_font_size: f32,
) -> Vec<usize> {
    let line_str = &editor.lines[line_idx];
    if line_str.is_empty() || max_width <= 0.0 {
        return vec![0];
    }

    let spans = editor.formats.get(line_idx).iter_spans(line_str.len());
    let mut row_starts: Vec<usize> = vec![0];
    let mut row_x: f32 = 0.0;
    // First row has reduced width for first-line indent
    let mut effective_w = (max_width - first_indent_px).max(10.0);
    let mut last_break: Option<(usize, f32)> = None; // (byte_after_break, row_x_at_that_point)

    for span in &spans {
        let (fs, weight, style) = span_rendering(span, default_font_size);
        let family = span_family(span);
        for (rel_i, ch) in line_str[span.start..span.end].char_indices() {
            let byte_pos = span.start + rel_i;
            let ch_w = text.measure_width_full(
                &line_str[byte_pos..byte_pos + ch.len_utf8()],
                fs,
                weight,
                style,
                family,
            );

            if row_x + ch_w > effective_w && byte_pos > *row_starts.last().unwrap() {
                if let Some((br_byte, br_x)) = last_break {
                    if br_byte > *row_starts.last().unwrap() {
                        row_starts.push(br_byte);
                        row_x -= br_x;
                    } else {
                        row_starts.push(byte_pos);
                        row_x = 0.0;
                    }
                } else {
                    row_starts.push(byte_pos);
                    row_x = 0.0;
                }
                last_break = None;
                // Subsequent rows get full width (no indent)
                effective_w = max_width;
            }

            row_x += ch_w;

            // Track word-boundary break points: spaces and hyphens
            if ch == ' ' || ch == '-' {
                last_break = Some((byte_pos + ch.len_utf8(), row_x));
            }
        }
    }

    row_starts
}

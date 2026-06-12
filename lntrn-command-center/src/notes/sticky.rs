//! Floating sticky notes — overlay companions for the Notes page.
//!
//! Any note can be "stuck": it renders as a classic sticky-pad square
//! floating anywhere on screen, visible whenever the Command Center
//! overlay is up (including collapsed-to-dock) and gone when it closes.
//! Stickies live on painter/text layer 0 while the panel + outer chrome
//! render on layer 1 (modals on 2), so the CC always draws over any
//! sticky it overlaps.
//!
//! Display-only: drag to move, corner-drag to resize, hover ✕ to
//! unstick. Editing happens in the Notes page; the selected note's
//! sticky mirrors the live editor buffers while that page is open.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{store, Note, NotesState};

// Geometry (logical px — scaled by the output's fractional scale).
pub const DEFAULT_W: f32 = 320.0;
pub const DEFAULT_H: f32 = 260.0;
pub const MIN_W: f32 = 180.0;
pub const MIN_H: f32 = 130.0;
/// Bottom-right resize-grip hit zone.
const GRIP: f32 = 28.0;
/// Top-right unstick ✕ hit zone.
const UNSTICK: f32 = 32.0;

/// Classic sticky-pad paper tints, cycled per note id. Semantic
/// real-world constants (like the usage page's traffic-light colors),
/// not theme colors — sticky notes are supposed to look like paper.
const PADS: [(u8, u8, u8); 4] = [
    (250, 222, 100), // canary yellow
    (255, 172, 200), // pink
    (152, 208, 255), // sky blue
    (172, 232, 150), // mint green
];

fn paper(id: u64) -> Color {
    let (r, g, b) = PADS[(id % PADS.len() as u64) as usize];
    Color::from_rgb8(r, g, b)
}

/// Warm near-black ink that reads well on every pad tint.
fn ink(alpha: f32) -> Color {
    Color::from_rgb8(46, 38, 24).with_alpha(alpha)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickyPart {
    Body,
    Resize,
    Unstick,
}

/// An in-flight sticky drag. `grab` is the cursor offset in physical px
/// — from the sticky origin for moves, from the bottom-right corner for
/// resizes — so the paper doesn't jump to the cursor on grab.
#[derive(Debug, Clone, Copy)]
pub struct StickyDrag {
    pub id: u64,
    pub resize: bool,
    pub grab: (f32, f32),
}

/// Physical-px rect for a stuck note, clamped into the surface so a
/// sticky placed on the big monitor still lands on-screen on the
/// laptop (geometry is stored in logical px).
pub fn rect(note: &Note, scale: f32, surface_w: f32, surface_h: f32) -> Rect {
    let w = if note.sticky_w > 0.0 { note.sticky_w } else { DEFAULT_W };
    let h = if note.sticky_h > 0.0 { note.sticky_h } else { DEFAULT_H };
    let w = (w * scale).min(surface_w);
    let h = (h * scale).min(surface_h);
    let x = (note.sticky_x * scale).clamp(0.0, (surface_w - w).max(0.0));
    let y = (note.sticky_y * scale).clamp(0.0, (surface_h - h).max(0.0));
    Rect::new(x, y, w, h)
}

/// Topmost sticky under the cursor (physical px) and which part of it.
/// Iterates in reverse draw order so overlapping stickies resolve to
/// the one painted on top.
pub fn hit(
    notes: &[Note],
    scale: f32,
    surface_w: f32,
    surface_h: f32,
    px: f32,
    py: f32,
) -> Option<(u64, StickyPart)> {
    for note in notes.iter().rev().filter(|n| n.sticky) {
        let r = rect(note, scale, surface_w, surface_h);
        if px < r.x || px > r.x + r.w || py < r.y || py > r.y + r.h {
            continue;
        }
        let u = UNSTICK * scale;
        if px >= r.x + r.w - u && py <= r.y + u {
            return Some((note.id, StickyPart::Unstick));
        }
        let g = GRIP * scale;
        if px >= r.x + r.w - g && py >= r.y + r.h - g {
            return Some((note.id, StickyPart::Resize));
        }
        return Some((note.id, StickyPart::Body));
    }
    None
}

/// Logical-px spawn position for a freshly stuck note: the gutter to
/// the right of the panel (falling back to the left when the panel
/// hugs the right edge), cascading down so stickies don't stack.
pub fn spawn_pos(
    notes: &[Note],
    panel: Rect,
    scale: f32,
    surface_w: f32,
    surface_h: f32,
) -> (f32, f32) {
    let count = notes.iter().filter(|n| n.sticky).count() as f32;
    let gap = 24.0 * scale;
    let w = DEFAULT_W * scale;
    let h = DEFAULT_H * scale;
    let x = if panel.x + panel.w + gap + w <= surface_w {
        panel.x + panel.w + gap
    } else {
        (panel.x - gap - w).max(0.0)
    };
    let span = (surface_h - panel.y - h).max(1.0);
    let y = panel.y + (count * 40.0 * scale) % span;
    (x / scale, y / scale)
}

/// Toggle sticky on the selected note. Turning it on places it at
/// `spawn` (logical px) the first time; geometry survives unsticking
/// so re-sticking puts the paper back where it was.
pub fn toggle_selected(state: &mut NotesState, spawn: (f32, f32)) {
    let Some(idx) = state.selected_index() else {
        return;
    };
    let note = &mut state.notes[idx];
    note.sticky = !note.sticky;
    if note.sticky && note.sticky_w <= 0.0 {
        note.sticky_x = spawn.0;
        note.sticky_y = spawn.1;
        note.sticky_w = DEFAULT_W;
        note.sticky_h = DEFAULT_H;
    }
    // Deliberately not bumping modified_ms — geometry isn't an edit and
    // shouldn't re-sort the notes list.
    let _ = store::save_one(note);
}

/// Draw every stuck note. Returns true if anything was drawn so the
/// render pass knows whether a layer-0 → layer-1 flush is needed.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &NotesState,
    hover: Option<(u64, StickyPart)>,
    drag: Option<StickyDrag>,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> bool {
    let sw = surface_w as f32;
    let sh = surface_h as f32;
    let mut drew = false;
    for note in state.notes.iter().filter(|n| n.sticky) {
        // The selected note mirrors the live editor buffers while the
        // Notes page is open, so edits show up on the paper instantly.
        let live = state.open && state.selected_id == Some(note.id);
        let title = if live { state.title.query() } else { note.title.as_str() };
        let body = if live { state.body.raw() } else { note.body.as_str() };
        let hovered = hover.map(|(id, _)| id) == Some(note.id);
        let dragging = drag.map(|d| d.id) == Some(note.id);
        draw_one(
            painter, text, note, title, body, hovered, dragging,
            scale, text_size, alpha, sw, sh, surface_w, surface_h,
        );
        drew = true;
    }
    drew
}

#[allow(clippy::too_many_arguments)]
fn draw_one(
    painter: &mut Painter,
    text: &mut TextRenderer,
    note: &Note,
    title: &str,
    body: &str,
    hovered: bool,
    dragging: bool,
    scale: f32,
    text_size: f32,
    alpha: f32,
    sw: f32,
    sh: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let r = rect(note, scale, sw, sh);
    let radius = 6.0 * scale;
    let pad = 14.0 * scale;

    // Paper, with a soft drop shadow that deepens while dragging.
    let shadow_a = if dragging { 0.45 } else { 0.30 };
    painter.shadow(
        r, radius, 8.0 * scale,
        Color::from_rgb8(0, 0, 0).with_alpha(shadow_a * alpha),
        0.0, 3.0 * scale,
    );
    painter.rect_filled(r, radius, paper(note.id).with_alpha(0.97 * alpha));
    if dragging {
        painter.rect_stroke_sdf(r, radius, 1.6 * scale, ink(0.45 * alpha));
    }

    painter.push_clip(r);
    text.push_clip([r.x, r.y, r.w, r.h]);

    // Title + divider.
    let title_font = (text_size * scale * 1.04).max(16.0);
    let shown_title = if title.trim().is_empty() { "Untitled" } else { title };
    let title_a = if title.trim().is_empty() { 0.45 } else { 0.92 };
    let text_w = r.w - pad * 2.0;
    text.queue(
        shown_title, title_font, r.x + pad, r.y + pad * 0.85,
        ink(title_a * alpha), text_w - UNSTICK * scale * 0.6, surface_w, surface_h,
    );
    let div_y = r.y + pad * 0.85 + title_font * 1.35;
    painter.line(r.x + pad, div_y, r.x + r.w - pad, div_y, 1.0 * scale, ink(0.22 * alpha));

    // Body, word-wrapped to the paper width.
    let body_font = (text_size * scale * 0.94).max(15.0);
    let line_h = body_font * 1.32;
    let body_top = div_y + 8.0 * scale;
    let max_lines = (((r.y + r.h - pad * 0.7) - body_top) / line_h).floor().max(0.0) as usize;
    let lines = wrap(text, body, body_font, text_w, max_lines);
    for (i, line) in lines.iter().enumerate() {
        text.queue(
            line, body_font, r.x + pad, body_top + i as f32 * line_h,
            ink(0.80 * alpha), text_w, surface_w, surface_h,
        );
    }

    // Resize grip: three dots walking up the bottom-right diagonal.
    let grip_a = if hovered { 0.45 } else { 0.25 };
    for i in 0..3u32 {
        let off = (6.0 + i as f32 * 6.0) * scale;
        painter.circle_filled(
            r.x + r.w - off, r.y + r.h - off,
            1.8 * scale, ink(grip_a * alpha),
        );
    }

    // Unstick ✕, top-right, hover-only so the paper stays clean.
    if hovered {
        let s = UNSTICK * scale;
        let cx = r.x + r.w - s * 0.5;
        let cy = r.y + s * 0.5;
        let arm = s * 0.16;
        painter.circle_filled(cx, cy, s * 0.32, ink(0.12 * alpha));
        painter.line(cx - arm, cy - arm, cx + arm, cy + arm, 1.8 * scale, ink(0.75 * alpha));
        painter.line(cx - arm, cy + arm, cx + arm, cy - arm, 1.8 * scale, ink(0.75 * alpha));
    }

    text.pop_clip();
    painter.pop_clip();
}

/// Greedy word-wrap using real measured widths. A single over-long
/// word goes on its own line and lets the clip eat the overflow. The
/// final line gets an ellipsis when the note didn't fit.
fn wrap(
    text: &mut TextRenderer,
    s: &str,
    font: f32,
    max_w: f32,
    max_lines: usize,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut truncated = false;
    'outer: for raw in s.split('\n') {
        if out.len() >= max_lines {
            truncated = true;
            break;
        }
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in raw.split(' ') {
            let cand = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if cur.is_empty() || text.measure_width(&cand, font) <= max_w {
                cur = cand;
            } else {
                out.push(std::mem::take(&mut cur));
                if out.len() >= max_lines {
                    truncated = true;
                    continue 'outer;
                }
                cur = word.to_string();
            }
        }
        out.push(cur);
    }
    if out.len() > max_lines {
        out.truncate(max_lines);
        truncated = true;
    }
    if truncated {
        if let Some(last) = out.last_mut() {
            last.push('…');
        }
    }
    out
}

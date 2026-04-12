//! Code minimap — a zoomed-out, syntax-colored overview of the document
//! drawn in the right margin of the editor. Clicking or dragging the minimap
//! scrolls the editor. The viewport indicator replaces the scrollbar when the
//! minimap is visible.

use lntrn_render::{Color, Painter, Rect};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::editor::{self, Editor};
use crate::syntax::{tokenize_line, Token};
use crate::theme::Theme;

/// Logical width of the minimap column.
pub const MINIMAP_W: f32 = 80.0;
/// Height of each source line in the minimap (logical px).
const LINE_H: f32 = 2.5;
/// Width per character in the minimap (logical px).
const CHAR_W: f32 = 1.0;
/// Horizontal padding inside the minimap.
const PAD: f32 = 4.0;

/// Draw the minimap and register its hit zone. Returns nothing — scrolling is
/// handled by the caller via the zone ID.
pub fn draw_minimap(
    painter: &mut Painter,
    editor: &Editor,
    input: &mut InteractionContext,
    rect: Rect,
    theme: Theme,
    palette: &FoxPalette,
    scale: f32,
    zone_id: u32,
) {
    // Background — slightly offset from the editor bg for depth.
    painter.rect_filled(rect, 0.0, palette.surface);

    // Faint left edge separator.
    painter.line(rect.x, rect.y, rect.x, rect.y + rect.h, 1.0 * scale, palette.surface_2);

    let s = scale;
    let mini_line_h = LINE_H * s;
    let char_w = CHAR_W * s;
    let pad = PAD * s;
    let max_chars = ((rect.w - pad * 2.0) / char_w) as usize;
    let num_lines = editor.lines.len();
    let total_mini_h = num_lines as f32 * mini_line_h;

    // Minimap scroll: when the document is taller than the minimap viewport,
    // scroll proportionally with the editor.
    let editor_line_h = editor::FONT_SIZE * editor::LINE_HEIGHT * s;
    let editor_content_h = editor.content_height(s);
    let max_editor_scroll = (editor_content_h - rect.h).max(1.0);
    let scroll_frac = (editor.scroll_offset / max_editor_scroll).clamp(0.0, 1.0);
    let max_mini_scroll = (total_mini_h - rect.h).max(0.0);
    let mini_scroll = scroll_frac * max_mini_scroll;

    // ── Viewport indicator ───────────────────────────────────────────
    let visible_lines = rect.h / editor_line_h;
    let first_vis = editor.scroll_offset / editor_line_h;
    let ind_y = rect.y + first_vis * mini_line_h - mini_scroll;
    let ind_h = visible_lines * mini_line_h;
    // Clamp to minimap bounds.
    let ind_top = ind_y.max(rect.y);
    let ind_bot = (ind_y + ind_h).min(rect.y + rect.h);
    if ind_bot > ind_top {
        painter.rect_filled(
            Rect::new(rect.x, ind_top, rect.w, ind_bot - ind_top),
            0.0,
            Color::from_rgba8(255, 255, 255, 25),
        );
        // Top/bottom edges of the indicator.
        painter.line(rect.x, ind_top, rect.x + rect.w, ind_top, 1.0 * s, palette.accent.with_alpha(0.3));
        painter.line(rect.x, ind_bot, rect.x + rect.w, ind_bot, 1.0 * s, palette.accent.with_alpha(0.3));
    }

    // ── Per-line colored blocks ──────────────────────────────────────
    // Only draw lines that fall within the visible minimap viewport.
    let first_line = ((mini_scroll / mini_line_h).floor() as usize).min(num_lines);
    let visible_count = ((rect.h / mini_line_h).ceil() as usize) + 1;
    let last_line = (first_line + visible_count).min(num_lines);

    for i in first_line..last_line {
        let y = rect.y + i as f32 * mini_line_h - mini_scroll;
        if y + mini_line_h < rect.y || y > rect.y + rect.h {
            continue;
        }

        let line = &editor.lines[i];
        if line.trim().is_empty() {
            continue;
        }

        let tokens = tokenize_line(line, editor.language);
        if tokens.is_empty() {
            // Plain text — draw a single bar in the default text color.
            let trimmed_start = line.len() - line.trim_start().len();
            let trimmed_len = line.trim().len().min(max_chars);
            let x = rect.x + pad + trimmed_start.min(max_chars) as f32 * char_w;
            let w = trimmed_len as f32 * char_w;
            painter.rect_filled(
                Rect::new(x, y, w, mini_line_h - 0.5 * s),
                0.0,
                palette.text.with_alpha(0.2),
            );
        } else {
            // Draw each token as a colored block.
            draw_token_blocks(painter, &tokens, line, rect.x + pad, y, mini_line_h - 0.5 * s, char_w, max_chars, theme, palette, s);
        }
    }

    // Register the whole minimap as a hit zone for click-to-scroll.
    input.add_zone(zone_id, rect);
}

/// Draw colored blocks for each token in a line, plus plain-text gaps.
fn draw_token_blocks(
    painter: &mut Painter,
    tokens: &[Token],
    line: &str,
    base_x: f32,
    y: f32,
    h: f32,
    char_w: f32,
    max_chars: usize,
    theme: Theme,
    palette: &FoxPalette,
    _s: f32,
) {
    let line_len = line.len();
    let mut prev_end: usize = 0;

    for token in tokens {
        // Gap before this token = plain text.
        if token.start > prev_end {
            let gap_text = &line[prev_end..token.start];
            if !gap_text.trim().is_empty() {
                let start_ch = char_offset(line, prev_end).min(max_chars);
                let end_ch = char_offset(line, token.start).min(max_chars);
                if end_ch > start_ch {
                    let x = base_x + start_ch as f32 * char_w;
                    let w = (end_ch - start_ch) as f32 * char_w;
                    painter.rect_filled(Rect::new(x, y, w, h), 0.0, palette.text.with_alpha(0.16));
                }
            }
        }

        // The token itself — use the syntax color with reduced alpha.
        let start_ch = char_offset(line, token.start).min(max_chars);
        let end_ch = char_offset(line, token.end).min(max_chars);
        if end_ch > start_ch {
            let color = theme.syntax_color(token.kind).with_alpha(0.47);
            let x = base_x + start_ch as f32 * char_w;
            let w = (end_ch - start_ch) as f32 * char_w;
            painter.rect_filled(Rect::new(x, y, w, h), 0.0, color);
        }

        prev_end = token.end;
    }

    // Trailing plain text after the last token.
    if prev_end < line_len {
        let tail = &line[prev_end..];
        if !tail.trim().is_empty() {
            let start_ch = char_offset(line, prev_end).min(max_chars);
            let end_ch = char_offset(line, line_len).min(max_chars);
            if end_ch > start_ch {
                let x = base_x + start_ch as f32 * char_w;
                let w = (end_ch - start_ch) as f32 * char_w;
                painter.rect_filled(Rect::new(x, y, w, h), 0.0, palette.text.with_alpha(0.16));
            }
        }
    }
}

/// Approximate byte offset → character offset. Uses the simple heuristic of
/// counting chars up to the byte position. Good enough for minimap blocks.
fn char_offset(line: &str, byte_pos: usize) -> usize {
    line[..byte_pos.min(line.len())].chars().count()
}

/// Convert a click y-position on the minimap to the editor scroll offset that
/// centers that line in the viewport.
pub fn click_to_scroll(
    click_y: f32,
    rect: Rect,
    editor: &Editor,
    scale: f32,
) -> f32 {
    let s = scale;
    let mini_line_h = LINE_H * s;
    let editor_line_h = editor::FONT_SIZE * editor::LINE_HEIGHT * s;
    let editor_content_h = editor.content_height(s);
    let max_editor_scroll = (editor_content_h - rect.h).max(1.0);
    let scroll_frac = (editor.scroll_offset / max_editor_scroll).clamp(0.0, 1.0);
    let total_mini_h = editor.lines.len() as f32 * mini_line_h;
    let max_mini_scroll = (total_mini_h - rect.h).max(0.0);
    let mini_scroll = scroll_frac * max_mini_scroll;

    // Convert click y to a document line.
    let doc_line = ((click_y - rect.y + mini_scroll) / mini_line_h).max(0.0);
    // Convert to editor scroll, centering the line.
    let target = doc_line * editor_line_h - rect.h * 0.5;
    target.clamp(0.0, max_editor_scroll)
}

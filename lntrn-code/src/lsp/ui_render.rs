//! Drawing helpers for LSP overlays: squiggly underlines under diagnostic
//! ranges, gutter dots, hover popup, completion popup. Kept in its own file
//! so `render.rs` can stay focused on the editor body.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::editor::Editor;

use super::protocol::{
    Diagnostic, SEVERITY_ERROR, SEVERITY_HINT, SEVERITY_INFO, SEVERITY_WARNING,
};
use super::{CompletionState, HoverState};

/// Everything needed to map a (doc line, byte column) to an on-screen X.
/// Render.rs already computed all of these for the text pass; passing them
/// through avoids recomputing.
pub struct Layout<'a> {
    pub editor: &'a Editor,
    pub er: Rect,
    pub content_x: f32,
    pub text_y_start: f32,
    pub line_h: f32,
    pub font_size: f32,
    pub scale: f32,
    pub vis_offsets: &'a [usize],
    pub first_doc: usize,
    pub last_doc: usize,
}

// ── Diagnostics ──────────────────────────────────────────────────────────────

pub fn severity_color(severity: u8, palette: &FoxPalette) -> Color {
    match severity {
        SEVERITY_ERROR => palette.danger,
        SEVERITY_WARNING => palette.warning,
        SEVERITY_INFO => palette.info,
        SEVERITY_HINT => palette.muted,
        _ => palette.danger,
    }
}

pub fn draw_diagnostics(
    painter: &mut Painter,
    text: &mut TextRenderer,
    diagnostics: &[Diagnostic],
    palette: &FoxPalette,
    layout: &Layout,
) {
    let editor = layout.editor;
    // Draw gutter dot first so squigglies can overlay it if we want later.
    let dot_r = 4.0 * layout.scale;
    let gutter_x = layout.er.x + 6.0 * layout.scale;

    // Collapse diagnostics per line to one dot + the highest severity.
    let mut per_line_sev: std::collections::HashMap<usize, u8> =
        std::collections::HashMap::new();
    for d in diagnostics {
        let line = d.range.start.line as usize;
        let sev = d.severity.unwrap_or(SEVERITY_ERROR);
        per_line_sev
            .entry(line)
            .and_modify(|cur| {
                if sev < *cur {
                    *cur = sev;
                }
            })
            .or_insert(sev);
    }
    for (line, sev) in &per_line_sev {
        if *line < layout.first_doc || *line >= layout.last_doc {
            continue;
        }
        let wraps = &editor.wrap_rows[*line];
        let vis_row = layout.vis_offsets[*line];
        let y = layout.text_y_start + vis_row as f32 * layout.line_h
            + layout.line_h * 0.5;
        let _ = wraps; // first wrap row is enough
        let color = severity_color(*sev, palette);
        painter.rect_filled(
            Rect::new(gutter_x, y - dot_r, dot_r * 2.0, dot_r * 2.0),
            dot_r,
            color,
        );
    }

    // Squigglies under the range itself.
    for d in diagnostics {
        let sev = d.severity.unwrap_or(SEVERITY_ERROR);
        let color = severity_color(sev, palette);
        draw_squiggly_range(painter, text, editor, &d.range, color, layout);
    }
}

fn draw_squiggly_range(
    painter: &mut Painter,
    text: &mut TextRenderer,
    editor: &Editor,
    range: &super::protocol::Range,
    color: Color,
    layout: &Layout,
) {
    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;
    if end_line < layout.first_doc || start_line >= layout.last_doc {
        return;
    }
    for line_idx in start_line..=end_line {
        if line_idx < layout.first_doc || line_idx >= layout.last_doc {
            continue;
        }
        let Some(line_str) = editor.lines.get(line_idx) else { continue };
        let line_len = line_str.len();

        let start_byte = if line_idx == start_line {
            utf16_to_byte(line_str, range.start.character)
        } else {
            0
        };
        let end_byte = if line_idx == end_line {
            utf16_to_byte(line_str, range.end.character)
        } else {
            line_len
        };
        if end_byte <= start_byte {
            continue;
        }

        let wraps = &editor.wrap_rows[line_idx];
        for (row_idx, &row_start) in wraps.iter().enumerate() {
            let row_end = wraps.get(row_idx + 1).copied().unwrap_or(line_len);
            let lo = start_byte.max(row_start);
            let hi = end_byte.min(row_end);
            if lo >= hi {
                continue;
            }
            let vis_row = layout.vis_offsets[line_idx] + row_idx;
            let y_row = layout.text_y_start + vis_row as f32 * layout.line_h;
            // Underline sits just below the baseline. `font_size` here is
            // physical pixels and line_h has 1.5 multiplier so there's room.
            let y_underline = y_row + layout.font_size + 2.0 * layout.scale;

            let x1 = layout.content_x
                + crate::render::measure_range(text, editor, line_idx, row_start, lo, layout.font_size);
            let x2 = layout.content_x
                + crate::render::measure_range(text, editor, line_idx, row_start, hi, layout.font_size);
            if x2 <= x1 {
                continue;
            }

            draw_wavy_line(painter, x1, x2, y_underline, layout.scale, color);
        }
    }
}

fn draw_wavy_line(painter: &mut Painter, x1: f32, x2: f32, y: f32, scale: f32, color: Color) {
    let amp = 1.5 * scale;
    let period = 4.0 * scale;
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(((x2 - x1) / period) as usize + 2);
    let mut x = x1;
    let mut up = true;
    while x <= x2 {
        pts.push((x, if up { y - amp } else { y + amp }));
        x += period;
        up = !up;
    }
    pts.push((x2, y));
    painter.polyline(&pts, 1.25 * scale, color);
}

fn utf16_to_byte(line: &str, utf16_col: u32) -> usize {
    let mut u: u32 = 0;
    for (i, ch) in line.char_indices() {
        if u >= utf16_col {
            return i;
        }
        u += ch.len_utf16() as u32;
    }
    line.len()
}

// ── Hover popup ──────────────────────────────────────────────────────────────

pub fn draw_hover(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    hover: &HoverState,
    scale: f32,
    screen_w: u32,
    screen_h: u32,
) {
    if !hover.visible || hover.text.is_empty() {
        return;
    }
    let fs = 16.0 * scale;
    let pad = 10.0 * scale;
    let max_w = 600.0 * scale;
    // Strip markdown fences lightly so the popup isn't full of ``` lines.
    let cleaned = strip_markdown(&hover.text);
    let lines = wrap_lines(text, &cleaned, fs, max_w - pad * 2.0);
    if lines.is_empty() {
        return;
    }
    let text_w = lines
        .iter()
        .map(|l| text.measure_width(l, fs))
        .fold(0.0f32, f32::max)
        .max(60.0 * scale);
    let w = (text_w + pad * 2.0).min(max_w);
    let line_h = fs * 1.3;
    let h = lines.len() as f32 * line_h + pad * 2.0;

    // Clamp inside window.
    let mut x = hover.anchor_x;
    let mut y = hover.anchor_y;
    if x + w > screen_w as f32 {
        x = (screen_w as f32 - w - 4.0 * scale).max(0.0);
    }
    if y + h > screen_h as f32 {
        y = (hover.anchor_y - h - 30.0 * scale).max(0.0);
    }

    draw_popup_frame(painter, palette, Rect::new(x, y, w, h), scale);

    for (i, l) in lines.iter().enumerate() {
        let ly = y + pad + i as f32 * line_h;
        TextLabel::new(l, x + pad, ly)
            .size(FontSize::Custom(fs))
            .color(palette.text)
            .draw(text, screen_w, screen_h);
    }
}

fn strip_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_fence = false;
    for line in s.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        // If the whole line is just markdown emphasis, keep but drop the asterisks.
        let cleaned = line.replace("**", "").replace("__", "");
        out.push_str(&cleaned);
        out.push('\n');
    }
    // suppress leading/trailing blank lines
    out.trim_matches('\n').to_string()
}

/// Greedy word wrap. Measures width via `TextRenderer::measure_width`. Long
/// unbroken tokens are hard-cut at the wrap width.
fn wrap_lines(text: &mut TextRenderer, content: &str, fs: f32, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in content.lines().take(16) {
        let mut cur = String::new();
        for word in raw_line.split_inclusive(char::is_whitespace) {
            let try_line = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur}{word}")
            };
            if text.measure_width(&try_line, fs) <= max_w {
                cur = try_line;
            } else {
                if !cur.trim().is_empty() {
                    out.push(cur.trim_end().to_string());
                }
                cur = word.to_string();
            }
        }
        if !cur.trim().is_empty() {
            out.push(cur.trim_end().to_string());
        }
        if out.len() >= 16 {
            break;
        }
    }
    out
}

fn draw_popup_frame(painter: &mut Painter, palette: &FoxPalette, rect: Rect, scale: f32) {
    // Soft shadow, then surface, then hairline border.
    painter.rect_filled(
        Rect::new(rect.x + 2.0 * scale, rect.y + 4.0 * scale, rect.w, rect.h),
        8.0 * scale,
        Color::from_rgba8(0, 0, 0, 80),
    );
    painter.rect_filled(rect, 8.0 * scale, palette.surface_2);
    // 1px border — use four thin rects (painter has no stroke_rect).
    let c = Color::from_rgba8(0, 0, 0, 40);
    painter.rect_filled(Rect::new(rect.x, rect.y, rect.w, 1.0 * scale), 0.0, c);
    painter.rect_filled(Rect::new(rect.x, rect.y + rect.h - 1.0 * scale, rect.w, 1.0 * scale), 0.0, c);
    painter.rect_filled(Rect::new(rect.x, rect.y, 1.0 * scale, rect.h), 0.0, c);
    painter.rect_filled(Rect::new(rect.x + rect.w - 1.0 * scale, rect.y, 1.0 * scale, rect.h), 0.0, c);
}

// ── Completion popup ─────────────────────────────────────────────────────────

pub fn draw_completion(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    completion: &CompletionState,
    scale: f32,
    screen_w: u32,
    screen_h: u32,
) {
    if !completion.visible {
        return;
    }
    let items = completion.filtered();
    if items.is_empty() {
        return;
    }
    let fs = 16.0 * scale;
    let row_h = fs * 1.5;
    let pad = 8.0 * scale;
    let visible_rows = items.len().min(10);
    let h = visible_rows as f32 * row_h + pad;

    // Width: widest label + detail + kind badge + padding, clamped.
    let mut widest = 120.0 * scale;
    for it in items.iter().take(visible_rows) {
        let w_label = text.measure_width(&it.label, fs);
        let w_detail = it
            .detail
            .as_deref()
            .map(|d| text.measure_width(d, fs))
            .unwrap_or(0.0);
        widest = widest.max(w_label + w_detail + 40.0 * scale);
    }
    let w = widest.min(520.0 * scale).max(200.0 * scale);

    let mut x = completion.anchor_x;
    let mut y = completion.anchor_y;
    if x + w > screen_w as f32 {
        x = (screen_w as f32 - w - 4.0 * scale).max(0.0);
    }
    if y + h > screen_h as f32 {
        y = (completion.anchor_y - h - row_h).max(0.0);
    }

    draw_popup_frame(painter, palette, Rect::new(x, y, w, h), scale);

    // Visible window around the selected item.
    let start = completion.selected.saturating_sub(visible_rows.saturating_sub(1));
    let start = start.min(items.len().saturating_sub(visible_rows));
    for (row, it) in items
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
    {
        let rel = row - start;
        let row_y = y + pad * 0.5 + rel as f32 * row_h;
        let highlighted = row == completion.selected;
        if highlighted {
            painter.rect_filled(
                Rect::new(x + 4.0 * scale, row_y, w - 8.0 * scale, row_h),
                4.0 * scale,
                palette.accent.with_alpha(0.18),
            );
        }
        // Kind badge — small colored letter based on CompletionItemKind.
        let badge = kind_badge_letter(it.kind);
        TextLabel::new(badge, x + pad, row_y + (row_h - fs) * 0.5)
            .size(FontSize::Custom(fs))
            .color(palette.accent)
            .draw(text, screen_w, screen_h);

        let label_x = x + pad + 20.0 * scale;
        let mut lbl = TextLabel::new(&it.label, label_x, row_y + (row_h - fs) * 0.5)
            .size(FontSize::Custom(fs))
            .color(palette.text);
        if highlighted {
            lbl = lbl.bold();
        }
        lbl.draw(text, screen_w, screen_h);

        if let Some(detail) = &it.detail {
            let dw = text.measure_width(detail, fs);
            TextLabel::new(detail, x + w - pad - dw, row_y + (row_h - fs) * 0.5)
                .size(FontSize::Custom(fs))
                .color(palette.text_secondary)
                .draw(text, screen_w, screen_h);
        }
    }
}

/// Map LSP `CompletionItemKind` → single-letter badge. Keeps the UI
/// glanceable without needing icon fonts.
fn kind_badge_letter(kind: Option<u8>) -> &'static str {
    // LSP 3.17: 1=Text, 2=Method, 3=Function, 4=Constructor, 5=Field,
    // 6=Variable, 7=Class, 8=Interface, 9=Module, 10=Property, 14=Keyword,
    // 15=Snippet, 21=Constant, 22=Struct, 23=Event, 25=TypeParameter, etc.
    match kind {
        Some(2) | Some(3) => "fn",
        Some(4) => "c",
        Some(5) => "f",
        Some(6) => "v",
        Some(7) | Some(22) => "s",
        Some(8) => "i",
        Some(9) => "m",
        Some(10) => "p",
        Some(14) => "k",
        Some(21) => "C",
        Some(25) => "t",
        _ => "·",
    }
}

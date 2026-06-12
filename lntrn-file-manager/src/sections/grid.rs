use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, ScrollArea, TextLabel};

use crate::fs::FileEntry;
use crate::layout::{icon_size, item_size};

use super::{selection_tint, truncate_with_ellipsis, wrap_lines};

// ── Content grid ────────────────────────────────────────────────────────────

pub fn draw_content_grid(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    content_rect: Rect,
    entries: &[FileEntry],
    cols: usize,
    area: &ScrollArea,
    hovered: &[bool],
    has_icon: &[bool],
    drag_item: Option<usize>,
    renaming: Option<usize>,
    screen: (u32, u32),
    s: f32,
    zoom: f32,
) {
    let isz = item_size(s, zoom);
    let icsz = icon_size(s, zoom);
    let pad = 8.0 * s;

    // No bg fill — window-level bg already covers this area (would otherwise
    // compound the alpha and look near-opaque under transparency).
    area.begin(painter, text);

    let base_y = area.content_y();
    let content_top = content_rect.y;
    let content_bottom = content_rect.y + content_rect.h;

    for (index, entry) in entries.iter().enumerate() {
        let is_dragging = drag_item == Some(index);

        let col = index % cols.max(1);
        let row = index / cols.max(1);
        let x = content_rect.x + pad + col as f32 * (isz + pad);
        let y = base_y + pad + row as f32 * (isz + pad);

        if y + isz < content_top || y > content_bottom {
            continue;
        }

        // Dim the item being dragged but keep it in place (ghost follows cursor)
        let alpha = if is_dragging { 0.3 } else { 1.0 };


        // Highlight matches the tight hitbox, not the full cell, so the
        // hover/selection pill hugs the icon like the clickable area does.
        let item_rect = crate::layout::item_hit_rect(Rect::new(x, y, isz, isz), s, zoom);
        if entry.selected {
            let tint = selection_tint(palette);
            painter.rect_filled(item_rect, 6.0 * s, tint.with_alpha(0.25 * alpha));
            painter.rect_stroke(item_rect, 6.0 * s, 1.0 * s, tint.with_alpha(0.5 * alpha));
        } else if hovered.get(index).copied().unwrap_or(false) {
            painter.rect_filled(item_rect, 6.0 * s, palette.surface_2.with_alpha(0.4));
        }

        // Only draw procedural icons when no texture icon is available
        let label_font = 16.0 * s;
        let content_h = icsz + 2.0 * s + label_font;
        let top_pad = (isz - content_h) * 0.5;
        if !has_icon.get(index).copied().unwrap_or(false) {
            let icon_x = x + (isz - icsz) * 0.5;
            let icon_y = y + top_pad;
            if entry.is_dir {
                let body = Rect::new(icon_x, icon_y + 8.0 * s, icsz, icsz - 12.0 * s);
                painter.rect_filled(body, 4.0 * s, palette.accent.with_alpha(0.3 * alpha));
                painter.rect_stroke(body, 4.0 * s, 1.5 * s, palette.accent.with_alpha(alpha));
                let tab = Rect::new(icon_x, icon_y + 4.0 * s, icsz * 0.45, 8.0 * s);
                painter.rect_filled(tab, 2.0 * s, palette.accent.with_alpha(0.3 * alpha));
                painter.rect_stroke(tab, 2.0 * s, 1.5 * s, palette.accent.with_alpha(alpha));
            } else {
                let page = Rect::new(icon_x + 4.0 * s, icon_y + 2.0 * s, icsz - 8.0 * s, icsz - 4.0 * s);
                painter.rect_filled(page, 3.0 * s, Color::from_rgb8(72, 72, 72).with_alpha(alpha));
                painter.rect_stroke(page, 3.0 * s, 1.5 * s, Color::from_rgb8(102, 102, 102).with_alpha(alpha));
                let fold_x = icon_x + icsz - 16.0 * s;
                let fold_y = icon_y + 2.0 * s;
                painter.rect_filled(Rect::new(fold_x, fold_y, 12.0 * s, 12.0 * s), 0.0, palette.surface.with_alpha(alpha));
                painter.line(fold_x, fold_y, fold_x, fold_y + 12.0 * s, 1.0 * s, Color::from_rgb8(102, 102, 102).with_alpha(alpha));
                painter.line(fold_x, fold_y + 12.0 * s, fold_x + 12.0 * s, fold_y + 12.0 * s, 1.0 * s, Color::from_rgb8(102, 102, 102).with_alpha(alpha));
                let lx = icon_x + 10.0 * s;
                let lw = icsz - 24.0 * s;
                painter.line(lx, icon_y + 20.0*s, lx + lw, icon_y + 20.0*s, 1.5*s, palette.accent.with_alpha(0.6 * alpha));
                painter.line(lx, icon_y + 26.0*s, lx + lw*0.8, icon_y + 26.0*s, 1.5*s, Color::from_rgb8(224, 90, 138).with_alpha(0.6 * alpha));
                painter.line(lx, icon_y + 32.0*s, lx + lw*0.6, icon_y + 32.0*s, 1.5*s, Color::from_rgb8(155, 93, 229).with_alpha(0.6 * alpha));
            }
        }

        let label_y = y + top_pad + icsz + 2.0 * s;
        let min_margin = 2.0 * s;
        let max_label_w = isz - min_margin * 2.0;
        let char_w = label_font * 0.52;

        // Skip label for item being renamed (TextInput draws instead)
        if renaming == Some(index) { continue; }

        // Only draw text if the label is within the visible content area
        if label_y >= content_top && label_y + label_font <= content_bottom {
            let label_color = palette.text.with_alpha(alpha);
            if entry.selected {
                let lines = wrap_lines(&entry.name, max_label_w, char_w);
                let line_h = label_font * 1.25;
                for (li, line) in lines.iter().enumerate() {
                    let ly = label_y + li as f32 * line_h;
                    let est_w = line.len() as f32 * char_w;
                    let lx = (x + (isz - est_w) * 0.5).max(x + min_margin);
                    TextLabel::new(line, lx, ly)
                        .size(FontSize::Custom(label_font))
                        .color(label_color)
                        .max_width(max_label_w)
                        .draw(text, screen.0, screen.1);
                }
            } else {
                let display_name = truncate_with_ellipsis(&entry.name, max_label_w, char_w);
                let est_w = display_name.len() as f32 * char_w;
                let label_x = (x + (isz - est_w) * 0.5).max(x + min_margin);
                TextLabel::new(&display_name, label_x, label_y)
                    .size(FontSize::Custom(label_font))
                    .color(label_color)
                    .max_width(max_label_w)
                    .draw(text, screen.0, screen.1);
            }
        }

    }

    area.end(painter, text);
}

// ── Rubber band selection ────────────────────────────────────────────────────

pub fn draw_rubber_band(
    painter: &mut Painter,
    palette: &FoxPalette,
    start: (f32, f32),
    end: (f32, f32),
    content_rect: Rect,
) {
    let x0 = start.0.min(end.0).max(content_rect.x);
    let y0 = start.1.min(end.1).max(content_rect.y);
    let x1 = start.0.max(end.0).min(content_rect.x + content_rect.w);
    let y1 = start.1.max(end.1).min(content_rect.y + content_rect.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let r = Rect::new(x0, y0, x1 - x0, y1 - y0);
    painter.rect_filled(r, 2.0, selection_tint(palette).with_alpha(0.18));
    painter.rect_stroke(r, 2.0, 1.0, palette.accent.with_alpha(0.6));
}

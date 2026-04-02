use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, ScrollArea, TextLabel};
use std::time::SystemTime;

use crate::app::TreeEntry;
use crate::fs::FileEntry;
use crate::layout::sidebar_w;
use crate::sections::{selection_tint, truncate_with_ellipsis};

// ── List view ───────────────────────────────────────────────────────────────

pub fn draw_content_list(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    content_rect: Rect,
    entries: &[FileEntry],
    area: &ScrollArea,
    hovered: &[bool],
    has_icon: &[bool],
    drag_item: Option<usize>,
    renaming: Option<usize>,
    screen: (u32, u32),
    s: f32,
    _ox: f32,
) {
    let row_h = 40.0 * s;
    let font = FontSize::Custom(24.0 * s);
    let small_font = FontSize::Custom(20.0 * s);

    painter.rect_filled(content_rect, 0.0, palette.bg);

    // Column header
    let hdr_y = content_rect.y;
    let hdr_h = 32.0 * s;
    painter.rect_filled(
        Rect::new(content_rect.x, hdr_y, content_rect.w, hdr_h),
        0.0, palette.surface,
    );
    let name_x = content_rect.x + 42.0 * s;
    let size_x = content_rect.x + content_rect.w - 220.0 * s;
    let date_x = content_rect.x + content_rect.w - 120.0 * s;
    TextLabel::new("Name", name_x, hdr_y + 5.0 * s)
        .size(FontSize::Custom(20.0 * s)).color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    TextLabel::new("Size", size_x, hdr_y + 5.0 * s)
        .size(FontSize::Custom(20.0 * s)).color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    TextLabel::new("Modified", date_x, hdr_y + 5.0 * s)
        .size(FontSize::Custom(20.0 * s)).color(palette.text_secondary)
        .draw(text, screen.0, screen.1);

    area.begin(painter, text);
    let base_y = area.content_y();
    let content_top = content_rect.y + hdr_h;
    let content_bottom = content_rect.y + content_rect.h;

    for (index, entry) in entries.iter().enumerate() {
        let y = base_y + hdr_h + index as f32 * row_h;
        if y + row_h < content_top || y > content_bottom { continue; }

        let row_rect = Rect::new(content_rect.x, y, content_rect.w, row_h);
        let is_dragging = drag_item == Some(index);
        let alpha = if is_dragging { 0.3 } else { 1.0 };

        // Selection / hover background
        if entry.selected {
            let tint = selection_tint(palette);
            painter.rect_filled(row_rect, 0.0, tint.with_alpha(0.2 * alpha));
        } else if hovered.get(index).copied().unwrap_or(false) {
            painter.rect_filled(row_rect, 0.0, palette.surface_2.with_alpha(0.3));
        }

        // Alternating row tint
        if index % 2 == 1 && !entry.selected {
            painter.rect_filled(row_rect, 0.0, Color::WHITE.with_alpha(0.02));
        }

        if renaming == Some(index) { continue; }

        // Mini icon (fallback when no texture icon loaded)
        if !has_icon.get(index).copied().unwrap_or(false) {
            let icon_x = content_rect.x + 8.0 * s;
            let icon_y = y + (row_h - 24.0 * s) * 0.5;
            let icon_sz = 24.0 * s;
            if entry.is_dir {
                painter.rect_filled(Rect::new(icon_x, icon_y + 4.0*s, icon_sz, icon_sz - 6.0*s), 2.0*s, palette.accent.with_alpha(0.5 * alpha));
                painter.rect_filled(Rect::new(icon_x, icon_y + 2.0*s, icon_sz * 0.45, 4.0*s), 1.0*s, palette.accent.with_alpha(0.5 * alpha));
            } else {
                painter.rect_filled(Rect::new(icon_x + 2.0*s, icon_y, icon_sz - 4.0*s, icon_sz), 2.0*s, Color::from_rgb8(72, 72, 72).with_alpha(alpha));
            }
        }

        // Name
        let text_y = y + (row_h - 24.0 * s) * 0.5;
        let max_name_w = size_x - name_x - 12.0 * s;
        let name_color = if entry.selected { palette.accent.with_alpha(alpha) } else { palette.text.with_alpha(alpha) };
        let display = truncate_with_ellipsis(&entry.name, max_name_w, 24.0 * s * 0.52);
        TextLabel::new(&display, name_x, text_y)
            .size(font).color(name_color).max_width(max_name_w)
            .draw(text, screen.0, screen.1);

        // Size
        let size_str = if entry.is_dir { "--".to_string() } else { format_bytes(entry.size) };
        TextLabel::new(&size_str, size_x, text_y)
            .size(small_font).color(palette.muted.with_alpha(alpha))
            .draw(text, screen.0, screen.1);

        // Modified date
        let date_str = format_date(entry.modified);
        TextLabel::new(&date_str, date_x, text_y)
            .size(small_font).color(palette.muted.with_alpha(alpha))
            .draw(text, screen.0, screen.1);

        // Divider
        painter.rect_filled(
            Rect::new(content_rect.x + 8.0*s, y + row_h - 0.5*s, content_rect.w - 16.0*s, 0.5*s),
            0.0, Color::WHITE.with_alpha(0.05),
        );
    }
    area.end(painter, text);
}

// ── Tree view ───────────────────────────────────────────────────────────────

pub fn draw_content_tree(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    content_rect: Rect,
    tree_entries: &[TreeEntry],
    area: &ScrollArea,
    hovered: &[bool],
    has_icon: &[bool],
    screen: (u32, u32),
    s: f32,
    _ox: f32,
) {
    let row_h = 36.0 * s;
    let indent = 28.0 * s;
    let font = FontSize::Custom(24.0 * s);

    painter.rect_filled(content_rect, 0.0, palette.bg);
    area.begin(painter, text);
    let base_y = area.content_y();
    let content_top = content_rect.y;
    let content_bottom = content_rect.y + content_rect.h;

    for (index, te) in tree_entries.iter().enumerate() {
        let y = base_y + index as f32 * row_h;
        if y + row_h < content_top || y > content_bottom { continue; }

        let x_offset = te.depth as f32 * indent;
        let row_x = content_rect.x + 8.0 * s + x_offset;
        let row_rect = Rect::new(content_rect.x, y, content_rect.w, row_h);

        // Hover
        if hovered.get(index).copied().unwrap_or(false) {
            painter.rect_filled(row_rect, 0.0, palette.surface_2.with_alpha(0.3));
        }

        // Draw tree guide lines
        if te.depth > 0 {
            let guide_x = content_rect.x + 8.0 * s + (te.depth as f32 - 1.0) * indent + 8.0 * s;
            painter.line(guide_x, y, guide_x, y + row_h * 0.5, 1.0 * s, palette.muted.with_alpha(0.2));
            painter.line(guide_x, y + row_h * 0.5, guide_x + indent * 0.6, y + row_h * 0.5, 1.0 * s, palette.muted.with_alpha(0.2));
        }

        // Expand/collapse arrow for directories
        if te.entry.is_dir {
            let arrow_x = row_x + 2.0 * s;
            let arrow_y = y + row_h * 0.5;
            let ar = 4.0 * s;
            let arrow_color = palette.text_secondary;
            if te.is_expanded {
                // Down arrow (▼)
                painter.line(arrow_x - ar, arrow_y - ar * 0.5, arrow_x, arrow_y + ar * 0.5, 1.5*s, arrow_color);
                painter.line(arrow_x, arrow_y + ar * 0.5, arrow_x + ar, arrow_y - ar * 0.5, 1.5*s, arrow_color);
            } else {
                // Right arrow (▶)
                painter.line(arrow_x - ar * 0.5, arrow_y - ar, arrow_x + ar * 0.5, arrow_y, 1.5*s, arrow_color);
                painter.line(arrow_x + ar * 0.5, arrow_y, arrow_x - ar * 0.5, arrow_y + ar, 1.5*s, arrow_color);
            }
        }

        // Icon (fallback when no texture icon loaded)
        let icon_x = row_x + 16.0 * s;
        let icon_y = y + (row_h - 20.0 * s) * 0.5;
        let icon_sz = 20.0 * s;
        if !has_icon.get(index).copied().unwrap_or(false) {
            if te.entry.is_dir {
                painter.rect_filled(Rect::new(icon_x, icon_y + 3.0*s, icon_sz, icon_sz - 5.0*s), 2.0*s, palette.accent.with_alpha(0.5));
                painter.rect_filled(Rect::new(icon_x, icon_y + 1.0*s, icon_sz * 0.4, 3.0*s), 1.0*s, palette.accent.with_alpha(0.5));
            } else {
                painter.rect_filled(Rect::new(icon_x + 1.0*s, icon_y, icon_sz - 2.0*s, icon_sz), 2.0*s, Color::from_rgb8(72, 72, 72));
            }
        }

        // Name
        let name_x = icon_x + icon_sz + 8.0 * s;
        let text_y = y + (row_h - 24.0 * s) * 0.5;
        let max_w = content_rect.x + content_rect.w - name_x - 12.0 * s;
        let name_color = if te.entry.is_dir { palette.text } else { palette.text };
        TextLabel::new(&te.entry.name, name_x, text_y)
            .size(font).color(name_color).max_width(max_w)
            .draw(text, screen.0, screen.1);
    }
    area.end(painter, text);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn format_bytes(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let size_f = size as f64;
    if size_f >= GB {
        format!("{:.1} GB", size_f / GB)
    } else if size_f >= MB {
        format!("{:.1} MB", size_f / MB)
    } else if size_f >= KB {
        format!("{:.0} KB", size_f / KB)
    } else {
        format!("{} B", size)
    }
}

fn format_date(modified: Option<SystemTime>) -> String {
    let Some(time) = modified else { return "--".into() };
    let Ok(dur) = time.duration_since(SystemTime::UNIX_EPOCH) else { return "--".into() };
    let secs = dur.as_secs();
    let days = secs / 86400;
    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if remaining < year_days { break; }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let month_names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut m = 0usize;
    while m < 12 && remaining >= month_days[m] {
        remaining -= month_days[m];
        m += 1;
    }
    format!("{} {}, {}", month_names[m], remaining + 1, y)
}

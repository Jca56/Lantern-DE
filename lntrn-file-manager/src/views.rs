use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, ScrollArea, TextLabel};
use std::time::SystemTime;

use std::path::Path;

use crate::app::TreeEntry;
use crate::fs::FileEntry;
use crate::sections::{selection_tint, truncate_to_width};

// ── Row hit rects ───────────────────────────────────────────────────────────
//
// List/Tree rows span the full content width visually (stripes, dividers),
// but only the icon + name block is clickable — the rest of the row is empty
// space, so right-clicks beside a name reach the folder menu and clicks there
// don't select. Mirrors grid view's `item_hit_rect`. Zone registration
// (render.rs) and the highlight pills below share these so the hitbox and
// the visuals can't drift.

/// List view column x positions (normal mode): `(name_x, size_x, date_x)`.
/// The date column reserves ~180*m px ("Sep 30, 2026" at font 20*m) plus a
/// small right gutter so it doesn't kiss the preview pane / window edge.
fn list_columns(content_rect: Rect, m: f32, s: f32) -> (f32, f32, f32) {
    let right_pad = 12.0 * m * s;
    let date_w = 180.0 * m * s;
    let size_w = 110.0 * m * s;
    let date_x = content_rect.x + content_rect.w - right_pad - date_w;
    let size_x = date_x - size_w;
    let name_x = content_rect.x + 42.0 * m * s;
    (name_x, size_x, date_x)
}

/// Tight hit/highlight rect for a normal-mode list row: mini icon through the
/// end of the (truncated) name, full row height.
pub fn list_row_hit_rect(
    text: &mut TextRenderer,
    entry: &FileEntry,
    content_rect: Rect,
    y: f32,
    row_h: f32,
    s: f32,
    zoom: f32,
) -> Rect {
    let m = crate::layout::list_zoom_multiplier(zoom);
    let font_px = 24.0 * m * s;
    let (name_x, size_x, _) = list_columns(content_rect, m, s);
    let max_name_w = size_x - name_x - 12.0 * m * s;
    // Selected rows draw the untruncated name, so measure what's shown.
    let name_w = if entry.selected {
        text.measure_width(&entry.name, font_px)
    } else {
        let display = truncate_to_width(text, &entry.name, max_name_w, font_px);
        text.measure_width(&display, font_px)
    };
    let x = content_rect.x + 4.0 * m * s;
    let right = (name_x + name_w + 10.0 * m * s).min(content_rect.x + content_rect.w);
    Rect::new(x, y, right - x, row_h)
}

/// Tight hit/highlight rect for a tree row: expand arrow through the end of
/// the (truncated) name, full row height.
pub fn tree_row_hit_rect(
    text: &mut TextRenderer,
    te: &TreeEntry,
    content_rect: Rect,
    y: f32,
    row_h: f32,
    s: f32,
    zoom: f32,
) -> Rect {
    let m = crate::layout::list_zoom_multiplier(zoom);
    let indent = 28.0 * m * s;
    let row_x = content_rect.x + 8.0 * m * s + te.depth as f32 * indent;
    let name_x = row_x + 16.0 * m * s + 20.0 * m * s + 8.0 * m * s; // arrow gap + icon + gap
    let max_w = content_rect.x + content_rect.w - name_x - 12.0 * m * s;
    let font_px = 24.0 * m * s;
    let display = truncate_to_width(text, &te.entry.name, max_w, font_px);
    let name_w = text.measure_width(&display, font_px);
    // Start left of `row_x` — the expand arrow is drawn at row_x ± 6*m*s.
    let x = row_x - 6.0 * m * s;
    let right = (name_x + name_w + 10.0 * m * s).min(content_rect.x + content_rect.w);
    Rect::new(x, y, right - x, row_h)
}

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
    search_root: Option<&Path>,
    git: &crate::git_status::GitStatus,
    screen: (u32, u32),
    s: f32,
    zoom: f32,
) {
    let searching = search_root.is_some();
    let m = crate::layout::list_zoom_multiplier(zoom);
    let row_h = if searching { 56.0 * m * s } else { 40.0 * m * s };
    let font = FontSize::Custom(24.0 * m * s);
    let small_font = FontSize::Custom(20.0 * m * s);
    let path_font = FontSize::Custom(16.0 * m * s);

    // No bg fill here — the window-level bg already covers this area, and
    // re-painting it would double-paint the alpha and break transparency.

    // Column header
    let hdr_y = content_rect.y;
    let hdr_h = 32.0 * m * s;
    // Header bg intentionally transparent — labels + the bottom 1px separator
    // below still delimit the row from the file list.
    painter.rect_filled(
        Rect::new(content_rect.x, hdr_y + hdr_h - 1.0, content_rect.w, 1.0),
        0.0, palette.muted.with_alpha(0.2),
    );
    let (name_x, size_x, date_x) = list_columns(content_rect, m, s);
    let size_w = 110.0 * m * s;
    let date_w = 180.0 * m * s;
    let hdr_font = FontSize::Custom(20.0 * m * s);
    TextLabel::new("Name", name_x, hdr_y + 5.0 * m * s)
        .size(hdr_font).color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    if searching {
        TextLabel::new("Location", size_x, hdr_y + 5.0 * m * s)
            .size(hdr_font).color(palette.text_secondary)
            .draw(text, screen.0, screen.1);
    } else {
        TextLabel::new("Size", size_x, hdr_y + 5.0 * m * s)
            .size(hdr_font).color(palette.text_secondary)
            .draw(text, screen.0, screen.1);
        TextLabel::new("Modified", date_x, hdr_y + 5.0 * m * s)
            .size(hdr_font).color(palette.text_secondary)
            .draw(text, screen.0, screen.1);
    }

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

        // Selection / hover background. Normal mode hugs the icon+name block
        // (matching the clickable zone — the size/date columns are empty
        // space); search rows stay full-width like their zones.
        let (hl, radius) = if searching {
            (row_rect, 0.0)
        } else {
            (list_row_hit_rect(text, entry, content_rect, y, row_h, s, zoom), 6.0 * s)
        };
        if entry.selected {
            let tint = selection_tint(palette);
            painter.rect_filled(hl, radius, tint.with_alpha(0.25 * alpha));
            if !searching {
                painter.rect_stroke(hl, radius, 1.0 * s, tint.with_alpha(0.5 * alpha));
            }
        } else if hovered.get(index).copied().unwrap_or(false) {
            painter.rect_filled(hl, radius, palette.surface_2.with_alpha(0.3));
        }

        // Alternating row tint
        if index % 2 == 1 && !entry.selected {
            painter.rect_filled(row_rect, 0.0, Color::WHITE.with_alpha(0.02));
        }

        if renaming == Some(index) { continue; }

        // Mini icon (fallback when no texture icon loaded)
        if !has_icon.get(index).copied().unwrap_or(false) {
            let icon_x = content_rect.x + 8.0 * m * s;
            let icon_sz = 24.0 * m * s;
            let icon_y = y + (row_h - icon_sz) * 0.5;
            if entry.is_dir {
                painter.rect_filled(Rect::new(icon_x, icon_y + 4.0*m*s, icon_sz, icon_sz - 6.0*m*s), 2.0*s, palette.accent.with_alpha(0.5 * alpha));
                painter.rect_filled(Rect::new(icon_x, icon_y + 2.0*m*s, icon_sz * 0.45, 4.0*m*s), 1.0*s, palette.accent.with_alpha(0.5 * alpha));
            } else {
                painter.rect_filled(Rect::new(icon_x + 2.0*m*s, icon_y, icon_sz - 4.0*m*s, icon_sz), 2.0*s, Color::from_rgb8(72, 72, 72).with_alpha(alpha));
            }
        }

        // Git badge — dot at the icon slot's bottom-right corner.
        if !searching {
            if let Some(mark) = git.mark(&entry.path) {
                let color = match mark {
                    crate::git_status::GitMark::Modified => palette.warning,
                    crate::git_status::GitMark::Untracked => palette.success,
                };
                let icon_x = content_rect.x + 8.0 * m * s;
                let icon_sz = 24.0 * m * s;
                let icon_y = y + (row_h - icon_sz) * 0.5;
                let r = 4.0 * m * s;
                let cx = icon_x + icon_sz - r * 0.5;
                let cy = icon_y + icon_sz - r * 0.5;
                painter.rect_filled(Rect::new(cx - r - 1.0 * s, cy - r - 1.0 * s, (r + 1.0 * s) * 2.0, (r + 1.0 * s) * 2.0), r + 1.0 * s, palette.surface.with_alpha(alpha));
                painter.rect_filled(Rect::new(cx - r, cy - r, r * 2.0, r * 2.0), r, color.with_alpha(alpha));
            }
        }

        if searching {
            // Search mode: name on top, path below
            let name_y = y + 6.0 * m * s;
            let max_name_w = content_rect.w - 50.0 * m * s;
            let name_color = palette.text.with_alpha(alpha);
            let display = if entry.selected {
                entry.name.clone()
            } else {
                // Measure exactly — the char-width estimate undertrims wide
                // glyphs and lets the leftover wrap onto the row below.
                truncate_to_width(text, &entry.name, max_name_w, 24.0 * m * s)
            };
            TextLabel::new(&display, name_x, name_y)
                .size(font).color(name_color).max_width(if entry.selected { 9999.0 } else { max_name_w })
                .draw(text, screen.0, screen.1);

            // Parent path (relative to search root)
            let parent = entry.path.parent().unwrap_or(&entry.path);
            let rel_path = if let Some(root) = search_root {
                parent.strip_prefix(root)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| parent.to_string_lossy().to_string())
            } else {
                parent.to_string_lossy().to_string()
            };
            let path_display = if rel_path.is_empty() { "./".to_string() } else { format!("./{rel_path}") };
            let path_y = name_y + 26.0 * m * s;
            let max_path_w = content_rect.w - 50.0 * m * s;
            let path_display = truncate_to_width(text, &path_display, max_path_w, 16.0 * m * s);
            TextLabel::new(&path_display, name_x, path_y)
                .size(path_font).color(palette.muted.with_alpha(alpha * 0.7))
                .max_width(max_path_w)
                .draw(text, screen.0, screen.1);
        } else {
            // Normal mode: name, size, date columns
            let text_y = y + (row_h - 24.0 * m * s) * 0.5;
            let max_name_w = size_x - name_x - 12.0 * m * s;
            let name_color = palette.text.with_alpha(alpha);
            // Exact-measure truncation — see the tree view note below: the
            // char-width estimate let two-word names wrap onto the next row.
            let display = if entry.selected {
                entry.name.clone()
            } else {
                truncate_to_width(text, &entry.name, max_name_w, 24.0 * m * s)
            };
            TextLabel::new(&display, name_x, text_y)
                .size(font).color(name_color).max_width(if entry.selected { 9999.0 } else { max_name_w })
                .draw(text, screen.0, screen.1);

            // Size
            let size_str = if entry.is_dir { "--".to_string() } else { format_bytes(entry.size) };
            TextLabel::new(&size_str, size_x, text_y)
                .size(small_font).color(palette.muted.with_alpha(alpha))
                .max_width(size_w - 8.0 * m * s)
                .draw(text, screen.0, screen.1);

            // Modified date
            let date_str = format_date(entry.modified);
            TextLabel::new(&date_str, date_x, text_y)
                .size(small_font).color(palette.muted.with_alpha(alpha))
                .max_width(date_w)
                .draw(text, screen.0, screen.1);
        }

        // Divider
        painter.rect_filled(
            Rect::new(content_rect.x + 8.0*m*s, y + row_h - 0.5*s, content_rect.w - 16.0*m*s, 0.5*s),
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
    selected: &[bool],
    drag_item: Option<usize>,
    renaming_path: Option<&std::path::Path>,
    screen: (u32, u32),
    s: f32,
    zoom: f32,
) {
    let m = crate::layout::list_zoom_multiplier(zoom);
    let row_h = 36.0 * m * s;
    let indent = 28.0 * m * s;
    let font = FontSize::Custom(24.0 * m * s);

    // No bg fill — window-level bg already covers this area (avoids
    // compounded-alpha double paint).
    area.begin(painter, text);
    let base_y = area.content_y();
    let content_top = content_rect.y;
    let content_bottom = content_rect.y + content_rect.h;

    for (index, te) in tree_entries.iter().enumerate() {
        let y = base_y + index as f32 * row_h;
        if y + row_h < content_top || y > content_bottom { continue; }

        let x_offset = te.depth as f32 * indent;
        let row_x = content_rect.x + 8.0 * m * s + x_offset;
        let is_dragging = drag_item == Some(index);
        let alpha = if is_dragging { 0.3 } else { 1.0 };

        // Selection / hover pill hugs arrow+icon+name (matching the clickable
        // zone) — the rest of the row is empty space.
        let hl = tree_row_hit_rect(text, te, content_rect, y, row_h, s, zoom);
        if selected.get(index).copied().unwrap_or(false) {
            let tint = crate::sections::selection_tint(palette);
            painter.rect_filled(hl, 6.0 * s, tint.with_alpha(0.25 * alpha));
            painter.rect_stroke(hl, 6.0 * s, 1.0 * s, tint.with_alpha(0.5 * alpha));
        } else if hovered.get(index).copied().unwrap_or(false) {
            painter.rect_filled(hl, 6.0 * s, palette.surface_2.with_alpha(0.3));
        }

        // Draw tree guide lines
        if te.depth > 0 {
            let guide_x = content_rect.x + 8.0 * m * s + (te.depth as f32 - 1.0) * indent + 8.0 * m * s;
            painter.line(guide_x, y, guide_x, y + row_h * 0.5, 1.0 * s, palette.muted.with_alpha(0.2));
            painter.line(guide_x, y + row_h * 0.5, guide_x + indent * 0.6, y + row_h * 0.5, 1.0 * s, palette.muted.with_alpha(0.2));
        }

        // Expand/collapse arrow for directories
        if te.entry.is_dir {
            let arrow_x = row_x + 2.0 * m * s;
            let arrow_y = y + row_h * 0.5;
            let ar = 4.0 * m * s;
            let arrow_color = palette.text_secondary.with_alpha(alpha);
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
        let icon_x = row_x + 16.0 * m * s;
        let icon_sz = 20.0 * m * s;
        let icon_y = y + (row_h - icon_sz) * 0.5;
        if !has_icon.get(index).copied().unwrap_or(false) {
            if te.entry.is_dir {
                painter.rect_filled(Rect::new(icon_x, icon_y + 3.0*m*s, icon_sz, icon_sz - 5.0*m*s), 2.0*s, palette.accent.with_alpha(0.5 * alpha));
                painter.rect_filled(Rect::new(icon_x, icon_y + 1.0*m*s, icon_sz * 0.4, 3.0*m*s), 1.0*s, palette.accent.with_alpha(0.5 * alpha));
            } else {
                painter.rect_filled(Rect::new(icon_x + 1.0*m*s, icon_y, icon_sz - 2.0*m*s, icon_sz), 2.0*s, Color::from_rgb8(72, 72, 72).with_alpha(alpha));
            }
        }

        // Name (skip if this row is being renamed — TextInput is drawn over it)
        if renaming_path.map_or(false, |p| p == te.entry.path.as_path()) {
            continue;
        }
        let name_x = icon_x + icon_sz + 8.0 * m * s;
        let text_y = y + (row_h - 24.0 * m * s) * 0.5;
        let max_w = content_rect.x + content_rect.w - name_x - 12.0 * m * s;
        let name_color = palette.text.with_alpha(alpha);
        // Truncate to a single line with an ellipsis using real glyph widths.
        // Passing the raw name with only max_width lets cosmic-text WRAP onto a
        // second line, which then overlaps the row below; the char-width
        // *estimate* undertrims wide glyphs and still wraps, so measure exactly.
        let display = truncate_to_width(text, &te.entry.name, max_w, 24.0 * m * s);
        TextLabel::new(&display, name_x, text_y)
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

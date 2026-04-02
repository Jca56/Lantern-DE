use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FontSize, FoxPalette, GradientStrip, InteractionState, ScrollArea, Scrollbar,
    TextLabel,
};

pub use crate::sidebar::draw_sidebar;
pub use crate::nav_bar::draw_nav_bar;

/// Blend accent toward warning to get a warmer amber-gold for selection highlights.
/// Pure accent (rgb 200,134,10) reads orange at low alpha on dark bg.
pub fn selection_tint(palette: &FoxPalette) -> Color {
    let a = palette.accent;
    let w = palette.warning;
    Color {
        r: a.r * 0.7 + w.r * 0.3,
        g: a.g * 0.7 + w.g * 0.3,
        b: a.b * 0.7 + w.b * 0.3,
        a: 1.0,
    }
}

use crate::{
    fs::FileEntry,
    layout::{icon_size, item_size},
};

// ── Gradient separators ─────────────────────────────────────────────────────

pub fn draw_gradient_h(painter: &mut Painter, palette: &FoxPalette, x: f32, y: f32, width: f32, s: f32) {
    let mut bar = GradientStrip::new(x, y, width);
    bar.height = 4.0 * s;
    bar.colors = palette.file_manager_gradient_stops();
    bar.draw(painter);
}

pub fn draw_gradient_v(painter: &mut Painter, palette: &FoxPalette, x: f32, y: f32, height: f32, s: f32, ox: f32) {
    let colors = palette.file_manager_gradient_stops();
    let w = 4.0 * s;
    let segments = height.max(1.0).ceil() as usize;
    let step = height / segments as f32;
    for i in 0..segments {
        let sy = y + i as f32 * step;
        let sh = if i + 1 == segments { y + height - sy } else { step };
        let t = i as f32 / segments as f32;
        let color = sample_gradient_5(&colors, t);
        painter.rect_filled(Rect::new(x + ox, sy, w, sh), 0.0, color);
    }
}

fn sample_gradient_5(colors: &[Color; 5], t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let stops = [0.0_f32, 0.25, 0.50, 0.75, 1.0];
    for i in 0..4 {
        if t <= stops[i + 1] {
            let local = (t - stops[i]) / (stops[i + 1] - stops[i]);
            return lerp_color(colors[i], colors[i + 1], local);
        }
    }
    colors[4]
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

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
    _ox: f32,
) {
    let isz = item_size(s, zoom);
    let icsz = icon_size(s, zoom);
    let pad = 8.0 * s;

    painter.rect_filled(content_rect, 0.0, palette.bg);
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


        let item_rect = Rect::new(x, y, isz, isz);
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
            let display_name = truncate_with_ellipsis(&entry.name, max_label_w, char_w);
            let est_w = display_name.len() as f32 * char_w;
            let label_x = (x + (isz - est_w) * 0.5).max(x + min_margin);
            TextLabel::new(&display_name, label_x, label_y)
                .size(FontSize::Custom(label_font))
                .color(if entry.selected { palette.accent.with_alpha(alpha) } else { palette.text.with_alpha(alpha) })
                .max_width(max_label_w)
                .draw(text, screen.0, screen.1);
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
    _ox: f32,
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

// ── Scrollbar ───────────────────────────────────────────────────────────────

pub fn draw_scrollbar(
    painter: &mut Painter,
    scrollbar: &Scrollbar,
    state: InteractionState,
    palette: &FoxPalette,
    _ox: f32,
) {
    scrollbar.draw(painter, state, palette);
}

// ── Status bar ──────────────────────────────────────────────────────────────

pub fn draw_status_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    status_rect: Rect,
    entries: &[FileEntry],
    file_info: &mut crate::file_info::FileInfoCache,
    screen: (u32, u32),
    s: f32,
    _ox: f32,
) {
    let r = 10.0 * s;
    painter.rect_filled(status_rect, r, palette.surface);
    painter.rect_filled(Rect::new(status_rect.x, status_rect.y, r, r), 0.0, palette.surface);
    painter.rect_filled(Rect::new(status_rect.x + status_rect.w - r, status_rect.y, r, r), 0.0, palette.surface);
    painter.rect_filled(Rect::new(status_rect.x, status_rect.y, status_rect.w, 1.0), 0.0, palette.bg);

    let total = entries.len();
    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let files = total - dirs;
    let selected: Vec<&FileEntry> = entries.iter().filter(|e| e.selected).collect();
    let sel_count = selected.len();
    let sel_bytes: u64 = selected.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    let font = FontSize::Custom(20.0 * s);
    let dot_sep = " \u{2022} ";
    let mut x = status_rect.x + 12.0 * s;
    let y = status_rect.y + 4.0 * s;
    let cw = 9.0 * s; // approximate char width

    let counts = format!("{dirs} folders, {files} files");
    TextLabel::new(&counts, x, y)
        .size(font)
        .color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    x += counts.len() as f32 * cw;

    if sel_count > 0 {
        // Dot separator
        TextLabel::new(dot_sep, x, y)
            .size(font)
            .color(palette.muted.with_alpha(0.5))
            .draw(text, screen.0, screen.1);
        x += 24.0 * s;

        let sel_text = format!("{sel_count} selected");
        TextLabel::new(&sel_text, x, y)
            .size(font)
            .color(palette.accent)
            .draw(text, screen.0, screen.1);
        x += sel_text.len() as f32 * cw + 6.0 * s;

        let size_text = format!("({})", format_bytes(sel_bytes));
        TextLabel::new(&size_text, x, y)
            .size(font)
            .color(palette.muted)
            .draw(text, screen.0, screen.1);
        x += size_text.len() as f32 * cw;

        // Single file selected — show detailed info
        if sel_count == 1 && !selected[0].is_dir {
            let info = file_info.get(&selected[0].path);

            // File type
            TextLabel::new(dot_sep, x, y)
                .size(font)
                .color(palette.muted.with_alpha(0.5))
                .draw(text, screen.0, screen.1);
            x += 24.0 * s;

            TextLabel::new(&info.type_name, x, y)
                .size(font)
                .color(palette.text_secondary)
                .draw(text, screen.0, screen.1);
            x += info.type_name.len() as f32 * cw;

            // Dimensions (images and video)
            if let Some((w, h)) = info.dimensions {
                TextLabel::new(dot_sep, x, y)
                    .size(font)
                    .color(palette.muted.with_alpha(0.5))
                    .draw(text, screen.0, screen.1);
                x += 24.0 * s;

                let dims = format!("{w}\u{00D7}{h}");
                TextLabel::new(&dims, x, y)
                    .size(font)
                    .color(palette.text_secondary)
                    .draw(text, screen.0, screen.1);
                x += dims.len() as f32 * cw;
            }

            // Duration (audio and video)
            if let Some(ref dur) = info.duration {
                TextLabel::new(dot_sep, x, y)
                    .size(font)
                    .color(palette.muted.with_alpha(0.5))
                    .draw(text, screen.0, screen.1);
                x += 24.0 * s;

                TextLabel::new(dur, x, y)
                    .size(font)
                    .color(palette.text_secondary)
                    .draw(text, screen.0, screen.1);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub fn truncate_with_ellipsis(name: &str, max_w: f32, char_w: f32) -> String {
    let est_w = name.len() as f32 * char_w;
    if est_w <= max_w {
        return name.to_string();
    }
    let ellipsis_w = 3.0 * char_w; // "…"
    let max_chars = ((max_w - ellipsis_w) / char_w).floor().max(1.0) as usize;
    let truncated: String = name.chars().take(max_chars).collect();
    format!("{truncated}\u{2026}")
}

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

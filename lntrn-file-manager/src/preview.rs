//! Right-side preview pane shown in List/Tree views.
//!
//! Renders a thumbnail (icon) + name + type + size + modified/created dates
//! for the currently-selected file. The toggle button lives in the nav bar
//! (left of the Sort button), and a draggable handle on the pane's left edge
//! lets the user resize it.

use std::time::SystemTime;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::fs::FileEntry;

/// The entry the preview pane should describe. Returns the last-selected file
/// (so the user sees the one they just clicked when multi-selecting), or
/// `None` if nothing is selected.
pub fn preview_entry(entries: &[FileEntry]) -> Option<&FileEntry> {
    entries.iter().rev().find(|e| e.selected)
}

/// Draw the preview pane background, divider, and metadata text. The actual
/// thumbnail texture is rendered by the caller (render.rs) which has access
/// to the icon cache — this function only reserves space for it and returns
/// the rect that should receive the texture.
#[allow(clippy::too_many_arguments)]
pub fn draw_preview_pane(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    file_info: &mut crate::file_info::FileInfoCache,
    rect: Rect,
    entry: Option<&FileEntry>,
    handle_rect: Rect,
    handle_hovered: bool,
    handle_dragging: bool,
    screen: (u32, u32),
    s: f32,
) -> Option<Rect> {
    // Background
    painter.rect_filled(rect, 0.0, palette.surface);
    // Subtle left border separating from content
    painter.rect_filled(
        Rect::new(rect.x, rect.y, 1.0 * s, rect.h),
        0.0,
        Color::WHITE.with_alpha(0.08),
    );

    // Drag handle highlight
    if handle_hovered || handle_dragging {
        let alpha = if handle_dragging { 0.35 } else { 0.18 };
        painter.rect_filled(handle_rect, 1.5 * s, palette.accent.with_alpha(alpha));
    }

    let Some(entry) = entry else {
        // Empty state — centered hint text.
        let hint = "No file selected";
        let font = 18.0 * s;
        let hint_w = hint.len() as f32 * font * 0.5;
        let tx = rect.x + (rect.w - hint_w) * 0.5;
        let ty = rect.y + rect.h * 0.5 - font * 0.5;
        TextLabel::new(hint, tx, ty)
            .size(FontSize::Custom(font))
            .color(palette.muted)
            .max_width(rect.w - 32.0 * s)
            .draw(text, screen.0, screen.1);
        return None;
    };

    let pad = 16.0 * s;
    let inner_x = rect.x + pad;
    let inner_w = rect.w - pad * 2.0;
    let mut cy = rect.y + pad;

    // Thumbnail box — square, fills the pane width minus padding. Capped
    // so it doesn't push the metadata rows off the bottom of the pane on
    // short windows.
    let metadata_reserve = 220.0 * s; // name + 4 rows of metadata + separator + padding
    let max_thumb_h = (rect.y + rect.h - cy - metadata_reserve).max(120.0 * s);
    let thumb_sz = inner_w.min(max_thumb_h);
    let thumb_x = rect.x + (rect.w - thumb_sz) * 0.5;
    let thumb_rect = Rect::new(thumb_x, cy, thumb_sz, thumb_sz);
    // Fallback background card behind the texture in case nothing loads.
    painter.rect_filled(thumb_rect, 8.0 * s, palette.bg.with_alpha(0.6));
    painter.rect_stroke(thumb_rect, 8.0 * s, 1.0 * s, palette.muted.with_alpha(0.15));
    cy += thumb_sz + 16.0 * s;

    // Filename — wraps to up to two lines.
    let name_font = 22.0 * s;
    let char_w = name_font * 0.52;
    let lines = wrap_two_lines(&entry.name, inner_w, char_w);
    for line in &lines {
        TextLabel::new(line, inner_x, cy)
            .size(FontSize::Custom(name_font))
            .color(palette.text)
            .max_width(inner_w)
            .draw(text, screen.0, screen.1);
        cy += name_font * 1.25;
    }
    cy += 4.0 * s;

    // Separator
    painter.rect_filled(
        Rect::new(inner_x, cy, inner_w, 1.0 * s),
        0.0,
        palette.muted.with_alpha(0.18),
    );
    cy += 10.0 * s;

    // Metadata rows
    let info = file_info.get(&entry.path);
    let kind = if entry.is_dir {
        "Folder".to_string()
    } else {
        info.type_name.clone()
    };
    let size = if entry.is_dir {
        "—".to_string()
    } else {
        format_bytes(entry.size)
    };
    let modified = format_date(entry.modified);
    let created = std::fs::metadata(&entry.path)
        .ok()
        .and_then(|m| m.created().ok())
        .map(|t| format_date(Some(t)))
        .unwrap_or_else(|| "—".into());

    let row_label_w = 88.0 * s;
    let row_font = 16.0 * s;
    let row_h = row_font + 8.0 * s;
    let row = |label: &str, value: &str, painter: &mut Painter, text: &mut TextRenderer, cy: &mut f32| {
        TextLabel::new(label, inner_x, *cy)
            .size(FontSize::Custom(row_font))
            .color(palette.text_secondary)
            .max_width(row_label_w)
            .draw(text, screen.0, screen.1);
        TextLabel::new(value, inner_x + row_label_w, *cy)
            .size(FontSize::Custom(row_font))
            .color(palette.text)
            .max_width(inner_w - row_label_w)
            .draw(text, screen.0, screen.1);
        *cy += row_h;
        let _ = painter; // silence unused
    };
    row("Kind", &kind, painter, text, &mut cy);
    row("Size", &size, painter, text, &mut cy);
    row("Modified", &modified, painter, text, &mut cy);
    row("Created", &created, painter, text, &mut cy);

    // Optional media rows
    if let Some((w, h)) = info.dimensions {
        let dim = format!("{} × {}", w, h);
        row("Dimensions", &dim, painter, text, &mut cy);
    }
    if let Some(ref dur) = info.duration.clone() {
        row("Duration", dur, painter, text, &mut cy);
    }

    Some(thumb_rect)
}

fn wrap_two_lines(name: &str, max_w: f32, char_w: f32) -> Vec<String> {
    let max_chars = (max_w / char_w).floor().max(1.0) as usize;
    if name.chars().count() <= max_chars {
        return vec![name.to_string()];
    }
    let chars: Vec<char> = name.chars().collect();
    let first: String = chars[..max_chars.min(chars.len())].iter().collect();
    let remaining = &chars[max_chars.min(chars.len())..];
    if remaining.is_empty() {
        return vec![first];
    }
    // Second line gets ellipsis if it would itself overflow.
    if remaining.len() <= max_chars {
        let second: String = remaining.iter().collect();
        vec![first, second]
    } else {
        let visible = max_chars.saturating_sub(1);
        let second: String = remaining[..visible.min(remaining.len())].iter().collect();
        vec![first, format!("{second}\u{2026}")]
    }
}

fn format_bytes(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = size as f64;
    if f >= GB { format!("{:.2} GB", f / GB) }
    else if f >= MB { format!("{:.1} MB", f / MB) }
    else if f >= KB { format!("{:.0} KB", f / KB) }
    else { format!("{} B", size) }
}

fn format_date(modified: Option<SystemTime>) -> String {
    let Some(t) = modified else { return "—".into() };
    let Ok(dur) = t.duration_since(SystemTime::UNIX_EPOCH) else { return "—".into() };
    let secs = dur.as_secs();
    let days = secs / 86400;
    let tod = secs % 86400;
    let hh = (tod / 3600) as u32;
    let mm = ((tod % 3600) / 60) as u32;
    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let yd = if leap { 366 } else { 365 };
        if remaining < yd { break; }
        remaining -= yd;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let md = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mn = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut m = 0usize;
    while m < 12 && remaining >= md[m] {
        remaining -= md[m];
        m += 1;
    }
    let h12 = if hh == 0 { 12 } else if hh > 12 { hh - 12 } else { hh };
    let ampm = if hh < 12 { "AM" } else { "PM" };
    format!("{} {}, {} · {:02}:{:02} {}", mn[m], remaining + 1, y, h12, mm, ampm)
}

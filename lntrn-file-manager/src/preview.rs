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
    // Background intentionally not painted — the window bg already covers
    // this region; stacking surface here would compound the alpha. The 1px
    // left border below still gives the eye a clean visual separator.
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
    let lines = wrap_two_lines(text, &entry.name, inner_w, name_font, 2.0 * s);
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
    let info = file_info
        .get(&entry.path, (entry.size, entry.modified))
        .clone();
    let kind = match (entry.is_symlink, entry.is_dir) {
        (false, true) => "Folder".to_string(),
        (false, false) => info.type_name.clone(),
        (true, true) => "Link to folder".to_string(),
        (true, false) => format!("Link to {}", info.type_name),
    };
    let size = if entry.is_dir {
        "—".to_string()
    } else {
        format_bytes(entry.size)
    };
    let modified = format_date(entry.modified);
    // Creation time comes from the background probe — a stat per frame on
    // an MTP path used to be one of the render-thread stalls.
    let created = match info.created {
        Some(t) => format_date(Some(t)),
        None if info.probing => "\u{2026}".to_string(),
        None => "—".into(),
    };

    let row_label_w = 88.0 * s;
    let row_font = 16.0 * s;
    let row_h = row_font + 8.0 * s;
    let row =
        |label: &str, value: &str, painter: &mut Painter, text: &mut TextRenderer, cy: &mut f32| {
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

/// Split a name over at most two lines by MEASURED width (an estimate of
/// 0.52 em per character lost characters at the wrap point on wide names).
/// `slack` keeps each line a hair under `max_w`: text queued with exactly its
/// own measured width as the limit clips its last glyph.
fn wrap_two_lines(
    text: &mut TextRenderer,
    name: &str,
    max_w: f32,
    font: f32,
    slack: f32,
) -> Vec<String> {
    let fit_w = (max_w - slack).max(1.0);
    if text.measure_width(name, font) <= fit_w {
        return vec![name.to_string()];
    }
    let chars: Vec<char> = name.chars().collect();
    // Longest prefix that fits on the first line (at least one char).
    let (mut lo, mut hi) = (1usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect();
        if text.measure_width(&candidate, font) <= fit_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let first: String = chars[..lo].iter().collect();
    let rest: String = chars[lo..].iter().collect();
    if rest.is_empty() {
        return vec![first];
    }
    // Second line gets an ellipsis if it would itself overflow.
    let second = crate::sections::truncate_to_width(text, &rest, fit_w, font);
    vec![first, second]
}

fn format_bytes(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = size as f64;
    if f >= GB {
        format!("{:.2} GB", f / GB)
    } else if f >= MB {
        format!("{:.1} MB", f / MB)
    } else if f >= KB {
        format!("{:.0} KB", f / KB)
    } else {
        format!("{} B", size)
    }
}

fn format_date(modified: Option<SystemTime>) -> String {
    let Some(t) = modified.and_then(crate::datetime::local) else {
        return "—".into();
    };
    let (h12, ampm) = t.hour12();
    format!(
        "{} {}, {} · {:02}:{:02} {}",
        t.month_name(),
        t.day,
        t.year,
        h12,
        t.minute,
        ampm
    )
}

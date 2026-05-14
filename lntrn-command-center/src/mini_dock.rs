//! Mini-dock of pinned apps that floats just under the panel while it
//! is collapsed (or animating into collapse). Click an icon to launch.
//!
//! Lives outside the panel rect so the panel can stay tiny while these
//! shortcuts remain a single click away.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::toplevel::ToplevelInfo;

/// Icon side length (logical px).
pub const ICON_SIZE: f32 = 56.0;
/// Gap between icons (logical px).
pub const ICON_GAP: f32 = 20.0;
/// Vertical gap from the panel's bottom edge to the dock.
pub const TOP_GAP: f32 = 16.0;
/// Plate corner radius and padding around the icons.
pub const PLATE_RADIUS: f32 = 18.0;
pub const PLATE_PAD: f32 = 10.0;

const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);
const PLATE_ALPHA: f32 = 0.85;
const PLATE_BORDER_ALPHA: f32 = 0.08;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

/// Compute the plate rect (rounded background behind the icons) for
/// the given pin count. Returns None when there are no pins.
pub fn plate_rect(panel: Rect, scale: f32, count: usize) -> Option<Rect> {
    if count == 0 {
        return None;
    }
    let icon = ICON_SIZE * scale;
    let gap = ICON_GAP * scale;
    let pad = PLATE_PAD * scale;
    let top_gap = TOP_GAP * scale;
    let icons_w = count as f32 * icon + (count as f32 - 1.0) * gap;
    let plate_w = icons_w + pad * 2.0;
    let plate_h = icon + pad * 2.0;
    let center_x = panel.x + panel.w / 2.0;
    let x = center_x - plate_w / 2.0;
    let y = panel.y + panel.h + top_gap;
    Some(Rect::new(x, y, plate_w, plate_h))
}

/// Rect of the i-th icon (in physical px). `count` should be the total
/// number of icons in the dock so the layout matches `plate_rect`.
pub fn icon_rect(panel: Rect, scale: f32, count: usize, idx: usize) -> Option<Rect> {
    if idx >= count {
        return None;
    }
    let plate = plate_rect(panel, scale, count)?;
    let icon = ICON_SIZE * scale;
    let gap = ICON_GAP * scale;
    let pad = PLATE_PAD * scale;
    let x = plate.x + pad + idx as f32 * (icon + gap);
    let y = plate.y + pad;
    Some(Rect::new(x, y, icon, icon))
}

/// Hit-test the dock. Returns the pin index under (px, py) if any.
pub fn hit_test(panel: Rect, scale: f32, count: usize, px: f32, py: f32) -> Option<usize> {
    for i in 0..count {
        let r = icon_rect(panel, scale, count, i)?;
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(i);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    pinned: &[&DesktopEntry],
    toplevels: &[ToplevelInfo],
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered_idx: Option<usize>,
    _apps: &AppsProvider,
) {
    let Some(plate) = plate_rect(panel, scale, pinned.len()) else { return };
    let radius = PLATE_RADIUS * scale;

    // Plate background + faint border.
    painter.rect_filled(
        plate,
        radius,
        Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2)
            .with_alpha(PLATE_ALPHA * alpha),
    );
    painter.rect_stroke_sdf(
        plate,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(PLATE_BORDER_ALPHA * alpha),
    );

    // Icons.
    for (i, entry) in pinned.iter().enumerate() {
        let Some(r) = icon_rect(panel, scale, pinned.len(), i) else { continue };

        // Hover ring around the icon (accent gold).
        if hovered_idx == Some(i) {
            painter.rect_stroke_sdf(
                Rect::new(r.x - 4.0 * scale, r.y - 4.0 * scale, r.w + 8.0 * scale, r.h + 8.0 * scale),
                (r.w * 0.4) + 4.0 * scale,
                2.0 * scale,
                Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.65 * alpha),
            );
        }

        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: r.x,
            y: r.y,
            size: r.w,
            opacity: alpha,
            clip: None,
        });

        // Open-window indicator — a small accent pill just under the
        // icon when the pinned app has at least one running window.
        let has_window = toplevels.iter().any(|t| t.app_id == entry.app_id);
        if has_window {
            let indicator_w = (r.w * 0.66).max(20.0 * scale);
            let indicator_h = 3.0 * scale;
            let indicator_x = r.x + (r.w - indicator_w) / 2.0;
            let indicator_y = r.y + r.h + 4.0 * scale;
            painter.rect_filled(
                Rect::new(indicator_x, indicator_y, indicator_w, indicator_h),
                indicator_h * 0.5,
                Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2)
                    .with_alpha(0.95 * alpha),
            );
        }
    }
}

// ── Hover preview ───────────────────────────────────────────────────────────

/// Preview-tile geometry (logical px). Sized so the thumbnail area
/// (tile minus the top toolbar strip) lands at a 16:9 aspect:
/// `H = W * 9/16 + TOOLBAR_H` → 260 × (146 + 32) ≈ 16:9 inside.
pub const PREVIEW_TILE_W: f32 = 260.0;
pub const PREVIEW_TILE_H: f32 = 178.0;
pub const PREVIEW_TILE_GAP_TOP: f32 = 10.0;
pub const PREVIEW_TILE_RADIUS: f32 = 14.0;
/// Deprecated — compositor renders the close button on top of the
/// thumbnail so we don't need to reserve a top strip any more.
pub const PREVIEW_TOOLBAR_H: f32 = 0.0;
const PREVIEW_BG_RGB: (u8, u8, u8) = (28, 28, 28);
const PREVIEW_BG_ALPHA: f32 = 0.95;
const PREVIEW_BORDER_ALPHA: f32 = 0.12;
const ACCENT_RGB_PREVIEW: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
/// X close button geometry inside the preview tile (logical).
pub const PREVIEW_CLOSE_SIZE: f32 = 22.0;
pub const PREVIEW_CLOSE_INSET: f32 = 6.0;

/// Rect of the hover-preview tile that pops up under icon `idx` for a
/// single-window app. Returns `None` when `idx` is out of range.
#[allow(dead_code)] // kept for callers that don't have window-count info
pub fn preview_tile_rect(
    panel: Rect,
    scale: f32,
    count: usize,
    idx: usize,
) -> Option<Rect> {
    preview_tile_rects(panel, scale, count, idx, 1).into_iter().next()
}

/// Rects for the per-window preview tiles arrayed horizontally below
/// the icon at `idx`. Empty when `idx` is out of range or
/// `num_windows == 0`.
pub fn preview_tile_rects(
    panel: Rect,
    scale: f32,
    count: usize,
    idx: usize,
    num_windows: usize,
) -> Vec<Rect> {
    if num_windows == 0 {
        return Vec::new();
    }
    let Some(icon) = icon_rect(panel, scale, count, idx) else { return Vec::new() };
    let Some(plate) = plate_rect(panel, scale, count) else { return Vec::new() };
    let tile_w = PREVIEW_TILE_W * scale;
    let tile_h = PREVIEW_TILE_H * scale;
    let inter_gap = 12.0 * scale;
    let n = num_windows as f32;
    let total_w = n * tile_w + (n - 1.0).max(0.0) * inter_gap;
    let cx = icon.x + icon.w / 2.0;
    let mut x = cx - total_w / 2.0;
    let surface_left = panel.x - 240.0 * scale;
    let surface_right = panel.x + panel.w + 240.0 * scale;
    if x < surface_left {
        x = surface_left;
    }
    if x + total_w > surface_right {
        x = surface_right - total_w;
    }
    let y = plate.y + plate.h + PREVIEW_TILE_GAP_TOP * scale;
    (0..num_windows)
        .map(|i| Rect::new(x + i as f32 * (tile_w + inter_gap), y, tile_w, tile_h))
        .collect()
}

/// X close button rect within a preview tile.
pub fn preview_close_button_rect(tile: Rect, scale: f32) -> Rect {
    let size = PREVIEW_CLOSE_SIZE * scale;
    let inset = PREVIEW_CLOSE_INSET * scale;
    Rect::new(tile.x + tile.w - size - inset, tile.y + inset, size, size)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewHit {
    Body,
    Close,
}

/// Generous hit-zone covering the icon + gap + every preview tile.
/// Used by the hover-sticky check so the cursor can travel from the
/// icon down into any one of the per-window thumbnails without the
/// preview disappearing.
pub fn hit_test_preview_zone(
    panel: Rect,
    scale: f32,
    count: usize,
    idx: usize,
    num_windows: usize,
    px: f32,
    py: f32,
) -> bool {
    let rects = preview_tile_rects(panel, scale, count, idx, num_windows);
    if rects.is_empty() {
        return false;
    }
    let Some(icon) = icon_rect(panel, scale, count, idx) else { return false };
    let mut x_min = icon.x;
    let mut x_max = icon.x + icon.w;
    let mut y_max = icon.y + icon.h;
    for r in &rects {
        x_min = x_min.min(r.x);
        x_max = x_max.max(r.x + r.w);
        y_max = y_max.max(r.y + r.h);
    }
    let y_min = icon.y;
    px >= x_min && px <= x_max && py >= y_min && py <= y_max
}

/// Hit-test a cursor against the per-window preview tiles. Returns
/// `(window_idx, hit)` where `window_idx` selects which window in the
/// hovered app's list was clicked.
pub fn hit_test_preview(
    panel: Rect,
    scale: f32,
    count: usize,
    idx: usize,
    num_windows: usize,
    px: f32,
    py: f32,
) -> Option<(usize, PreviewHit)> {
    let rects = preview_tile_rects(panel, scale, count, idx, num_windows);
    for (i, tile) in rects.iter().enumerate() {
        let close = preview_close_button_rect(*tile, scale);
        if px >= close.x && px <= close.x + close.w
            && py >= close.y && py <= close.y + close.h
        {
            return Some((i, PreviewHit::Close));
        }
        if px >= tile.x && px <= tile.x + tile.w
            && py >= tile.y && py <= tile.y + tile.h
        {
            return Some((i, PreviewHit::Body));
        }
    }
    None
}

/// Draw the hover preview tile. The live thumbnail (when one exists) is
/// painted by the compositor over the lower portion via the thumb IPC;
/// here we just lay down the plate, the icon fallback, the X, the badge,
/// and the window-title label.
/// Draw one tile per window. Each tile gets its own plate +
/// activation ring + title. The live thumbnail + close button are
/// rendered by the compositor on top.
#[allow(clippy::too_many_arguments)]
pub fn draw_preview(
    painter: &mut Painter,
    text: &mut TextRenderer,
    _icons: &mut Vec<IconRequest>,
    entry: &DesktopEntry,
    windows: &[&ToplevelInfo],
    tiles: &[Rect],
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let radius = PREVIEW_TILE_RADIUS * scale;
    for (i, tile) in tiles.iter().enumerate() {
        let window = match windows.get(i) {
            Some(w) => *w,
            None => continue,
        };
        // Plate.
        painter.rect_filled(
            *tile,
            radius,
            Color::from_rgb8(PREVIEW_BG_RGB.0, PREVIEW_BG_RGB.1, PREVIEW_BG_RGB.2)
                .with_alpha(PREVIEW_BG_ALPHA * alpha),
        );
        painter.rect_stroke_sdf(
            *tile,
            radius,
            1.0 * scale,
            Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(PREVIEW_BORDER_ALPHA * alpha),
        );
        // Highlight ring on the currently-activated window.
        if window.activated {
            painter.rect_stroke_sdf(
                *tile,
                radius,
                2.0 * scale,
                Color::from_rgb8(ACCENT_RGB_PREVIEW.0, ACCENT_RGB_PREVIEW.1, ACCENT_RGB_PREVIEW.2)
                    .with_alpha(0.70 * alpha),
            );
        }

        // Title strip across the bottom — readable window title.
        let title = if window.title.is_empty() {
            entry.name.clone()
        } else {
            window.title.clone()
        };
        let label = truncate(&title, 32);
        let font = 14.0 * scale;
        let lw = text.measure_width(&label, font);
        text.queue(
            &label,
            font,
            tile.x + (tile.w - lw) / 2.0,
            tile.y + tile.h - font - 6.0 * scale,
            Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.92 * alpha),
            lw,
            surface_w,
            surface_h,
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

/// Pick the window in `windows` whose live thumbnail should fill the
/// hover preview — prefer the activated one, else the first.
pub fn preview_target_window<'a>(windows: &[&'a ToplevelInfo]) -> Option<&'a ToplevelInfo> {
    windows.iter().find(|w| w.activated).copied().or_else(|| windows.first().copied())
}

/// All windows whose `app_id` matches the given pinned app's id.
pub fn windows_for_app<'a>(toplevels: &'a [ToplevelInfo], app_id: &str) -> Vec<&'a ToplevelInfo> {
    toplevels.iter().filter(|t| t.app_id == app_id).collect()
}

//! Hover preview for the mini-dock — one tile per window of the hovered
//! app, arrayed above its icon. The compositor paints the live
//! thumbnails and close buttons on top (thumb IPC); this module owns the
//! tile geometry, hit-testing, and the plate/title underneath.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{DockEntry, DockLayout};
use crate::render::IconRequest;
use crate::toplevel::ToplevelInfo;

/// Preview-tile geometry (logical px). Sized roughly 16:9 so live
/// thumbnails fit comfortably; bumped up from 260×178 now that the
/// dock sits at the bottom of the screen with plenty of room above.
pub const PREVIEW_TILE_W: f32 = 320.0;
pub const PREVIEW_TILE_H: f32 = 220.0;
/// Vertical gap between the dock plate's top edge and the preview
/// tiles' bottom edge (tiles float above the icons).
pub const PREVIEW_TILE_GAP: f32 = 16.0;
pub const PREVIEW_TILE_RADIUS: f32 = 14.0;
const PREVIEW_BG_RGB: (u8, u8, u8) = (28, 28, 28);
const PREVIEW_BG_ALPHA: f32 = 0.95;
const PREVIEW_BORDER_ALPHA: f32 = 0.12;
const ACCENT_RGB_PREVIEW: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
/// X close button geometry inside the preview tile (logical).
pub const PREVIEW_CLOSE_SIZE: f32 = 22.0;
pub const PREVIEW_CLOSE_INSET: f32 = 6.0;

/// Rects for the per-window preview tiles arrayed horizontally above
/// the icon at `idx`. Empty when `idx` is out of range or
/// `num_windows == 0`. Tiles are clamped to roughly the panel's
/// horizontal bounds so they don't drift off-screen for icons near the
/// edges of a wide dock.
pub fn preview_tile_rects(
    layout: &DockLayout,
    panel: Rect,
    idx: usize,
    num_windows: usize,
) -> Vec<Rect> {
    if num_windows == 0 || idx >= layout.icons.len() {
        return Vec::new();
    }
    let icon = layout.icons[idx];
    let scale = layout.scale;
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
    // Tiles sit ABOVE the dock plate: bottom edge of the tile is
    // `PREVIEW_TILE_GAP` above the plate's top edge.
    let y = layout.plate.y - PREVIEW_TILE_GAP * scale - tile_h;
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
/// icon up into any one of the per-window thumbnails without the
/// preview disappearing.
pub fn hit_test_preview_zone(
    layout: &DockLayout,
    panel: Rect,
    idx: usize,
    num_windows: usize,
    px: f32,
    py: f32,
) -> bool {
    let rects = preview_tile_rects(layout, panel, idx, num_windows);
    if rects.is_empty() || idx >= layout.icons.len() {
        return false;
    }
    let icon = layout.icons[idx];
    let mut x_min = icon.x;
    let mut x_max = icon.x + icon.w;
    let mut y_min = icon.y;
    for r in &rects {
        x_min = x_min.min(r.x);
        x_max = x_max.max(r.x + r.w);
        y_min = y_min.min(r.y);
    }
    let y_max = icon.y + icon.h;
    px >= x_min && px <= x_max && py >= y_min && py <= y_max
}

/// Hit-test a cursor against the per-window preview tiles. Returns
/// `(window_idx, hit)` where `window_idx` selects which window in the
/// hovered app's list was clicked.
pub fn hit_test_preview(
    layout: &DockLayout,
    panel: Rect,
    idx: usize,
    num_windows: usize,
    px: f32,
    py: f32,
) -> Option<(usize, PreviewHit)> {
    let rects = preview_tile_rects(layout, panel, idx, num_windows);
    let scale = layout.scale;
    for (i, tile) in rects.iter().enumerate() {
        let close = preview_close_button_rect(*tile, scale);
        if px >= close.x && px <= close.x + close.w && py >= close.y && py <= close.y + close.h {
            return Some((i, PreviewHit::Close));
        }
        if px >= tile.x && px <= tile.x + tile.w && py >= tile.y && py <= tile.y + tile.h {
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
    entry: &DockEntry,
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
                Color::from_rgb8(
                    ACCENT_RGB_PREVIEW.0,
                    ACCENT_RGB_PREVIEW.1,
                    ACCENT_RGB_PREVIEW.2,
                )
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
            // Half-em slack: an exact measured-width bound can wrap the
            // last glyph onto a clipped second line.
            lw + font * 0.5,
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

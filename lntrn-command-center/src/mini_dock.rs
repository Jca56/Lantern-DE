//! Mini-dock of pinned apps that floats above the screen's bottom edge
//! while the Command Center is collapsed (or animating into collapse).
//! Click an icon to launch.
//!
//! Shows pinned apps on the left; running-but-unpinned apps fill in to
//! the right in FIFO order (first window opened = leftmost), separated
//! by a thin divider. Apps with at least one open window get a small
//! horizontal accent line above the icon as a "running" indicator.
//!
//! Hovering near the dock magnifies icons macOS-style: the icon under
//! the cursor scales to ~1.4×, immediate neighbors to ~1.25×, and the
//! falloff continues smoothly. The plate widens to accommodate.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::toplevel::ToplevelInfo;

/// Icon side length (logical px).
pub const ICON_SIZE: f32 = 56.0;
/// Gap between icons (logical px).
pub const ICON_GAP: f32 = 20.0;
/// Distance from the screen's bottom edge to the plate's bottom edge
/// (logical px). Mirrors `PANEL_TOP_MARGIN_LOGICAL` so the dock floats
/// the same distance from the bottom as the panel does from the top.
pub const BOTTOM_GAP: f32 = 48.0;
/// Plate corner radius and padding around the icons.
pub const PLATE_RADIUS: f32 = 18.0;
pub const PLATE_PAD: f32 = 10.0;
/// Magnification scale for the icon directly under the cursor. Neighbor
/// icons fall off smoothly toward 1.0 with distance.
pub const MAG_PEAK: f32 = 1.6;
/// How fast magnification falls off (in slot-pitch units). Larger →
/// more icons magnify; smaller → tighter wave.
const MAG_SIGMA: f32 = 1.5;

const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);
const PLATE_ALPHA: f32 = 0.85;
const PLATE_BORDER_ALPHA: f32 = 0.08;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const DIVIDER_ALPHA: f32 = 0.18;
/// Running-indicator strip: a thin pill above the icon for any app that
/// has at least one open window.
const INDICATOR_BAR_H: f32 = 3.0;
const INDICATOR_GAP_ABOVE_ICON: f32 = 6.0;

/// One slot in the dock. Either a user-pinned app or a running app
/// that isn't pinned (appears on the right of the divider).
#[derive(Debug, Clone)]
pub struct DockEntry {
    pub app_id: String,
    pub name: String,
    pub icon_name: Option<String>,
    /// True when this slot is a user pin; false when it's auto-added
    /// because the app has at least one open window.
    pub pinned: bool,
}

/// Pre-computed geometry for one dock frame. Cheap to build, used by
/// both rendering and hit-testing so they always agree.
pub struct DockLayout {
    pub entries: Vec<DockEntry>,
    /// Plate background rect (physical px).
    pub plate: Rect,
    /// Drawn icon rect per entry — already magnified, bottom-aligned to
    /// the plate's icon baseline so larger icons grow *upward*.
    pub icons: Vec<Rect>,
    /// Number of pinned entries at the front of `entries`. The divider
    /// sits between index `pinned_count - 1` and `pinned_count` when
    /// both halves are non-empty.
    pub pinned_count: usize,
    /// Scale factor used to build the layout (physical-px per logical-px).
    pub scale: f32,
}

/// Build the merged list of dock entries — pinned first (in user order),
/// then unpinned apps that currently have at least one open window
/// (FIFO by first-seen toplevel).
pub fn dock_entries(
    pinned: &[&DesktopEntry],
    toplevels: &[ToplevelInfo],
    apps: &AppsProvider,
) -> Vec<DockEntry> {
    let mut out: Vec<DockEntry> = pinned
        .iter()
        .map(|e| DockEntry {
            app_id: e.app_id.clone(),
            name: e.name.clone(),
            icon_name: e.icon_name.clone(),
            pinned: true,
        })
        .collect();

    // Insertion-order: iterate toplevels (already in creation order),
    // skip any whose app_id is already represented (pinned or already
    // added as a running entry), append the rest.
    for t in toplevels {
        if t.app_id.is_empty() {
            continue;
        }
        if out.iter().any(|e| e.app_id == t.app_id) {
            continue;
        }
        let (name, icon_name) = lookup_app_meta(apps, &t.app_id);
        out.push(DockEntry {
            app_id: t.app_id.clone(),
            name,
            icon_name,
            pinned: false,
        });
    }
    out
}

fn lookup_app_meta(apps: &AppsProvider, app_id: &str) -> (String, Option<String>) {
    // 1) Exact match on the .desktop stem (the usual case).
    for i in 0..apps.count() {
        if let Some(e) = apps.get(i) {
            if e.app_id.eq_ignore_ascii_case(app_id) {
                return (e.name.clone(), e.icon_name.clone());
            }
        }
    }
    // 2) Fall back to matching the window's Wayland app_id against a
    //    .desktop's `Icon=` name. Some apps set an app_id that differs from
    //    their .desktop filename (e.g. window `lntrn-media-player` vs entry
    //    `org.lantern.MediaPlayer`), but the app_id IS the icon name — so
    //    this still recovers both the proper name and the icon.
    for i in 0..apps.count() {
        if let Some(e) = apps.get(i) {
            if e.icon_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(app_id))
            {
                return (e.name.clone(), e.icon_name.clone());
            }
        }
    }
    // 3) Last resort: use the app_id itself as the icon name, so any icon
    //    file named after the app_id (all our lntrn-* apps) still resolves.
    (app_id.to_string(), Some(app_id.to_string()))
}

/// Smooth Gaussian-shaped magnification curve. `d_slots` is the cursor's
/// distance to the icon's *base* (un-magnified) center measured in
/// slot-pitch units (one slot = ICON_SIZE + ICON_GAP). Returns a scale
/// factor in `[1.0, MAG_PEAK]`.
fn magnification(d_slots: f32) -> f32 {
    let d = d_slots.abs();
    if d > 4.0 {
        return 1.0;
    }
    1.0 + (MAG_PEAK - 1.0) * (-(d * d) / (MAG_SIGMA * MAG_SIGMA)).exp()
}

/// Build the full dock layout for the current frame. Returns `None`
/// when there's nothing to show (no pinned + no running apps).
///
/// Magnification kicks in only when `cursor_phys` is provided **and**
/// the cursor is within the dock's hover zone — the plate's vertical
/// band plus enough headroom above to cover a fully-magnified icon.
/// Outside that zone the dock renders flat (1.0× everywhere).
pub fn compute_layout(
    panel: Rect,
    surface_h: f32,
    scale: f32,
    pinned: &[&DesktopEntry],
    toplevels: &[ToplevelInfo],
    apps: &AppsProvider,
    cursor_phys: Option<(f32, f32)>,
) -> Option<DockLayout> {
    let entries = dock_entries(pinned, toplevels, apps);
    if entries.is_empty() {
        return None;
    }

    let icon = ICON_SIZE * scale;
    let gap = ICON_GAP * scale;
    let pad = PLATE_PAD * scale;
    let bottom_gap = BOTTOM_GAP * scale;
    let pinned_count = entries.iter().take_while(|e| e.pinned).count();
    let n = entries.len();

    // Base plate (no magnification). We need its left edge to map the
    // cursor into slot-distance space; downstream we re-center the
    // *magnified* plate on the panel center.
    let base_icons_w = n as f32 * icon + (n as f32 - 1.0).max(0.0) * gap;
    let base_plate_w = base_icons_w + pad * 2.0;
    let center_x = panel.x + panel.w / 2.0;
    let base_plate_x = center_x - base_plate_w / 2.0;
    let slot_pitch = icon + gap;

    // Base plate vertical extent (un-magnified) — used as the y anchor
    // and as the cursor hover-zone reference.
    let base_plate_h = icon + pad * 2.0;
    let base_plate_y = surface_h - bottom_gap - base_plate_h;

    // Vertical hover zone for magnification. We extend it *upward* by
    // the maximum amount a magnified icon overflows the plate so the
    // cursor can hover over a giant icon and keep it big; we also add
    // a small buffer above the plate so approaching from above feels
    // sticky. Below the plate is irrelevant (it's already at the
    // screen edge).
    let mag_overflow_top = (MAG_PEAK - 1.0) * icon;
    let zone_top = base_plate_y - mag_overflow_top - 8.0 * scale;
    let zone_bottom = base_plate_y + base_plate_h;
    let cursor_in_zone = cursor_phys
        .map(|(_, cy)| cy >= zone_top && cy <= zone_bottom)
        .unwrap_or(false);

    // Per-icon magnification factor based on cursor distance — only when
    // the cursor is within the vertical hover zone. Otherwise the dock
    // renders flat.
    let mags: Vec<f32> = (0..n)
        .map(|i| {
            if !cursor_in_zone {
                return 1.0;
            }
            let base_cx = base_plate_x + pad + i as f32 * slot_pitch + icon / 2.0;
            match cursor_phys {
                Some((cx, _)) => magnification((cx - base_cx) / slot_pitch),
                None => 1.0,
            }
        })
        .collect();

    // Magnified plate: width grows to fit larger icons.
    let mag_icons_w: f32 =
        mags.iter().map(|m| icon * m).sum::<f32>() + (n as f32 - 1.0).max(0.0) * gap;
    let plate_w = mag_icons_w + pad * 2.0;
    let plate_h = icon + pad * 2.0;
    let plate_x = center_x - plate_w / 2.0;
    let plate_y = surface_h - bottom_gap - plate_h;
    let plate = Rect::new(plate_x, plate_y, plate_w, plate_h);

    // Bottom-align icons inside the plate so larger ones grow upward.
    let baseline_y = plate.y + plate.h - pad;
    let mut icons_out: Vec<Rect> = Vec::with_capacity(n);
    let mut x_cursor = plate.x + pad;
    for (i, m) in mags.iter().enumerate() {
        let w = icon * m;
        let h = w; // square icons
        let y = baseline_y - h;
        icons_out.push(Rect::new(x_cursor, y, w, h));
        x_cursor += w;
        if i + 1 < n {
            x_cursor += gap;
        }
    }

    Some(DockLayout {
        entries,
        plate,
        icons: icons_out,
        pinned_count,
        scale,
    })
}

/// Hit-test the dock. Returns the entry index under `(px, py)` if any.
pub fn hit_test(layout: &DockLayout, px: f32, py: f32) -> Option<usize> {
    for (i, r) in layout.icons.iter().enumerate() {
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(i);
        }
    }
    None
}

pub fn draw(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    layout: &DockLayout,
    toplevels: &[ToplevelInfo],
    alpha: f32,
) {
    let scale = layout.scale;
    let plate = layout.plate;
    let radius = PLATE_RADIUS * scale;

    // Plate background + faint border.
    painter.rect_filled(
        plate,
        radius,
        Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(PLATE_ALPHA * alpha),
    );
    painter.rect_stroke_sdf(
        plate,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(PLATE_BORDER_ALPHA * alpha),
    );

    // Divider between pinned section and unpinned-running section.
    if layout.pinned_count > 0
        && layout.pinned_count < layout.entries.len()
        && layout.pinned_count <= layout.icons.len()
    {
        let left_icon = layout.icons[layout.pinned_count - 1];
        let right_icon = layout.icons[layout.pinned_count];
        let div_x = (left_icon.x + left_icon.w + right_icon.x) / 2.0;
        let div_w = (1.5 * scale).max(1.0);
        let inset = 4.0 * scale;
        let div = Rect::new(
            div_x - div_w / 2.0,
            plate.y + inset,
            div_w,
            plate.h - inset * 2.0,
        );
        painter.rect_filled(
            div,
            div_w * 0.5,
            Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(DIVIDER_ALPHA * alpha),
        );
    }

    // Icons.
    for (i, entry) in layout.entries.iter().enumerate() {
        let r = layout.icons[i];

        // No hover highlight here — the magnification grow *is* the hover
        // affordance (see MAG_PEAK).

        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: r.x,
            y: r.y,
            size: r.w,
            opacity: alpha,
            clip: None,
        });

        // Running-window indicator: a thin pill *above* the icon for any
        // app with at least one open window. Sits a small gap above the
        // (possibly magnified) icon top edge so it tracks the wave.
        let has_window = toplevels.iter().any(|t| t.app_id == entry.app_id);
        if has_window {
            let indicator_w = r.w;
            let indicator_h = INDICATOR_BAR_H * scale;
            let indicator_x = r.x + (r.w - indicator_w) / 2.0;
            let indicator_y = r.y - INDICATOR_GAP_ABOVE_ICON * scale - indicator_h;
            painter.rect_filled(
                Rect::new(indicator_x, indicator_y, indicator_w, indicator_h),
                indicator_h * 0.5,
                Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.95 * alpha),
            );
        }
    }
}

// ── Hover preview ───────────────────────────────────────────────────────────

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

/// All windows whose `app_id` matches the given pinned app's id.
pub fn windows_for_app<'a>(toplevels: &'a [ToplevelInfo], app_id: &str) -> Vec<&'a ToplevelInfo> {
    toplevels.iter().filter(|t| t.app_id == app_id).collect()
}

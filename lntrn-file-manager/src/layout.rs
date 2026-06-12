use lntrn_render::Rect;
use std::sync::atomic::{AtomicBool, Ordering};

/// When true, layout omits the title bar (desktop widget mode).
pub static DESKTOP_MODE: AtomicBool = AtomicBool::new(false);

/// "Rice mode" — Super+F11 hides/shows the title bar at runtime (same toggle
/// the terminal has). Everything below shifts up since all layout derives
/// from `title_bar_h_base`.
pub static CHROME_HIDDEN: AtomicBool = AtomicBool::new(false);

pub fn chrome_hidden() -> bool {
    CHROME_HIDDEN.load(Ordering::Relaxed)
}

fn title_bar_h_base() -> f32 {
    if DESKTOP_MODE.load(Ordering::Relaxed) || CHROME_HIDDEN.load(Ordering::Relaxed) {
        0.0
    } else {
        40.0
    }
}

/// The top gradient strip (between title bar and nav bar) goes away in rice
/// mode along with the title bar. Desktop mode keeps it.
fn top_gradient_h_base() -> f32 {
    if CHROME_HIDDEN.load(Ordering::Relaxed) { 0.0 } else { GRADIENT_H }
}
const NAV_BAR_H: f32 = 48.0;
const GRADIENT_H: f32 = 4.0;
const TAB_BAR_H: f32 = 46.0;
const SIDEBAR_W: f32 = 240.0;
const STATUS_BAR_H: f32 = 34.0;
const ITEM_SIZE: f32 = 80.0;
const ICON_SIZE: f32 = 48.0;
const ITEM_PAD: f32 = 8.0;
const LIST_ROW_H: f32 = 40.0;
const TREE_ROW_H: f32 = 36.0;
#[allow(dead_code)]
const TREE_INDENT: f32 = 24.0;

/// Zoom 0.0 → 1.8x, 0.5 → 2.9x, 1.0 → 4.0x
/// Floor is already a comfortable size; top end is huge.
pub fn zoom_multiplier(zoom: f32) -> f32 {
    1.8 + zoom * 2.2
}

/// Gentler zoom multiplier for List/Tree rows — they're inherently dense,
/// so we don't want them to balloon like grid items.
/// 0.0 → 0.8x, 0.5 → 1.5x (default), 1.0 → 2.2x.
pub fn list_zoom_multiplier(zoom: f32) -> f32 {
    0.8 + zoom * 1.4
}

/// Scaled layout helper. All public functions return physical-pixel values.
#[allow(dead_code)]
pub fn title_bar_h(s: f32) -> f32 { title_bar_h_base() * s }
#[allow(dead_code)]
pub fn gradient_h(s: f32) -> f32 { GRADIENT_H * s }
pub fn sidebar_w(s: f32) -> f32 { SIDEBAR_W * s }
pub fn item_size(s: f32, zoom: f32) -> f32 { (ITEM_SIZE * zoom_multiplier(zoom)).max(60.0) * s }
pub fn icon_size(s: f32, zoom: f32) -> f32 { ICON_SIZE * s * zoom_multiplier(zoom) }
#[allow(dead_code)]
pub fn item_pad(s: f32) -> f32 { ITEM_PAD * s }

pub fn title_bar_rect(width: f32, s: f32) -> Rect {
    Rect::new(0.0, 0.0, width, title_bar_h_base() * s)
}

pub fn nav_bar_y(s: f32) -> f32 {
    (title_bar_h_base() + top_gradient_h_base()) * s
}

pub fn nav_bar_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    Rect::new(x, nav_bar_y(s), width - x, NAV_BAR_H * s)
}

pub fn view_toggle_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 6.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn cloud_button_rect(s: f32) -> Rect {
    // Bigger than the nav arrows so it reads as a destination, not a control.
    // Vertically centered against the 36px arrow buttons that sit at y + 6.
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 48.0 * s, y + 2.0 * s, 44.0 * s, 44.0 * s)
}

pub fn back_button_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 100.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn forward_button_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 138.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn up_button_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 176.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn path_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    let path_x = x + 224.0 * s;
    // Reserve space for preview-toggle, sort, and search buttons (each 36px + gap).
    let trailing_space = 130.0 * s;
    Rect::new(path_x, y + 5.0 * s, width - path_x - trailing_space, 38.0 * s)
}

pub fn preview_toggle_rect(width: f32, s: f32) -> Rect {
    let y = nav_bar_y(s);
    Rect::new(width - 126.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn sort_button_rect(width: f32, s: f32) -> Rect {
    let y = nav_bar_y(s);
    Rect::new(width - 84.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn search_button_rect(width: f32, s: f32) -> Rect {
    let y = nav_bar_y(s);
    Rect::new(width - 42.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn tab_bar_y(s: f32) -> f32 {
    (title_bar_h_base() + top_gradient_h_base() + NAV_BAR_H + GRADIENT_H) * s
}

pub fn tab_bar_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    Rect::new(x, tab_bar_y(s), width - x, TAB_BAR_H * s)
}

pub fn content_top(s: f32) -> f32 {
    (title_bar_h_base() + top_gradient_h_base() + NAV_BAR_H + GRADIENT_H + TAB_BAR_H) * s
}

pub fn content_bottom(height: f32, s: f32) -> f32 {
    height - STATUS_BAR_H * s
}

pub fn content_rect_with_bottom(width: f32, bottom: f32, s: f32) -> Rect {
    let top = content_top(s);
    Rect::new(SIDEBAR_W * s, top, width - SIDEBAR_W * s, bottom - top)
}

pub fn sidebar_rect(height: f32, s: f32) -> Rect {
    let top = nav_bar_y(s);
    let bottom = content_bottom(height, s);
    Rect::new(0.0, top, SIDEBAR_W * s, bottom - top)
}

// ── Sidebar layout (dynamic) ────────────────────────────────────────────────
//
// Sections (Places / Favorites / Devices) are collapsible and the Favorites
// list is user-driven, so item rects can't be computed from index alone. We
// walk the layout once per frame and hand out a struct of all hit/draw rects.
// Both the renderer (draw) and the input pass (zone registration) consume the
// same `SidebarLayout`, so they can't drift.

const SIDEBAR_HEADER_H: f32 = 30.0;
const SIDEBAR_HEADER_GAP: f32 = 12.0;     // gap above each section header
const SIDEBAR_PLACE_ITEM_H: f32 = 40.0;
const SIDEBAR_DRIVE_ITEM_H: f32 = 64.0;
const SIDEBAR_PHONE_ITEM_H: f32 = 56.0;

pub struct SidebarLayout {
    pub places_header: Rect,
    pub place_items: Vec<Rect>,
    pub favorites_header: Rect,
    pub favorites_plus: Rect,
    pub favorite_items: Vec<Rect>,
    pub devices_header: Rect,
    pub drive_items: Vec<Rect>,
    pub phone_items: Vec<Rect>,
    /// True if the Devices section was emitted at all (hidden when no
    /// drives + no phones are present).
    pub has_devices: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn build_sidebar_layout(
    s: f32,
    num_places: usize,
    num_favorites: usize,
    num_drives: usize,
    num_phones: usize,
    places_collapsed: bool,
    favorites_collapsed: bool,
    devices_collapsed: bool,
) -> SidebarLayout {
    let item_x = 4.0 * s;
    let item_w = (SIDEBAR_W - 12.0) * s;
    let header_h = SIDEBAR_HEADER_H * s;
    let header_gap = SIDEBAR_HEADER_GAP * s;
    let place_h = SIDEBAR_PLACE_ITEM_H * s;
    let drive_h = SIDEBAR_DRIVE_ITEM_H * s;
    let phone_h = SIDEBAR_PHONE_ITEM_H * s;

    let mut y = nav_bar_y(s) + header_gap;

    // ── PLACES ───────────────────────────────────────────────────────
    let places_header = Rect::new(0.0, y, SIDEBAR_W * s, header_h);
    y += header_h;
    let mut place_items = Vec::with_capacity(num_places);
    if !places_collapsed {
        for _ in 0..num_places {
            place_items.push(Rect::new(item_x, y, item_w, place_h));
            y += place_h;
        }
    }

    // ── FAVORITES ────────────────────────────────────────────────────
    y += header_gap;
    let favorites_header = Rect::new(0.0, y, SIDEBAR_W * s, header_h);
    // Plus button on the right side of the header.
    let plus_sz = 24.0 * s;
    let favorites_plus = Rect::new(
        SIDEBAR_W * s - plus_sz - 12.0 * s,
        y + (header_h - plus_sz) * 0.5,
        plus_sz,
        plus_sz,
    );
    y += header_h;
    let mut favorite_items = Vec::with_capacity(num_favorites);
    if !favorites_collapsed {
        for _ in 0..num_favorites {
            favorite_items.push(Rect::new(item_x, y, item_w, place_h));
            y += place_h;
        }
    }

    // ── DEVICES ──────────────────────────────────────────────────────
    let has_devices = num_drives > 0 || num_phones > 0;
    let devices_header;
    let mut drive_items = Vec::with_capacity(num_drives);
    let mut phone_items = Vec::with_capacity(num_phones);
    if has_devices {
        y += header_gap;
        devices_header = Rect::new(0.0, y, SIDEBAR_W * s, header_h);
        y += header_h;
        if !devices_collapsed {
            for _ in 0..num_drives {
                drive_items.push(Rect::new(item_x, y, item_w, drive_h));
                y += drive_h;
            }
            for _ in 0..num_phones {
                phone_items.push(Rect::new(item_x, y, item_w, phone_h));
                y += phone_h;
            }
        }
    } else {
        devices_header = Rect::new(0.0, 0.0, 0.0, 0.0);
    }

    SidebarLayout {
        places_header,
        place_items,
        favorites_header,
        favorites_plus,
        favorite_items,
        devices_header,
        drive_items,
        phone_items,
        has_devices,
    }
}

pub fn content_rect(width: f32, height: f32, s: f32) -> Rect {
    let top = content_top(s);
    let bottom = content_bottom(height, s);
    Rect::new(SIDEBAR_W * s, top, width - SIDEBAR_W * s, bottom - top)
}

/// Width of the resize handle that sits on the preview pane's left edge.
pub const PREVIEW_HANDLE_W: f32 = 6.0;

/// Min/max bounds for the preview pane width (logical px, pre-scale).
pub const PREVIEW_MIN_W: f32 = 220.0;
pub const PREVIEW_MAX_FRACTION: f32 = 0.6; // never more than 60% of content area

/// Effective preview width in physical px, clamped to bounds for the current
/// content area. Returns 0 if the preview is closed or would not fit.
pub fn preview_effective_w(content_w_px: f32, preview_w_logical: f32, open: bool, s: f32) -> f32 {
    if !open { return 0.0; }
    let min = PREVIEW_MIN_W * s;
    let max = (content_w_px * PREVIEW_MAX_FRACTION).max(min);
    (preview_w_logical * s).clamp(min, max)
}

pub fn preview_pane_rect(content: Rect, preview_w: f32) -> Rect {
    Rect::new(content.x + content.w - preview_w, content.y, preview_w, content.h)
}

pub fn preview_handle_rect(content: Rect, preview_w: f32, s: f32) -> Rect {
    let hw = PREVIEW_HANDLE_W * s;
    Rect::new(content.x + content.w - preview_w - hw * 0.5, content.y, hw, content.h)
}

pub fn status_rect(width: f32, height: f32, s: f32) -> Rect {
    Rect::new(0.0, height - STATUS_BAR_H * s, width, STATUS_BAR_H * s)
}

pub fn list_row_h(s: f32, zoom: f32) -> f32 { LIST_ROW_H * list_zoom_multiplier(zoom) * s }
pub fn search_list_row_h(s: f32, zoom: f32) -> f32 { 56.0 * list_zoom_multiplier(zoom) * s }
pub fn tree_row_h(s: f32, zoom: f32) -> f32 { TREE_ROW_H * list_zoom_multiplier(zoom) * s }
#[allow(dead_code)]
pub fn tree_indent(s: f32, zoom: f32) -> f32 { TREE_INDENT * list_zoom_multiplier(zoom) * s }

pub fn list_content_height(entry_count: usize, s: f32, zoom: f32) -> f32 {
    entry_count as f32 * list_row_h(s, zoom)
}

pub fn tree_content_height(entry_count: usize, s: f32, zoom: f32) -> f32 {
    entry_count as f32 * tree_row_h(s, zoom)
}

#[allow(dead_code)]
pub fn list_row_rect(index: usize, content_x: f32, content_w: f32, base_y: f32, s: f32, zoom: f32) -> Rect {
    let rh = list_row_h(s, zoom);
    let y = base_y + index as f32 * rh;
    Rect::new(content_x, y, content_w, rh)
}

#[allow(dead_code)]
pub fn tree_row_rect(index: usize, depth: usize, content_x: f32, content_w: f32, base_y: f32, s: f32, zoom: f32) -> Rect {
    let rh = tree_row_h(s, zoom);
    let indent = depth as f32 * TREE_INDENT * list_zoom_multiplier(zoom) * s;
    let y = base_y + index as f32 * rh;
    Rect::new(content_x + indent, y, content_w - indent, rh)
}

pub fn grid_columns(content_width: f32, s: f32, zoom: f32) -> usize {
    let item = item_size(s, zoom);
    let pad = ITEM_PAD * s;
    ((content_width - pad) / (item + pad)).max(1.0) as usize
}

pub fn grid_content_height(entry_count: usize, cols: usize, s: f32, zoom: f32) -> f32 {
    let item = item_size(s, zoom);
    let pad = ITEM_PAD * s;
    let rows = (entry_count + cols.saturating_sub(1)) / cols.max(1);
    rows as f32 * (item + pad) + pad
}

pub fn file_item_rect(index: usize, cols: usize, content_x: f32, base_y: f32, s: f32, zoom: f32) -> Rect {
    let item = item_size(s, zoom);
    let pad = ITEM_PAD * s;
    let col = index % cols.max(1);
    let row = index / cols.max(1);
    let x = content_x + pad + col as f32 * (item + pad);
    let y = base_y + pad + row as f32 * (item + pad);
    Rect::new(x, y, item, item)
}

/// Tight hit/highlight rect inside a grid cell: hugs the icon + label block
/// instead of the full cell, so the gaps between items stay right-clickable
/// (empty-area menu) while the grid spacing itself is unchanged. Mirrors the
/// icon/label geometry in `sections/grid.rs`.
pub fn item_hit_rect(cell: Rect, s: f32, zoom: f32) -> Rect {
    let icsz = icon_size(s, zoom);
    let label_font = 16.0 * s;
    let content_h = icsz + 2.0 * s + label_font; // icon + gap + filename line
    let margin = 8.0 * s;
    let w = (icsz + margin * 2.0).min(cell.w);
    let h = (content_h + margin * 2.0).min(cell.h);
    Rect::new(cell.x + (cell.w - w) * 0.5, cell.y + (cell.h - h) * 0.5, w, h)
}

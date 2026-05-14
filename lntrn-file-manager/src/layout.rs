use lntrn_render::Rect;
use std::sync::atomic::{AtomicBool, Ordering};

/// When true, layout omits the title bar (desktop widget mode).
pub static DESKTOP_MODE: AtomicBool = AtomicBool::new(false);

fn title_bar_h_base() -> f32 {
    if DESKTOP_MODE.load(Ordering::Relaxed) { 0.0 } else { 40.0 }
}
const NAV_BAR_H: f32 = 48.0;
const GRADIENT_H: f32 = 4.0;
const TAB_BAR_H: f32 = 46.0;
const SIDEBAR_W: f32 = 200.0;
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
    (title_bar_h_base() + GRADIENT_H) * s
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
    (title_bar_h_base() + GRADIENT_H + NAV_BAR_H + GRADIENT_H) * s
}

pub fn tab_bar_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    Rect::new(x, tab_bar_y(s), width - x, TAB_BAR_H * s)
}

pub fn content_top(s: f32) -> f32 {
    (title_bar_h_base() + GRADIENT_H + NAV_BAR_H + GRADIENT_H + TAB_BAR_H) * s
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

pub fn sidebar_item_rect(index: usize, s: f32) -> Rect {
    let mut y = nav_bar_y(s) + 42.0 * s;
    y += index as f32 * 40.0 * s;
    Rect::new(4.0 * s, y, (SIDEBAR_W - 12.0) * s, 40.0 * s)
}

/// Y position where the drives section starts (after places + header gap).
pub fn drives_section_y(num_places: usize, s: f32) -> f32 {
    nav_bar_y(s) + 42.0 * s + num_places as f32 * 40.0 * s + 20.0 * s
}

pub fn drive_item_rect(index: usize, num_places: usize, s: f32) -> Rect {
    let mut y = drives_section_y(num_places, s) + 30.0 * s; // after "DEVICES" header
    y += index as f32 * 64.0 * s; // taller items for usage bar
    Rect::new(4.0 * s, y, (SIDEBAR_W - 12.0) * s, 64.0 * s)
}

/// Phones live below drives in the same DEVICES section.
pub fn phone_item_rect(index: usize, num_places: usize, num_drives: usize, s: f32) -> Rect {
    let header_y = drives_section_y(num_places, s) + 30.0 * s;
    let drives_h = num_drives as f32 * 64.0 * s;
    let y = header_y + drives_h + index as f32 * 56.0 * s;
    Rect::new(4.0 * s, y, (SIDEBAR_W - 12.0) * s, 56.0 * s)
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

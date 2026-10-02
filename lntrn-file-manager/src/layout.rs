use lntrn_render::Rect;
use std::sync::atomic::{AtomicBool, Ordering};

mod list;
mod pane_nav;
mod sidebar;

pub use list::{list_columns, list_header_h, list_rows_rect};
pub use pane_nav::{pane_nav, shown, Strip};
pub use sidebar::{build_sidebar_layout, SidebarLayout, SidebarSpec};

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
    if CHROME_HIDDEN.load(Ordering::Relaxed) {
        0.0
    } else {
        GRADIENT_H
    }
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

/// Positions of the zoom slider that List/Tree rows tell apart.
const LIST_ZOOM_STEPS: f32 = 16.0;

/// Gentler zoom multiplier for List/Tree rows — they're inherently dense,
/// so we don't want them to balloon like grid items.
/// 0.0 → 0.8x, 0.5 → 1.5x (default), 1.0 → 2.2x.
///
/// In steps, not continuous: the List/Tree font sizes are this multiplier
/// times a constant, and the text engine keeps every glyph it has ever
/// rasterised at every size (its atlas only grows, and once full new glyphs
/// come out blank). A slider dragged end to end used to walk each font
/// through well over a hundred sizes; now it is seventeen, and rows, icons
/// and text step together.
pub fn list_zoom_multiplier(zoom: f32) -> f32 {
    let zoom = if zoom.is_finite() { zoom.clamp(0.0, 1.0) } else { 0.5 };
    let step = (zoom * LIST_ZOOM_STEPS).round() / LIST_ZOOM_STEPS;
    0.8 + step * 1.4
}

/// Scaled layout helper. All public functions return physical-pixel values.
#[allow(dead_code)]
pub fn title_bar_h(s: f32) -> f32 {
    title_bar_h_base() * s
}
#[allow(dead_code)]
pub fn gradient_h(s: f32) -> f32 {
    GRADIENT_H * s
}
pub fn sidebar_w(s: f32) -> f32 {
    SIDEBAR_W * s
}
pub fn item_size(s: f32, zoom: f32) -> f32 {
    (ITEM_SIZE * zoom_multiplier(zoom)).max(60.0) * s
}
pub fn icon_size(s: f32, zoom: f32) -> f32 {
    ICON_SIZE * s * zoom_multiplier(zoom)
}
#[allow(dead_code)]
pub fn item_pad(s: f32) -> f32 {
    ITEM_PAD * s
}

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
    // Reserve space for split-toggle, preview-toggle, sort, and search buttons.
    let trailing_space = 172.0 * s;
    Rect::new(
        path_x,
        y + 5.0 * s,
        width - path_x - trailing_space,
        38.0 * s,
    )
}

/// Split-view toggle button — sits left of the preview toggle.
pub fn split_toggle_rect(width: f32, s: f32) -> Rect {
    let y = nav_bar_y(s);
    Rect::new(width - 168.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
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

pub fn content_rect(width: f32, height: f32, s: f32) -> Rect {
    let top = content_top(s);
    let bottom = content_bottom(height, s);
    Rect::new(SIDEBAR_W * s, top, width - SIDEBAR_W * s, bottom - top)
}

// ── Split view layout ───────────────────────────────────────────────────────
//
// Two pane columns share the area right of the sidebar, separated by a
// draggable divider. The left pane keeps the tab bar; the right pane's
// content reclaims that strip (it has no tabs).

/// Width of the draggable divider between the panes.
pub const SPLIT_HANDLE_W: f32 = 8.0;

/// Divider ratio bounds (fraction of the content area given to the left pane).
pub const SPLIT_RATIO_MIN: f32 = 0.2;
pub const SPLIT_RATIO_MAX: f32 = 0.8;
/// Narrowest a pane gets (logical px): its nav buttons and a path strip
/// worth reading side by side (see `pane_nav`).
pub const SPLIT_PANE_MIN_W: f32 = 400.0;

/// Width of the left pane out of `avail` (both panes, without the handle).
/// The ratio alone let a pane shrink to a fifth of a narrow window, where
/// its right-aligned buttons sat on top of Back/Forward/Up and took their
/// clicks. Each pane now keeps `SPLIT_PANE_MIN_W`; a window too narrow for
/// two of those is shared evenly.
fn split_left_w(avail: f32, ratio: f32, s: f32) -> f32 {
    let ratio = if ratio.is_finite() { ratio } else { 0.5 };
    let lw = avail * ratio.clamp(SPLIT_RATIO_MIN, SPLIT_RATIO_MAX);
    let floor = (SPLIT_PANE_MIN_W * s).min(avail * 0.5);
    lw.clamp(floor, avail - floor)
}

/// Horizontal columns of the two panes: (left_x, left_w, right_x, right_w).
pub fn split_pane_cols(width: f32, ratio: f32, s: f32) -> (f32, f32, f32, f32) {
    let x0 = SIDEBAR_W * s;
    let handle = SPLIT_HANDLE_W * s;
    let avail = (width - x0 - handle).max(0.0);
    let lw = split_left_w(avail, ratio, s);
    let rx = x0 + lw + handle;
    (x0, lw, rx, (width - rx).max(0.0))
}

/// The divider ratio for a divider dragged to `cursor_x`: where the panes
/// will actually be, so the ratio that is saved is the one that was shown.
/// `None` when there is no room for panes at all.
pub fn split_ratio_at(cursor_x: f32, width: f32, s: f32) -> Option<f32> {
    let x0 = SIDEBAR_W * s;
    let avail = width - x0 - SPLIT_HANDLE_W * s;
    if avail <= 0.0 {
        return None;
    }
    Some(split_left_w(avail, (cursor_x - x0) / avail, s) / avail)
}

/// The divider handle, spanning nav bar through content.
pub fn split_divider_rect(width: f32, height: f32, ratio: f32, s: f32) -> Rect {
    let (lx, lw, _, _) = split_pane_cols(width, ratio, s);
    let top = nav_bar_y(s);
    Rect::new(
        lx + lw,
        top,
        SPLIT_HANDLE_W * s,
        content_bottom(height, s) - top,
    )
}

/// Per-pane nav bar strip.
pub fn pane_nav_bar_rect(pane_x: f32, pane_w: f32, s: f32) -> Rect {
    Rect::new(pane_x, nav_bar_y(s), pane_w, NAV_BAR_H * s)
}

/// Left-pane tab bar (right pane has no tabs).
pub fn pane_tab_bar_rect(pane_x: f32, pane_w: f32, s: f32) -> Rect {
    Rect::new(pane_x, tab_bar_y(s), pane_w, TAB_BAR_H * s)
}

/// Pane content column. The right pane (no tab bar) starts higher.
pub fn pane_content_rect(pane_x: f32, pane_w: f32, height: f32, s: f32, has_tab_bar: bool) -> Rect {
    let top = if has_tab_bar {
        content_top(s)
    } else {
        tab_bar_y(s)
    };
    let bottom = content_bottom(height, s);
    Rect::new(pane_x, top, pane_w, bottom - top)
}

/// Width of the resize handle that sits on the preview pane's left edge.
pub const PREVIEW_HANDLE_W: f32 = 6.0;

/// Min/max bounds for the preview pane width (logical px, pre-scale).
pub const PREVIEW_MIN_W: f32 = 220.0;
pub const PREVIEW_MAX_FRACTION: f32 = 0.6; // never more than 60% of content area
/// Narrowest file list the preview pane may leave beside it (logical px).
pub const PREVIEW_MIN_LIST_W: f32 = 200.0;

/// Effective preview width in physical px, clamped to bounds for the current
/// content area. Returns 0 if the preview is closed or would not fit.
pub fn preview_effective_w(content_w_px: f32, preview_w_logical: f32, open: bool, s: f32) -> f32 {
    if !open {
        return 0.0;
    }
    // Not enough room for the pane AND a usable list beside it: no pane.
    // (It used to keep its minimum width regardless, leaving the list a
    // negative width and drawing itself over the sidebar.)
    if content_w_px - PREVIEW_MIN_W * s < PREVIEW_MIN_LIST_W * s {
        return 0.0;
    }
    let min = PREVIEW_MIN_W * s;
    let max = (content_w_px * PREVIEW_MAX_FRACTION).max(min);
    (preview_w_logical * s).clamp(min, max)
}

pub fn preview_pane_rect(content: Rect, preview_w: f32) -> Rect {
    Rect::new(
        content.x + content.w - preview_w,
        content.y,
        preview_w,
        content.h,
    )
}

pub fn preview_handle_rect(content: Rect, preview_w: f32, s: f32) -> Rect {
    let hw = PREVIEW_HANDLE_W * s;
    Rect::new(
        content.x + content.w - preview_w - hw * 0.5,
        content.y,
        hw,
        content.h,
    )
}

pub fn status_rect(width: f32, height: f32, s: f32) -> Rect {
    Rect::new(0.0, height - STATUS_BAR_H * s, width, STATUS_BAR_H * s)
}

pub fn list_row_h(s: f32, zoom: f32) -> f32 {
    LIST_ROW_H * list_zoom_multiplier(zoom) * s
}
pub fn search_list_row_h(s: f32, zoom: f32) -> f32 {
    56.0 * list_zoom_multiplier(zoom) * s
}
pub fn tree_row_h(s: f32, zoom: f32) -> f32 {
    TREE_ROW_H * list_zoom_multiplier(zoom) * s
}
#[allow(dead_code)]
pub fn tree_indent(s: f32, zoom: f32) -> f32 {
    TREE_INDENT * list_zoom_multiplier(zoom) * s
}

pub fn list_content_height(entry_count: usize, s: f32, zoom: f32) -> f32 {
    entry_count as f32 * list_row_h(s, zoom)
}

pub fn tree_content_height(entry_count: usize, s: f32, zoom: f32) -> f32 {
    entry_count as f32 * tree_row_h(s, zoom)
}

#[allow(dead_code)]
pub fn list_row_rect(
    index: usize,
    content_x: f32,
    content_w: f32,
    base_y: f32,
    s: f32,
    zoom: f32,
) -> Rect {
    let rh = list_row_h(s, zoom);
    let y = base_y + index as f32 * rh;
    Rect::new(content_x, y, content_w, rh)
}

#[allow(dead_code)]
pub fn tree_row_rect(
    index: usize,
    depth: usize,
    content_x: f32,
    content_w: f32,
    base_y: f32,
    s: f32,
    zoom: f32,
) -> Rect {
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

pub fn file_item_rect(
    index: usize,
    cols: usize,
    content_x: f32,
    base_y: f32,
    s: f32,
    zoom: f32,
) -> Rect {
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
    Rect::new(
        cell.x + (cell.w - w) * 0.5,
        cell.y + (cell.h - h) * 0.5,
        w,
        h,
    )
}

#[cfg(test)]
mod tests;

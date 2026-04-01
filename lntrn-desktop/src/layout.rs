use lntrn_render::Rect;

// Base design values (at 1x scale)
const GRADIENT_H: f32 = 4.0;
const GLOBAL_TAB_H: f32 = 46.0;
const NAV_BAR_H: f32 = 48.0;
const FILE_TAB_H: f32 = 46.0;
const SIDEBAR_W: f32 = 200.0;
const STATUS_BAR_H: f32 = 28.0;
const ITEM_SIZE: f32 = 80.0;
const ICON_SIZE: f32 = 48.0;
const ITEM_PAD: f32 = 8.0;
const LIST_ROW_H: f32 = 40.0;
const TREE_ROW_H: f32 = 36.0;
const TREE_INDENT: f32 = 24.0;

/// Zoom 0.0 → 1.8x, 0.5 → 2.9x, 1.0 → 4.0x
pub fn zoom_multiplier(zoom: f32) -> f32 {
    1.8 + zoom * 2.2
}

pub fn gradient_h(s: f32) -> f32 { GRADIENT_H * s }
pub fn sidebar_w(s: f32) -> f32 { SIDEBAR_W * s }
pub fn item_size(s: f32, zoom: f32) -> f32 { (ITEM_SIZE * zoom_multiplier(zoom)).max(60.0) * s }
pub fn icon_size(s: f32, zoom: f32) -> f32 { ICON_SIZE * s * zoom_multiplier(zoom) }
pub fn item_pad(s: f32) -> f32 { ITEM_PAD * s }

// ── Global header (shared across all panels) ──────────────────────────────

/// Y=0: first gradient strip
pub fn gradient1_y(_s: f32) -> f32 { 0.0 }

/// Y after gradient 1: global tab bar
pub fn global_tab_y(s: f32) -> f32 { GRADIENT_H * s }

/// Global tab bar rect (full width, between the two gradient strips)
pub fn global_tab_rect(width: f32, s: f32) -> Rect {
    Rect::new(0.0, global_tab_y(s), width, GLOBAL_TAB_H * s)
}

/// Y after global tabs: second gradient strip
pub fn gradient2_y(s: f32) -> f32 { (GRADIENT_H + GLOBAL_TAB_H) * s }

/// Y where panel content begins (below both gradients + global tabs)
pub fn panel_top(s: f32) -> f32 { (GRADIENT_H + GLOBAL_TAB_H + GRADIENT_H) * s }

// ── Files panel layout (below panel_top) ──────────────────────────────────

/// Nav bar Y (first thing in the files panel content)
pub fn nav_bar_y(s: f32) -> f32 { panel_top(s) }

pub fn nav_bar_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    Rect::new(x, nav_bar_y(s), width - x, NAV_BAR_H * s)
}

pub fn view_toggle_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 6.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn back_button_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 48.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn forward_button_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 86.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn up_button_rect(s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    Rect::new(x + 124.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

pub fn path_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    let y = nav_bar_y(s);
    let path_x = x + 172.0 * s;
    let search_space = 46.0 * s;
    Rect::new(path_x, y + 5.0 * s, width - path_x - search_space, 38.0 * s)
}

pub fn search_button_rect(width: f32, s: f32) -> Rect {
    let y = nav_bar_y(s);
    Rect::new(width - 42.0 * s, y + 6.0 * s, 36.0 * s, 36.0 * s)
}

/// File tab bar Y (below nav bar)
pub fn tab_bar_y(s: f32) -> f32 {
    panel_top(s) + NAV_BAR_H * s
}

pub fn tab_bar_rect(width: f32, s: f32) -> Rect {
    let x = SIDEBAR_W * s;
    Rect::new(x, tab_bar_y(s), width - x, FILE_TAB_H * s)
}

/// Content area top (below nav bar + file tabs)
pub fn content_top(s: f32) -> f32 {
    panel_top(s) + NAV_BAR_H * s + FILE_TAB_H * s
}

pub fn content_bottom(height: f32, s: f32) -> f32 {
    height - STATUS_BAR_H * s
}

pub fn content_rect_with_bottom(width: f32, bottom: f32, s: f32) -> Rect {
    let top = content_top(s);
    Rect::new(SIDEBAR_W * s, top, width - SIDEBAR_W * s, bottom - top)
}

pub fn sidebar_rect(height: f32, s: f32) -> Rect {
    let top = panel_top(s);
    Rect::new(0.0, top, SIDEBAR_W * s, height - top)
}

pub fn sidebar_item_rect(index: usize, s: f32) -> Rect {
    let mut y = panel_top(s) + 42.0 * s;
    y += index as f32 * 40.0 * s;
    Rect::new(4.0 * s, y, (SIDEBAR_W - 12.0) * s, 40.0 * s)
}

pub fn drives_section_y(num_places: usize, s: f32) -> f32 {
    panel_top(s) + 42.0 * s + num_places as f32 * 40.0 * s + 20.0 * s
}

pub fn drive_item_rect(index: usize, num_places: usize, s: f32) -> Rect {
    let mut y = drives_section_y(num_places, s) + 30.0 * s;
    y += index as f32 * 64.0 * s;
    Rect::new(4.0 * s, y, (SIDEBAR_W - 12.0) * s, 64.0 * s)
}

pub fn content_rect(width: f32, height: f32, s: f32) -> Rect {
    let top = content_top(s);
    let bottom = content_bottom(height, s);
    Rect::new(SIDEBAR_W * s, top, width - SIDEBAR_W * s, bottom - top)
}

pub fn status_rect(width: f32, height: f32, s: f32) -> Rect {
    Rect::new(SIDEBAR_W * s, height - STATUS_BAR_H * s, width - SIDEBAR_W * s, STATUS_BAR_H * s)
}

pub fn list_row_h(s: f32) -> f32 { LIST_ROW_H * s }
pub fn tree_row_h(s: f32) -> f32 { TREE_ROW_H * s }
pub fn tree_indent(s: f32) -> f32 { TREE_INDENT * s }

pub fn list_content_height(entry_count: usize, s: f32) -> f32 {
    entry_count as f32 * LIST_ROW_H * s
}

pub fn tree_content_height(entry_count: usize, s: f32) -> f32 {
    entry_count as f32 * TREE_ROW_H * s
}

pub fn list_row_rect(index: usize, content_x: f32, content_w: f32, base_y: f32, s: f32) -> Rect {
    let y = base_y + index as f32 * LIST_ROW_H * s;
    Rect::new(content_x, y, content_w, LIST_ROW_H * s)
}

pub fn tree_row_rect(index: usize, depth: usize, content_x: f32, content_w: f32, base_y: f32, s: f32) -> Rect {
    let y = base_y + index as f32 * TREE_ROW_H * s;
    let indent = depth as f32 * TREE_INDENT * s;
    Rect::new(content_x + indent, y, content_w - indent, TREE_ROW_H * s)
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

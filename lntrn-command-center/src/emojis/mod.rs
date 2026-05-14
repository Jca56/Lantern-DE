//! Emojis overlay page.
//!
//! Loads Fluent UI Emoji 3D PNGs from `~/.lantern/emojis/3d/{codepoint}.png`
//! and lays them out in a category-tabbed, filterable grid. See
//! `scripts/setup-emojis.py` for the one-time asset/data-table install.

pub mod data;
pub mod render;

use std::path::PathBuf;
use lntrn_render::Rect;

/// Top-level emoji groupings (Unicode CLDR groups).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Smileys,
    People,
    AnimalsNature,
    FoodDrink,
    Activities,
    TravelPlaces,
    Objects,
    Symbols,
    Flags,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Smileys,
        Category::People,
        Category::AnimalsNature,
        Category::FoodDrink,
        Category::Activities,
        Category::TravelPlaces,
        Category::Objects,
        Category::Symbols,
        Category::Flags,
    ];

    #[allow(dead_code)] // used in tooltips planned for phase 2
    pub fn label(self) -> &'static str {
        match self {
            Category::Smileys => "Smileys & Emotion",
            Category::People => "People & Body",
            Category::AnimalsNature => "Animals & Nature",
            Category::FoodDrink => "Food & Drink",
            Category::Activities => "Activities",
            Category::TravelPlaces => "Travel & Places",
            Category::Objects => "Objects",
            Category::Symbols => "Symbols",
            Category::Flags => "Flags",
        }
    }

    /// Codepoint used as the visual tab icon for this category.
    pub fn icon_unicode(self) -> &'static str {
        match self {
            Category::Smileys => "1f600",       // 😀
            Category::People => "1f44b",        // 👋
            Category::AnimalsNature => "1f436", // 🐶
            Category::FoodDrink => "1f355",     // 🍕
            Category::Activities => "26bd",      // ⚽
            Category::TravelPlaces => "1f697",  // 🚗
            Category::Objects => "1f4a1",       // 💡
            Category::Symbols => "1f4af",       // 💯
            Category::Flags => "1f1fa-1f1f8",   // 🇺🇸
        }
    }
}

/// Logical-px layout constants. Scaled by `scale` at draw time.
pub const FILTER_BAR_H: f32 = 56.0;
pub const CATEGORY_STRIP_H: f32 = 60.0;
pub const CATEGORY_GAP: f32 = 8.0;
pub const GRID_PAD: f32 = 16.0;
pub const CELL_SIZE: f32 = 56.0;
pub const CELL_GAP: f32 = 8.0;
pub const SECTION_GAP: f32 = 8.0;

/// Returns the on-disk path for a given codepoint string (e.g. `"1f600"`
/// or `"1f1fa 1f1f8"`). Multi-codepoint emoji use dashes in the filename.
pub fn png_path_for(unicode: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let name = unicode.replace(' ', "-");
    home.join(".lantern").join("emojis").join("png").join(format!("{}.png", name))
}

/// Hit-test result inside the emoji overlay.
#[derive(Debug, Clone, Copy)]
pub enum Hit {
    None,
    Filter,
    Category(Category),
    Cell(usize),
}

/// Persistent state for the emojis overlay.
pub struct EmojisState {
    /// True when the overlay is on screen.
    pub open: bool,
    /// Currently active category tab.
    pub active_category: Category,
    /// Search filter input. Empty means no filter.
    pub filter: crate::search::input::Input,
    /// Vertical scroll offset of the grid (logical px).
    pub scroll: f32,
    /// Hover state, used for visual feedback.
    pub hover_entry: Option<usize>,
    pub hover_category: Option<Category>,
    /// Brief post-click highlight on the entry just copied. We display
    /// the flash for ~280 ms.
    pub recent_copy: Option<(usize, std::time::Instant)>,
}

impl Default for EmojisState {
    fn default() -> Self {
        Self {
            open: false,
            active_category: Category::Smileys,
            filter: crate::search::input::Input::new(),
            scroll: 0.0,
            hover_entry: None,
            hover_category: None,
            recent_copy: None,
        }
    }
}

impl EmojisState {
    /// Whether any filter text is present.
    pub fn filtered(&self) -> bool {
        !self.filter.is_empty()
    }

    /// Return the indices into [`data::EMOJIS`] visible under the
    /// current category / filter.
    pub fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.query().to_lowercase();
        let needle = needle.trim();
        data::EMOJIS
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if needle.is_empty() {
                    e.category == self.active_category
                } else {
                    e.name.to_lowercase().contains(needle)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(needle))
                }
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn reset_scroll(&mut self) {
        self.scroll = 0.0;
    }
}

// ── Layout ──────────────────────────────────────────────────────────────────

pub fn filter_bar_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = GRID_PAD * scale;
    Rect::new(
        panel.x + pad,
        top_y + pad * 0.5,
        panel.w - pad * 2.0,
        FILTER_BAR_H * scale,
    )
}

pub fn category_strip_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = GRID_PAD * scale;
    let filter = filter_bar_rect(panel, top_y, scale);
    Rect::new(
        panel.x + pad,
        filter.y + filter.h + SECTION_GAP * scale,
        panel.w - pad * 2.0,
        CATEGORY_STRIP_H * scale,
    )
}

/// Tab rect for the i-th category in [`Category::ALL`].
pub fn category_tab_rect(panel: Rect, top_y: f32, scale: f32, idx: usize) -> Rect {
    let strip = category_strip_rect(panel, top_y, scale);
    let gap = CATEGORY_GAP * scale;
    let n = Category::ALL.len() as f32;
    let tab_w = (strip.w - gap * (n - 1.0)) / n;
    let x = strip.x + idx as f32 * (tab_w + gap);
    Rect::new(x, strip.y, tab_w, strip.h)
}

pub fn grid_rect(panel: Rect, top_y: f32, scale: f32, panel_bottom: f32) -> Rect {
    let strip = category_strip_rect(panel, top_y, scale);
    let pad = GRID_PAD * scale;
    let top = strip.y + strip.h + SECTION_GAP * scale;
    let h = (panel_bottom - top - pad * 0.5).max(0.0);
    Rect::new(panel.x + pad, top, panel.w - pad * 2.0, h)
}

/// How many cells fit per row inside the grid.
pub fn cells_per_row(grid: Rect, scale: f32) -> usize {
    let cell = CELL_SIZE * scale;
    let gap = CELL_GAP * scale;
    let total = grid.w + gap;
    let stride = cell + gap;
    (total / stride).floor().max(1.0) as usize
}

/// Cell rect at index in the visible list (BEFORE scroll offset).
pub fn cell_rect_at(grid: Rect, scale: f32, visible_idx: usize, per_row: usize) -> Rect {
    let cell = CELL_SIZE * scale;
    let gap = CELL_GAP * scale;
    let row = visible_idx / per_row;
    let col = visible_idx % per_row;
    let row_stride = cell + gap;
    let col_stride = cell + gap;
    let x = grid.x + col as f32 * col_stride;
    let y = grid.y + row as f32 * row_stride;
    Rect::new(x, y, cell, cell)
}

/// Maximum scroll offset (logical px) for the current visible set.
pub fn max_scroll(visible_count: usize, per_row: usize, grid_h: f32, scale: f32) -> f32 {
    if visible_count == 0 || per_row == 0 {
        return 0.0;
    }
    let rows = ((visible_count + per_row - 1) / per_row) as f32;
    let cell = CELL_SIZE * scale;
    let gap = CELL_GAP * scale;
    let total = rows * cell + (rows - 1.0).max(0.0) * gap;
    (total - grid_h).max(0.0)
}

/// Hit-test the overlay. `panel_bottom` is the physical y of the panel
/// bottom (so the grid clip is correct).
pub fn hit_test(
    state: &EmojisState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    panel_bottom: f32,
    px: f32,
    py: f32,
) -> Hit {
    let filter = filter_bar_rect(panel, top_y, scale);
    if point_in(filter, px, py) {
        return Hit::Filter;
    }
    for (i, c) in Category::ALL.iter().enumerate() {
        let r = category_tab_rect(panel, top_y, scale, i);
        if point_in(r, px, py) {
            return Hit::Category(*c);
        }
    }
    let grid = grid_rect(panel, top_y, scale, panel_bottom);
    if point_in(grid, px, py) {
        let per_row = cells_per_row(grid, scale);
        let visible = state.visible_indices();
        let cell = CELL_SIZE * scale;
        let gap = CELL_GAP * scale;
        let row_stride = cell + gap;
        let col_stride = cell + gap;
        // Apply scroll: cells are drawn at (grid.y - scroll + row * stride)
        let scroll_px = state.scroll * scale;
        let rel_y = py - grid.y + scroll_px;
        let row = (rel_y / row_stride).floor() as i32;
        let rel_x = px - grid.x;
        let col = (rel_x / col_stride).floor() as i32;
        if row >= 0 && col >= 0 && (col as usize) < per_row {
            let idx = row as usize * per_row + col as usize;
            // Reject clicks that fell inside the gap rather than the cell.
            let cell_x = grid.x + col as f32 * col_stride;
            let cell_y = grid.y - scroll_px + row as f32 * row_stride;
            let cell_rect = Rect::new(cell_x, cell_y, cell, cell);
            if point_in(cell_rect, px, py) && idx < visible.len() {
                return Hit::Cell(idx);
            }
        }
    }
    Hit::None
}

fn point_in(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

//! Search orchestration: text input + apps provider + ranked results.
//!
//! Phase 2.2: text input + apps provider + simple text-list result
//! rendering. Phases 2.5+ replace the text list with an icon grid and
//! add file/web/math/command/clipboard providers via `dispatch`.

pub mod apps;
pub mod fuzzy;
pub mod input;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;
use self::apps::{AppsProvider, RankedEntry};
use self::input::Input;

/// Hard cap on ranked results. Set well above what fits in the visible
/// viewport — anything past the bottom of the panel is reachable via
/// scroll. 50 is plenty for fuzzy matches against typical app sets.
pub const MAX_RESULTS: usize = 50;

/// Top-level search state for the panel.
///
/// Owns the text input and the most recent ranked result list. The
/// `AppsProvider` lives one level up in `AppState` so the launcher
/// (pinned favorites) can also reach it without going through search.
pub struct Search {
    pub input: Input,
    results: Vec<RankedEntry>,
    /// Vertical scroll offset (physical px) into the result list.
    /// Clamped each render to `[0, max_scroll]` based on the current
    /// viewport height. Reset to 0 whenever the query changes.
    pub scroll_offset: f32,
    /// "All apps" browse mode — populated by clicking the waffle icon
    /// on the search bar. Shows every non-NoDisplay app alphabetically.
    /// Cleared the moment the user types into the input.
    pub all_apps_mode: bool,
}

impl Search {
    pub fn new() -> Self {
        Self {
            input: Input::new(),
            results: Vec::new(),
            scroll_offset: 0.0,
            all_apps_mode: false,
        }
    }

    /// Re-rank results against the current input, using the provided
    /// apps cache. Called when the input buffer changes.
    pub fn refresh_results(&mut self, apps: &AppsProvider, hidden: &crate::launcher::hidden::Hidden) {
        // Any keystroke exits all-apps browse mode — typing implies the
        // user wants to filter, not browse the full list.
        self.all_apps_mode = false;
        let q = self.input.query();
        if q.is_empty() {
            self.results.clear();
            self.scroll_offset = 0.0;
            return;
        }
        self.results = apps.rank(q, MAX_RESULTS, hidden);
        // Reset scroll on every query change so the user sees the
        // top-ranked match without having to scroll back up.
        self.scroll_offset = 0.0;
    }

    /// Open "all apps" mode — populate `results` with every visible app
    /// sorted alphabetically. Triggered by the waffle icon on the
    /// search bar. `hidden` filters out user-hidden app_ids.
    pub fn show_all_apps(&mut self, apps: &AppsProvider, hidden: &crate::launcher::hidden::Hidden) {
        let mut all: Vec<RankedEntry> = (0..apps.count())
            .filter(|i| {
                let Some(e) = apps.get(*i) else { return false };
                !e.no_display && !hidden.is_hidden(&e.app_id)
            })
            .map(|i| RankedEntry {
                entry_idx: i,
                score: 0.0,
            })
            .collect();
        all.sort_by(|a, b| {
            let na = apps.get(a.entry_idx).map(|e| e.name.as_str()).unwrap_or("");
            let nb = apps.get(b.entry_idx).map(|e| e.name.as_str()).unwrap_or("");
            na.to_lowercase().cmp(&nb.to_lowercase())
        });
        self.results = all;
        self.scroll_offset = 0.0;
        self.all_apps_mode = true;
        self.input.clear();
    }

    /// Reset to a clean slate (called when the panel re-opens).
    pub fn reset(&mut self) {
        self.input.clear();
        self.results.clear();
        self.scroll_offset = 0.0;
        self.all_apps_mode = false;
    }

    /// Borrow the result list for rendering / navigation.
    pub fn results(&self) -> &[RankedEntry] {
        &self.results
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Layout constants (logical pixels) for the result list.
const RESULT_ROW_HEIGHT: f32 = 60.0;
const RESULT_FONT_SIZE: f32 = 28.0;
const RESULT_GAP: f32 = 4.0;
const RESULT_TOP_MARGIN: f32 = 16.0;
const RESULT_ICON_SIZE: f32 = 36.0;
const RESULT_ICON_PAD_LEFT: f32 = 12.0;
const RESULT_TEXT_PAD_LEFT: f32 = 16.0;

/// Grid layout (used in `all_apps_mode`).
pub const GRID_COLS: usize = 5;
pub const GRID_TILE_SIZE: f32 = 120.0;
pub const GRID_TILE_GAP: f32 = 32.0;
pub const GRID_LABEL_FONT: f32 = 18.0;
pub const GRID_LABEL_GAP: f32 = 12.0;
pub const GRID_ROW_GAP: f32 = 32.0;

/// White text — user prefers white over the Studio tan everywhere.
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
/// Accent gold #C8860A.
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const SECONDARY_ALPHA: f32 = 0.55;
const HIGHLIGHT_ALPHA: f32 = 0.06;

fn text_color(alpha: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(alpha)
}
fn accent_color(alpha: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
}

/// Y-coordinate (physical px) where content beneath the search input
/// should start. Helper so launcher and result-list use the same offset.
pub fn content_top_y(panel: Rect, scale: f32) -> f32 {
    panel.y
        + crate::controls::total_logical_height() * scale
        + (input::SEARCH_HORIZONTAL_PAD * 0.5 + input::SEARCH_ROW_HEIGHT) * scale
}

/// Draw the search input row at the top of the panel.
#[allow(clippy::too_many_arguments)]
pub fn draw_input(
    painter: &mut Painter,
    text: &mut TextRenderer,
    search: &Search,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    waffle_hovered: bool,
) {
    input::draw(
        painter,
        text,
        &search.input,
        panel,
        scale,
        alpha,
        surface_w,
        surface_h,
        search.all_apps_mode,
        waffle_hovered,
    );
}

/// Draw the ranked result list. Only call when `search.input` is
/// non-empty — the empty state is owned by the launcher (pinned).
/// Pushes one `IconRequest` per result into `icons`. `selected_result`
/// highlights one row (used for keyboard navigation / Enter-to-launch).
pub fn draw_results(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    search: &Search,
    apps: &AppsProvider,
    selected_result: Option<usize>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let row_h = RESULT_ROW_HEIGHT * scale;
    let gap = RESULT_GAP * scale;
    let font = RESULT_FONT_SIZE * scale;

    let list_x = panel.x + pad;
    let list_w = panel.w - pad * 2.0;
    let list_y_start = content_top_y(panel, scale) + RESULT_TOP_MARGIN * scale;
    // Bottom of the result viewport — clamp inside the panel so a long
    // result list doesn't bleed past the panel rounded corners.
    let viewport_bottom = panel.y + panel.h - RESULT_TOP_MARGIN * scale;

    if search.all_apps_mode {
        draw_results_grid(
            painter, text, icons, search, apps, selected_result,
            panel, scale, alpha, surface_w, surface_h,
        );
        return;
    }

    let results = search.results();
    if results.is_empty() {
        text.queue(
            "No matches.",
            font,
            list_x,
            list_y_start,
            text_color(SECONDARY_ALPHA * alpha),
            list_w,
            surface_w,
            surface_h,
        );
        return;
    }

    let icon_size = RESULT_ICON_SIZE * scale;
    let icon_pad_left = RESULT_ICON_PAD_LEFT * scale;
    let text_pad_left = RESULT_TEXT_PAD_LEFT * scale;
    let text_x_offset = icon_pad_left + icon_size + text_pad_left;

    // Clip everything to the result viewport so scrolled-out rows don't
    // paint over the search bar / panel edges.
    let clip = Rect::new(
        list_x,
        list_y_start,
        list_w,
        (viewport_bottom - list_y_start).max(0.0),
    );
    painter.push_clip(clip);
    text.push_clip([clip.x, clip.y, clip.w, clip.h]);

    let scroll = search.scroll_offset;

    for (i, r) in results.iter().enumerate() {
        let Some(entry) = apps.get(r.entry_idx) else { continue };
        let row_y = list_y_start + (i as f32) * (row_h + gap) - scroll;
        // Skip rows entirely outside the viewport — saves draw calls
        // when the user scrolls a long list, and avoids wasted text
        // glyph layout for off-screen items.
        if row_y + row_h < list_y_start || row_y > viewport_bottom {
            continue;
        }
        let row_rect = Rect::new(list_x, row_y, list_w, row_h);

        let is_selected = selected_result == Some(i);
        if is_selected {
            // Accent gold tinted row + thin accent stroke.
            painter.rect_filled(row_rect, 6.0 * scale, accent_color(0.18 * alpha));
            painter.rect_stroke_sdf(row_rect, 6.0 * scale, 1.5 * scale, accent_color(0.55 * alpha));
        } else if i % 2 == 0 {
            painter.rect_filled(
                row_rect,
                6.0 * scale,
                Color::rgba(1.0, 1.0, 1.0, HIGHLIGHT_ALPHA * alpha),
            );
        }

        // Icon on the left, vertically centered.
        let icon_y = row_y + (row_h - icon_size) / 2.0;
        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: list_x + icon_pad_left,
            y: icon_y,
            size: icon_size,
            opacity: alpha,
            clip: Some([clip.x, clip.y, clip.w, clip.h]),
        });

        let text_y = row_y + (row_h - font) / 2.0;
        let primary = text_color(alpha);
        let secondary = text_color(SECONDARY_ALPHA * alpha);

        // Primary: app name (shifted right past the icon).
        // Secondary: app_id in the right gutter.
        text.queue(
            &entry.name,
            font,
            list_x + text_x_offset,
            text_y,
            primary,
            list_w * 0.55 - text_x_offset,
            surface_w,
            surface_h,
        );
        text.queue(
            &entry.app_id,
            font * 0.85,
            list_x + list_w * 0.6,
            text_y + 2.0 * scale,
            secondary,
            list_w * 0.4 - 12.0 * scale,
            surface_w,
            surface_h,
        );
    }

    text.pop_clip();
    painter.pop_clip();
}

/// Maximum scroll offset for the current panel + result count, in
/// physical pixels. The render loop uses this to clamp `scroll_offset`
/// after each axis event so scrolling can't run past the bottom row.
pub fn max_scroll(search: &Search, panel: Rect, scale: f32) -> f32 {
    let n = search.results().len();
    if n == 0 {
        return 0.0;
    }
    let list_y_start = content_top_y(panel, scale) + RESULT_TOP_MARGIN * scale;
    let viewport_bottom = panel.y + panel.h - RESULT_TOP_MARGIN * scale;
    let viewport_h = (viewport_bottom - list_y_start).max(0.0);

    let total_h = if search.all_apps_mode {
        let tile = GRID_TILE_SIZE * scale;
        let row_gap = GRID_ROW_GAP * scale;
        let label_gap = GRID_LABEL_GAP * scale;
        let label_font = GRID_LABEL_FONT * scale;
        let cell_h = tile + label_gap + label_font;
        let rows = ((n + GRID_COLS - 1) / GRID_COLS) as f32;
        rows * cell_h + (rows - 1.0).max(0.0) * row_gap
    } else {
        let row_h = RESULT_ROW_HEIGHT * scale;
        let gap = RESULT_GAP * scale;
        let nf = n as f32;
        nf * row_h + (nf - 1.0).max(0.0) * gap
    };
    (total_h - viewport_h).max(0.0)
}

/// Draw the result list as an icon grid (used in `all_apps_mode`).
fn draw_results_grid(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    search: &Search,
    apps: &AppsProvider,
    selected_result: Option<usize>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile = GRID_TILE_SIZE * scale;
    let tile_gap = GRID_TILE_GAP * scale;
    let row_gap = GRID_ROW_GAP * scale;
    let label_font = GRID_LABEL_FONT * scale;
    let label_gap = GRID_LABEL_GAP * scale;
    let cell_h = tile + label_gap + label_font;

    let list_x = panel.x + pad;
    let list_w = panel.w - pad * 2.0;
    let list_y_start = content_top_y(panel, scale) + RESULT_TOP_MARGIN * scale;
    let viewport_bottom = panel.y + panel.h - RESULT_TOP_MARGIN * scale;

    let results = search.results();
    if results.is_empty() {
        text.queue(
            "No apps installed.",
            RESULT_FONT_SIZE * scale,
            list_x,
            list_y_start,
            text_color(SECONDARY_ALPHA * alpha),
            list_w,
            surface_w,
            surface_h,
        );
        return;
    }

    // Center the grid horizontally inside the available width.
    let cols_total = GRID_COLS as f32 * tile + (GRID_COLS as f32 - 1.0) * tile_gap;
    let grid_x0 = list_x + (list_w - cols_total).max(0.0) / 2.0;

    let clip = Rect::new(
        list_x,
        list_y_start,
        list_w,
        (viewport_bottom - list_y_start).max(0.0),
    );
    painter.push_clip(clip);
    text.push_clip([clip.x, clip.y, clip.w, clip.h]);

    let scroll = search.scroll_offset;

    for (i, r) in results.iter().enumerate() {
        let Some(entry) = apps.get(r.entry_idx) else { continue };
        let col = i % GRID_COLS;
        let row = i / GRID_COLS;
        let cell_x = grid_x0 + col as f32 * (tile + tile_gap);
        let cell_y = list_y_start + row as f32 * (cell_h + row_gap) - scroll;

        // Cull rows fully outside the viewport.
        if cell_y + cell_h < list_y_start || cell_y > viewport_bottom {
            continue;
        }

        let tile_rect = Rect::new(cell_x, cell_y, tile, tile);
        let is_selected = selected_result == Some(i);

        // No background plate — only a soft accent ring on the
        // selected tile so keyboard nav stays visible.
        if is_selected {
            painter.rect_stroke_sdf(
                tile_rect,
                16.0 * scale,
                2.0 * scale,
                accent_color(0.55 * alpha),
            );
        }

        let inset = tile * 0.13;
        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: cell_x + inset,
            y: cell_y + inset,
            size: tile - inset * 2.0,
            opacity: alpha,
            clip: Some([clip.x, clip.y, clip.w, clip.h]),
        });

        // Truncated label centered under the tile.
        let label = grid_truncate(&entry.name, 12);
        let label_w = text.measure_width(&label, label_font);
        text.queue(
            &label,
            label_font,
            cell_x + (tile - label_w) / 2.0,
            cell_y + tile + label_gap,
            text_color(0.85 * alpha),
            tile + tile_gap,
            surface_w,
            surface_h,
        );
    }

    text.pop_clip();
    painter.pop_clip();
}

fn grid_truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

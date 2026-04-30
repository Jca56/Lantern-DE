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

/// Maximum number of results shown at once. Phase 2.5 will replace the
/// list with an icon grid; the count stays similar.
pub const MAX_RESULTS: usize = 8;

/// Top-level search state for the panel.
///
/// Owns the text input and the most recent ranked result list. The
/// `AppsProvider` lives one level up in `AppState` so the launcher
/// (pinned favorites) can also reach it without going through search.
pub struct Search {
    pub input: Input,
    results: Vec<RankedEntry>,
}

impl Search {
    pub fn new() -> Self {
        Self {
            input: Input::new(),
            results: Vec::new(),
        }
    }

    /// Re-rank results against the current input, using the provided
    /// apps cache. Called when the input buffer changes.
    pub fn refresh_results(&mut self, apps: &AppsProvider) {
        let q = self.input.query();
        if q.is_empty() {
            self.results.clear();
            return;
        }
        self.results = apps.rank(q, MAX_RESULTS);
    }

    /// Reset to a clean slate (called when the panel re-opens).
    pub fn reset(&mut self) {
        self.input.clear();
        self.results.clear();
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

/// Studio tan #e8dcc8 — passed through `Color::from_rgb8` so the sRGB
/// surface format (`Bgra8UnormSrgb`) gets the gamma right.
const TEXT_RGB: (u8, u8, u8) = (0xe8, 0xdc, 0xc8);
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
    panel.y + (input::SEARCH_HORIZONTAL_PAD * 0.5 + input::SEARCH_ROW_HEIGHT) * scale
}

/// Draw the search input row at the top of the panel.
pub fn draw_input(
    painter: &mut Painter,
    text: &mut TextRenderer,
    search: &Search,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
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

    for (i, r) in results.iter().enumerate() {
        let Some(entry) = apps.get(r.entry_idx) else { continue };
        let row_y = list_y_start + (i as f32) * (row_h + gap);
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
}

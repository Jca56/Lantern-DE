//! Launcher — pinned favorites + (Phase 2.5) result grid icons.
//!
//! When the search query is empty, we draw a row of pinned app tiles
//! beneath the search input. As the user types, the search/results
//! module takes over the same vertical space.

pub mod context_menu;
pub mod hidden;
pub mod icons;
pub mod pins;

use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use self::hidden::Hidden;
use self::pins::Pins;
use crate::render::IconRequest;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::search::input;

pub struct Launcher {
    pins: Pins,
    hidden: Hidden,
}

/// One slot in the mixed pinned grid. `App` carries a `DesktopEntry`
/// borrow (same as the legacy code path); `Path` carries a filesystem
/// path the user pinned from the Files view.
pub enum PinnedItem<'a> {
    App(&'a DesktopEntry),
    Path { path: PathBuf, is_dir: bool },
}

impl PinnedItem<'_> {
    /// Human-visible label rendered under the tile.
    pub fn label(&self) -> String {
        match self {
            PinnedItem::App(e) => e.name.clone(),
            PinnedItem::Path { path, .. } => path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        }
    }
}

impl Launcher {
    pub fn new() -> Self {
        Self {
            pins: Pins::load(),
            hidden: Hidden::load(),
        }
    }

    #[allow(dead_code)] // used by Phase 2.6 right-click pin/unpin handler
    pub fn pins(&self) -> &Pins {
        &self.pins
    }

    pub fn hidden(&self) -> &Hidden {
        &self.hidden
    }

    /// Toggle hidden state for an app_id. Returns whether the app is now hidden.
    pub fn toggle_hidden(&mut self, app_id: &str) -> bool {
        let now_hidden = self.hidden.toggle(app_id);
        tracing::info!(app_id, now_hidden, "hidden toggled");
        now_hidden
    }

    /// Look up the pinned app's `DesktopEntry` from the apps provider.
    /// Returns `None` for pinned ids that are no longer installed
    /// (e.g., the user uninstalled the app); those slots are skipped
    /// in the rendered row. Path entries are skipped entirely.
    pub fn pinned_entries<'a>(&'a self, apps: &'a AppsProvider) -> Vec<&'a DesktopEntry> {
        self.pins
            .items()
            .iter()
            .filter(|id| !is_path(id))
            .filter_map(|id| {
                // Linear scan; pin counts are tiny (typically <16) so it's fine.
                (0..apps.count())
                    .filter_map(|i| apps.get(i))
                    .find(|e| &e.app_id == id)
            })
            .collect()
    }

    /// Resolve the full pin list to a mixed `PinnedItem` list. App
    /// pins whose desktop entry can't be found are skipped; path pins
    /// are always kept.
    pub fn pinned_items<'a>(&'a self, apps: &'a AppsProvider) -> Vec<PinnedItem<'a>> {
        self.pins
            .items()
            .iter()
            .filter_map(|id| {
                if is_path(id) {
                    let path = PathBuf::from(id);
                    let is_dir = path.is_dir();
                    Some(PinnedItem::Path { path, is_dir })
                } else {
                    (0..apps.count())
                        .filter_map(|i| apps.get(i))
                        .find(|e| &e.app_id == id)
                        .map(PinnedItem::App)
                }
            })
            .collect()
    }

    /// Toggle pin state for an app_id.
    #[allow(dead_code)] // wired up by right-click handler
    pub fn toggle_pin(&mut self, app_id: &str) {
        let now_pinned = self.pins.toggle(app_id);
        tracing::info!(app_id, now_pinned, "pin toggled");
    }

    /// Reorder pins by index — used by drag-and-drop. Persists on commit.
    ///
    /// `visible_from` / `visible_to` are indices into the **visible**
    /// pin list (what the user sees and drags). `pins.items()` can
    /// contain entries for app_ids whose DesktopEntry isn't currently
    /// installed — those are filtered out of the visible list. We
    /// translate to the raw items-index space before reordering so a
    /// missing entry between two visible pins doesn't shift the move
    /// onto an unrelated slot.
    pub fn reorder_pins(&mut self, visible_from: usize, visible_to: usize, apps: &AppsProvider) {
        let mapping = self.visible_to_items_mapping(apps);
        let visible_len = mapping.len();
        if visible_from >= visible_len {
            return;
        }
        let items_from = mapping[visible_from];
        let items_to = if visible_to >= visible_len {
            self.pins.items().len()
        } else {
            mapping[visible_to]
        };
        self.pins.reorder(items_from, items_to);
        tracing::info!(
            visible_from,
            visible_to,
            items_from,
            items_to,
            "pins reordered"
        );
    }

    /// Map each *visible* pin index to the index of the same entry in
    /// `pins.items()`. App-ids whose DesktopEntry isn't installed are
    /// skipped; path entries are always visible.
    fn visible_to_items_mapping(&self, apps: &AppsProvider) -> Vec<usize> {
        self.pins
            .items()
            .iter()
            .enumerate()
            .filter_map(|(items_idx, id)| {
                let visible = if is_path(id) {
                    true
                } else {
                    (0..apps.count())
                        .filter_map(|i| apps.get(i))
                        .any(|e| &e.app_id == id)
                };
                visible.then_some(items_idx)
            })
            .collect()
    }
}

/// Treat any entry whose first character is `/` as a filesystem path.
pub fn is_path(item: &str) -> bool {
    item.starts_with('/')
}

/// Pick a (cache-key, freedesktop-icon-name) pair for a pinned path
/// tile. Folders resolve to the Lantern `folder` icon (a gold variant
/// of Adwaita's folder shape shipped at `~/.lantern/icons/folder.svg`).
/// Files map by extension to image/video/text-generic mime icons.
pub fn path_icon(path: &std::path::Path, is_dir: bool) -> (String, String) {
    if is_dir {
        return ("__pin_folder".into(), "folder".into());
    }
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "ico"
        | "heic" | "heif" | "avif" => ("__pin_image".into(), "image-x-generic".into()),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "m4v" | "3gp"
        | "ogv" => ("__pin_video".into(), "video-x-generic".into()),
        _ => ("__pin_file".into(), "text-x-generic".into()),
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Pinned tile dimensions (logical px). Phase 2.5 will swap the
/// placeholder rectangle for an actual icon; the tile size stays.
pub const PIN_TILE_SIZE: f32 = 120.0;
pub const PIN_TILE_GAP: f32 = 32.0;
pub const PIN_LABEL_FONT: f32 = 18.0;
pub const PIN_ROW_TOP_MARGIN: f32 = 24.0;
pub const PIN_LABEL_GAP: f32 = 12.0;
/// Vertical gap between pinned rows when pins wrap to multiple rows.
pub const PIN_ROW_GAP: f32 = 20.0;

/// Total vertical space the pinned row needs (logical px), including
/// the top margin from the search underline.
#[allow(dead_code)] // exported for future layout calculations (recents row, etc.)
pub const PIN_ROW_HEIGHT: f32 = PIN_ROW_TOP_MARGIN + PIN_TILE_SIZE + PIN_LABEL_GAP + PIN_LABEL_FONT;

/// Physical-pixel y where the first row of pin TILES starts (below the
/// section heading). `top_y` is the y the pinned section claims (just
/// under the search underline).
pub fn pins_row_top_y(top_y: f32, scale: f32) -> f32 {
    let label_gap = PIN_LABEL_GAP * scale;
    let section_label_font = SECTION_LABEL_FONT * scale;
    top_y + PIN_ROW_TOP_MARGIN * scale + section_label_font + label_gap
}

/// Logical-pixel rect for the i-th pin tile. Layout mirrors `draw`.
#[allow(dead_code)] // kept for future hit-test consumers (drag-snap, hover)
pub fn pin_tile_rect(panel: Rect, scale: f32, row_top: f32, idx: usize) -> Rect {
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile_size = PIN_TILE_SIZE * scale;
    let tile_gap = PIN_TILE_GAP * scale;
    let row_gap = PIN_ROW_GAP * scale;
    let label_font = PIN_LABEL_FONT * scale;
    let label_gap = PIN_LABEL_GAP * scale;
    let cell_h = tile_size + label_gap + label_font;
    let avail_w = panel.w - pad * 2.0;
    let cols = ((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize;
    let cols = cols.max(1);
    let col = idx % cols;
    let row = idx / cols;
    let x = panel.x + pad + col as f32 * (tile_size + tile_gap);
    let y = row_top + row as f32 * (cell_h + row_gap);
    Rect::new(x, y, tile_size, tile_size)
}

/// Draw the visual feedback for a pin drag-reorder in progress: a
/// translucent ghost icon following the cursor + a vertical insertion
/// indicator at the drop slot.
#[allow(clippy::too_many_arguments)]
pub fn draw_pin_drag_overlay(
    painter: &mut Painter,
    icons: &mut Vec<crate::render::IconRequest>,
    launcher: &Launcher,
    apps: &AppsProvider,
    drag: &crate::app::PinDrag,
    panel: Rect,
    row_top: f32,
    scale: f32,
    alpha: f32,
) {
    if !drag.started {
        return;
    }
    let pinned = launcher.pinned_items(apps);
    if pinned.is_empty() || drag.from_idx >= pinned.len() {
        return;
    }
    let entry = &pinned[drag.from_idx];

    // Drop indicator: a vertical accent-gold pill between two tiles
    // (or at the row's edge for first/last positions).
    let to = pin_drop_slot(
        panel,
        scale,
        row_top,
        pinned.len(),
        drag.current_x,
        drag.current_y,
    );
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile_size = PIN_TILE_SIZE * scale;
    let tile_gap = PIN_TILE_GAP * scale;
    let avail_w = panel.w - pad * 2.0;
    let cols = (((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize).max(1);
    let indicator_row = if to == pinned.len() {
        (to.saturating_sub(1)) / cols
    } else {
        to / cols
    };
    let indicator_col = if to == pinned.len() {
        (pinned.len() - indicator_row * cols).min(cols)
    } else {
        to % cols
    };
    let label_font = PIN_LABEL_FONT * scale;
    let label_gap = PIN_LABEL_GAP * scale;
    let row_gap = PIN_ROW_GAP * scale;
    let cell_h = tile_size + label_gap + label_font;
    let row_y = row_top + indicator_row as f32 * (cell_h + row_gap);
    let col_with_gap = tile_size + tile_gap;
    let indicator_x =
        panel.x + pad + indicator_col as f32 * col_with_gap - tile_gap * 0.5 - 1.5 * scale;
    let indicator = Rect::new(indicator_x, row_y, 3.0 * scale, tile_size);
    painter.rect_filled(indicator, 1.5 * scale, accent_color(0.85 * alpha));

    // Ghost icon following the cursor — slightly smaller + translucent.
    let ghost_size = tile_size * 0.85;
    let gx = drag.current_x - ghost_size / 2.0;
    let gy = drag.current_y - ghost_size / 2.0;
    let (cache_key, icon_name) = match entry {
        PinnedItem::App(e) => (e.app_id.clone(), e.icon_name.clone()),
        PinnedItem::Path { path, is_dir } => {
            let (k, n) = path_icon(path, *is_dir);
            (k, Some(n))
        }
    };
    icons.push(crate::render::IconRequest {
        app_id: cache_key,
        icon_name,
        x: gx,
        y: gy,
        size: ghost_size,
        opacity: 0.85 * alpha,
        clip: None,
    });
}

/// Compute the drop-target index (0..=`num_pins`) for a drag whose
/// cursor is at `(px, py)` in physical pixels. `row_top` should come
/// from [`pins_row_top_y`]. Wraps with the same column count `draw`
/// uses, so visual layout and drop math stay in sync.
pub fn pin_drop_slot(
    panel: Rect,
    scale: f32,
    row_top: f32,
    num_pins: usize,
    px: f32,
    py: f32,
) -> usize {
    if num_pins == 0 {
        return 0;
    }
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile_size = PIN_TILE_SIZE * scale;
    let tile_gap = PIN_TILE_GAP * scale;
    let row_gap = PIN_ROW_GAP * scale;
    let label_font = PIN_LABEL_FONT * scale;
    let label_gap = PIN_LABEL_GAP * scale;
    let cell_h = tile_size + label_gap + label_font;
    let avail_w = panel.w - pad * 2.0;
    let cols = ((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize;
    let cols = cols.max(1);

    let y_offset = py - row_top;
    let row = if y_offset < 0.0 {
        0
    } else {
        let row_with_gap = cell_h + row_gap;
        (y_offset / row_with_gap).floor() as usize
    };
    let max_row = num_pins.div_ceil(cols).saturating_sub(1);
    let row = row.min(max_row);

    let row_x_start = panel.x + pad;
    let x_offset = px - row_x_start;
    let col_with_gap = tile_size + tile_gap;
    // Insertion-point semantics: boundary between slot i and slot i+1
    // lands at tile i's center. Tile i's center sits at x_offset =
    // i*col_with_gap + tile_size/2, so we shift x_offset so that
    // boundaries map cleanly to integer col_with_gap multiples.
    let col = ((x_offset + col_with_gap - tile_size * 0.5) / col_with_gap).floor();
    let col = col.max(0.0) as usize;
    let row_pin_count = (num_pins - row * cols).min(cols);
    let col_clamped = col.min(row_pin_count);
    (row * cols + col_clamped).min(num_pins)
}

/// Bottom-y of the rendered pinned section. Mirrors the math inside
/// `draw` so callers (hit-testing, layout for the Open section) stay in
/// sync without needing to re-render.
pub fn pins_section_bottom(panel: Rect, top_y: f32, scale: f32, num_pins: usize) -> f32 {
    if num_pins == 0 {
        return top_y;
    }
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile_size = PIN_TILE_SIZE * scale;
    let tile_gap = PIN_TILE_GAP * scale;
    let label_font = PIN_LABEL_FONT * scale;
    let label_gap = PIN_LABEL_GAP * scale;
    let section_label_font = SECTION_LABEL_FONT * scale;
    let row_gap = PIN_ROW_GAP * scale;
    let avail_w = panel.w - pad * 2.0;
    let cols = ((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize;
    let cols = cols.max(1);
    let cell_h = tile_size + label_gap + label_font;
    let row_top = top_y + PIN_ROW_TOP_MARGIN * scale + section_label_font + label_gap;
    let rows = num_pins.div_ceil(cols);
    row_top + rows as f32 * cell_h + (rows.saturating_sub(1)) as f32 * row_gap
}

/// White text — user prefers white over the Studio tan everywhere.
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
/// Accent gold #C8860A.
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
#[allow(dead_code)]
const TILE_BG_ALPHA: f32 = 0.10;
#[allow(dead_code)]
const TILE_BORDER_ALPHA: f32 = 0.06;
const SECTION_LABEL_ALPHA: f32 = 0.55;
pub const SECTION_LABEL_FONT: f32 = 14.0;

fn text_color(alpha: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(alpha)
}
fn accent_color(alpha: f32) -> Color {
    Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha)
}

/// Draw the pinned-favorites row inside the panel. `top_y` is the
/// physical-pixel y at which the section starts (just below the search
/// underline). Pushes one `IconRequest` per visible pin into `icons`.
/// Returns the y-coordinate where this section ends.
///
/// `selected_pin` highlights one tile (e.g. for keyboard navigation /
/// Enter-to-launch).
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    launcher: &Launcher,
    apps: &AppsProvider,
    selected_pin: Option<usize>,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = input::SEARCH_HORIZONTAL_PAD * scale;
    let tile_size = PIN_TILE_SIZE * scale;
    let tile_gap = PIN_TILE_GAP * scale;
    let label_font = PIN_LABEL_FONT * scale;
    let label_gap = PIN_LABEL_GAP * scale;
    let section_label_font = SECTION_LABEL_FONT * scale;

    let entries = launcher.pinned_items(apps);
    if entries.is_empty() {
        return top_y;
    }

    let mut y = top_y + PIN_ROW_TOP_MARGIN * scale;

    // Section heading: "Pinned"
    text.queue(
        "Pinned",
        section_label_font,
        panel.x + pad,
        y,
        text_color(SECTION_LABEL_ALPHA * alpha),
        panel.w - pad * 2.0,
        surface_w,
        surface_h,
    );
    y += section_label_font + label_gap;

    // Tile grid. Compute columns from available width so pins wrap
    // onto additional rows instead of being clipped at the edge.
    let row_gap = PIN_ROW_GAP * scale;
    let avail_w = panel.w - pad * 2.0;
    let cols = ((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize;
    let cols = cols.max(1);
    let cell_h = tile_size + label_gap + label_font;
    let row_top = y;
    for (i, entry) in entries.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = panel.x + pad + col as f32 * (tile_size + tile_gap);
        let y = row_top + row as f32 * (cell_h + row_gap);

        let tile_rect = Rect::new(x, y, tile_size, tile_size);
        let is_selected = selected_pin == Some(i);

        // No background plate — icons sit directly on the panel. Only
        // the selected tile gets a soft accent ring so keyboard nav
        // stays visible.
        if is_selected {
            painter.rect_stroke_sdf(
                tile_rect,
                16.0 * scale,
                2.0 * scale,
                accent_color(0.55 * alpha),
            );
        }

        // Defer the icon to the texture pass. Larger icon (smaller
        // inset) since there's no plate framing it anymore.
        let inset = tile_size * 0.13;
        let (cache_key, icon_name) = match entry {
            PinnedItem::App(e) => (e.app_id.clone(), e.icon_name.clone()),
            PinnedItem::Path { path, is_dir } => {
                let (k, n) = path_icon(path, *is_dir);
                (k, Some(n))
            }
        };
        icons.push(IconRequest {
            app_id: cache_key,
            icon_name,
            x: x + inset,
            y: y + inset,
            size: tile_size - inset * 2.0,
            opacity: alpha,
            clip: None,
        });

        // Label below tile — truncated name.
        let label_text = truncate(&entry.label(), 12);
        let label_w = text.measure_width(&label_text, label_font);
        text.queue(
            &label_text,
            label_font,
            x + (tile_size - label_w) / 2.0,
            y + tile_size + label_gap,
            text_color(SECONDARY_LABEL_ALPHA * alpha),
            tile_size + tile_gap,
            surface_w,
            surface_h,
        );
    }

    let rows_used = entries.len().div_ceil(cols);
    row_top + rows_used as f32 * cell_h + (rows_used.saturating_sub(1)) as f32 * row_gap
}

const SECONDARY_LABEL_ALPHA: f32 = 0.85;

/// Truncate a string to at most `max_chars` chars, appending an ellipsis
/// if truncation happens. Operates on Unicode code points, not bytes.
fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("Firefox", 12), "Firefox");
    }

    #[test]
    fn truncate_long_string_ellipsised() {
        assert_eq!(truncate("Visual Studio Code", 12), "Visual Stud…");
    }

    #[test]
    fn truncate_unicode_safe() {
        assert_eq!(truncate("café-naïve-thing", 8), "café-na…");
    }
}

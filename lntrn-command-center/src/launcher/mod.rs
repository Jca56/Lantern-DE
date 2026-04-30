//! Launcher — pinned favorites + (Phase 2.5) result grid icons.
//!
//! When the search query is empty, we draw a row of pinned app tiles
//! beneath the search input. As the user types, the search/results
//! module takes over the same vertical space.

pub mod context_menu;
pub mod hidden;
pub mod icons;
pub mod pins;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::search::input;
use self::hidden::Hidden;
use self::pins::Pins;

pub struct Launcher {
    pins: Pins,
    hidden: Hidden,
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
    /// in the rendered row.
    pub fn pinned_entries<'a>(&'a self, apps: &'a AppsProvider) -> Vec<&'a DesktopEntry> {
        self.pins
            .items()
            .iter()
            .filter_map(|id| {
                // Linear scan; pin counts are tiny (typically <16) so it's fine.
                (0..apps.count())
                    .filter_map(|i| apps.get(i))
                    .find(|e| &e.app_id == id)
            })
            .collect()
    }

    /// Toggle pin state for an app_id.
    #[allow(dead_code)] // wired up by right-click handler
    pub fn toggle_pin(&mut self, app_id: &str) {
        let now_pinned = self.pins.toggle(app_id);
        tracing::info!(app_id, now_pinned, "pin toggled");
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

/// Total vertical space the pinned row needs (logical px), including
/// the top margin from the search underline.
#[allow(dead_code)] // exported for future layout calculations (recents row, etc.)
pub const PIN_ROW_HEIGHT: f32 = PIN_ROW_TOP_MARGIN + PIN_TILE_SIZE + PIN_LABEL_GAP + PIN_LABEL_FONT;

/// White text — user prefers white over the Studio tan everywhere.
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
/// Accent gold #C8860A.
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
#[allow(dead_code)]
const TILE_BG_ALPHA: f32 = 0.10;
#[allow(dead_code)]
const TILE_BORDER_ALPHA: f32 = 0.06;
const SECTION_LABEL_ALPHA: f32 = 0.55;
const SECTION_LABEL_FONT: f32 = 14.0;

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

    let entries = launcher.pinned_entries(apps);
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

    // Tile row.
    let mut x = panel.x + pad;
    let max_x = panel.x + panel.w - pad;
    for (i, entry) in entries.iter().enumerate() {
        if x + tile_size > max_x {
            break;
        }

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
        let inset = tile_size * 0.04;
        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: x + inset,
            y: y + inset,
            size: tile_size - inset * 2.0,
            opacity: alpha,
            clip: None,
        });

        // Label below tile — truncated app name.
        let label_text = truncate(&entry.name, 12);
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

        x += tile_size + tile_gap;
    }

    y + tile_size + label_gap + label_font
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

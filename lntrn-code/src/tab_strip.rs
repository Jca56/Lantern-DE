//! Browser-style tab strip drawn directly below the title bar.
//! Click handling is wired through `InteractionContext` zones; `main.rs` reads
//! the zone IDs to switch / close tabs.

use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::title_bar::TITLE_BAR_H;

/// Lightweight per-tab info passed to the strip renderer. Cloned each frame
/// so the strip code doesn't have to borrow the editor list.
#[derive(Clone)]
pub struct TabLabel {
    pub name: String,
    pub modified: bool,
}

/// Height of the tab strip in logical pixels.
pub const TAB_STRIP_H: f32 = 38.0;

/// Tab sizing.
const TAB_MIN_W: f32 = 110.0;
const TAB_MAX_W: f32 = 220.0;
const TAB_PAD_X: f32 = 12.0;
const CLOSE_BTN_W: f32 = 20.0;
const NEW_TAB_BTN_W: f32 = 32.0;
const FONT: f32 = 18.0;

/// Hit-zone IDs. Tab clicks use `ZONE_TAB_BASE + index`, close buttons use
/// `ZONE_TAB_CLOSE_BASE + index`. The new-tab button has its own ID.
pub const ZONE_NEW_TAB: u32 = 199;
pub const ZONE_TAB_BASE: u32 = 1000;
pub const ZONE_TAB_CLOSE_BASE: u32 = 2000;

/// State for an in-progress tab drag-to-reorder gesture.
pub struct TabDragState {
    /// Index of the tab being dragged (updated on each swap).
    pub idx: usize,
    /// Cursor x when the drag started.
    pub start_cx: f32,
    /// True once the cursor has moved past the dead-zone threshold.
    pub active: bool,
}

/// Returns the y origin of the tab strip in physical pixels. The strip lives
/// directly below the title bar, just above the editor area.
pub fn tab_strip_y(s: f32) -> f32 {
    TITLE_BAR_H * s
}

fn display_label(tab: &TabLabel) -> String {
    let mut name = tab.name.clone();
    if name.chars().count() > 28 {
        let truncated: String = name.chars().take(25).collect();
        name = format!("{}…", truncated);
    }
    if tab.modified {
        format!("{} •", name)
    } else {
        name
    }
}

/// Draw the tab strip and register hit zones. Returns the cumulative x
/// positions of each tab edge (length = tabs.len()), used by drag-reorder.
pub fn draw_tab_strip(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    tabs: &[TabLabel],
    active: usize,
    tab_drag: &Option<TabDragState>,
    palette: &FoxPalette,
    wf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> Vec<f32> {
    let y = tab_strip_y(s);
    let h = TAB_STRIP_H * s;

    // ── Strip background ──────────────────────────────────────────────
    // Transparent — the window bg shows through behind the tabs.
    painter.rect_filled(Rect::new(0.0, y, wf, h), 0.0, Color::TRANSPARENT);

    // Hairline below the strip (separates from toolbar).
    painter.line(
        0.0,
        y + h,
        wf,
        y + h,
        1.0 * s,
        Color::from_rgba8(60, 50, 35, 38),
    );

    let pad = TAB_PAD_X * s;
    let close_w = CLOSE_BTN_W * s;
    let font_px = FONT * s;

    let dragging = tab_drag.as_ref().map_or(false, |d| d.active);

    let mut x = 0.0;
    let mut tab_edges: Vec<f32> = Vec::with_capacity(tabs.len());
    for (i, tab) in tabs.iter().enumerate() {
        let label = display_label(tab);
        let label_w = text.measure_width(&label, font_px);
        let tab_w = (label_w + pad * 2.0 + close_w)
            .clamp(TAB_MIN_W * s, TAB_MAX_W * s);
        let tab_r = Rect::new(x, y, tab_w, h);

        let zone = input.add_zone(ZONE_TAB_BASE + i as u32, tab_r);
        let hovered = zone.is_hovered();
        let is_active = i == active;

        // ── Background fill ──────────────────────────────────────
        let bg = if is_active {
            palette.bg
        } else if hovered {
            Color::from_rgba8(255, 255, 255, 50)
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(tab_r, 0.0, bg);

        // Active tab: accent stripe at top, cover bottom hairline so the
        // active tab visually flows into the toolbar below.
        if is_active {
            painter.rect_filled(
                Rect::new(tab_r.x, tab_r.y, tab_r.w, 2.0 * s),
                0.0,
                palette.accent,
            );
            painter.rect_filled(
                Rect::new(tab_r.x, tab_r.y + tab_r.h - 1.0 * s, tab_r.w, 1.0 * s),
                0.0,
                palette.bg,
            );
        }

        // Right-edge separator between inactive tabs.
        if !is_active && i + 1 < tabs.len() {
            painter.rect_filled(
                Rect::new(
                    tab_r.x + tab_r.w - 1.0 * s,
                    tab_r.y + 6.0 * s,
                    1.0 * s,
                    h - 12.0 * s,
                ),
                0.0,
                Color::from_rgba8(60, 50, 35, 30),
            );
        }

        // ── Label text ───────────────────────────────────────────
        let label_color = if is_active {
            palette.text
        } else {
            palette.text_secondary
        };
        let label_x = tab_r.x + pad;
        let label_y = tab_r.y + (h - font_px) * 0.5;
        text.queue_styled(
            &label, font_px, label_x, label_y, label_color,
            tab_r.w - pad * 2.0 - close_w, FontWeight::Normal, FontStyle::Normal, sw, sh,
        );

        // ── Close button (visible on hover or active, hidden during drag) ─
        let close_r = Rect::new(
            tab_r.x + tab_r.w - close_w - pad * 0.4,
            tab_r.y + (h - close_w) * 0.5,
            close_w,
            close_w,
        );
        if (hovered || is_active) && !dragging {
            let close_zone = input.add_zone(ZONE_TAB_CLOSE_BASE + i as u32, close_r);
            let close_hovered = close_zone.is_hovered();
            if close_hovered {
                painter.rect_filled(
                    close_r,
                    4.0 * s,
                    Color::from_rgba8(204, 78, 60, 230),
                );
            }
            let icon_color = if close_hovered {
                Color::WHITE
            } else {
                palette.text_secondary
            };
            let cx = close_r.center_x();
            let cy = close_r.center_y();
            let half = 5.0 * s;
            painter.line(
                cx - half, cy - half, cx + half, cy + half, 1.5 * s, icon_color,
            );
            painter.line(
                cx + half, cy - half, cx - half, cy + half, 1.5 * s, icon_color,
            );
        }

        x += tab_w;
        tab_edges.push(x);
    }

    // ── New-tab "+" button ────────────────────────────────────────────
    let new_w = NEW_TAB_BTN_W * s;
    let new_r = Rect::new(x, y, new_w, h);
    let new_zone = input.add_zone(ZONE_NEW_TAB, new_r);
    if new_zone.is_hovered() {
        painter.rect_filled(new_r, 0.0, Color::from_rgba8(255, 255, 255, 50));
    }
    let plus_color = palette.text_secondary;
    let plus_cx = new_r.center_x();
    let plus_cy = new_r.center_y();
    let plus_half = 6.0 * s;
    painter.rect_filled(
        Rect::new(plus_cx - plus_half, plus_cy - 0.75 * s, plus_half * 2.0, 1.5 * s),
        0.0,
        plus_color,
    );
    painter.rect_filled(
        Rect::new(plus_cx - 0.75 * s, plus_cy - plus_half, 1.5 * s, plus_half * 2.0),
        0.0,
        plus_color,
    );

    tab_edges
}

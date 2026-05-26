//! Lock Screen → Style panel.
//!
//! Visual customization for the lock screen's password UI: field border color
//! and thickness, field fill color and opacity, password dot color, and the
//! scrim that darkens the wallpaper. All knobs write to `[lockscreen]` in
//! lantern.toml (the lock screen binary reads them live).
//!
//! Mirrors the appearance panel's card + labeled-slider + color-swatch layout
//! (see `appearance_focus.rs` / `panels.rs::draw_color_swatch_row`).

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, Slider};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, fit_swatches, slider_value_from_cursor,
    CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V, CARD_GAP,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
    LABEL_SIZE, LABEL_W, ROW_H, SLIDER_H, SLIDER_W, VALUE_SIZE, VALUE_W,
};

// ── Zone IDs (750+ block — distinct from wallpaper 700/710, sidebar 200+) ────

const ZONE_BORDER_SWATCH_BASE: u32 = 750; // + index into BORDER_SWATCHES
const ZONE_BORDER_THICKNESS: u32 = 770;
const ZONE_FIELD_SWATCH_BASE: u32 = 780; // + index into FIELD_SWATCHES
const ZONE_FIELD_OPACITY: u32 = 800;
const ZONE_DOT_SWATCH_BASE: u32 = 810; // + index into DOT_SWATCHES
const ZONE_SCRIM_OPACITY: u32 = 830;

// ── Slider ranges ────────────────────────────────────────────────────────────

const BORDER_THICKNESS_MAX: f32 = 8.0;

// ── Swatch palettes ───────────────────────────────────────────────────────────
//
// Each entry is `(hex, label)`. An empty hex ("") means "use the theme accent"
// — only the Border row offers it, and it highlights when the stored value is
// the empty string.

const BORDER_SWATCHES: &[(&str, &str)] = &[
    ("",        "Accent"),
    ("#FFFFFF", "White"),
    ("#000000", "Black"),
    ("#2563EB", "Blue"),
    ("#15803D", "Green"),
    ("#8B2DEB", "Purple"),
    ("#DC2626", "Red"),
];

const FIELD_SWATCHES: &[(&str, &str)] = &[
    ("#000000", "Black"),
    ("#FFFFFF", "White"),
    ("#1A1A1A", "Dark Gray"),
    ("#0F1F5C", "Navy"),
    ("#2C0F5C", "Purple"),
    ("#0F3D24", "Green"),
];

const DOT_SWATCHES: &[(&str, &str)] = &[
    ("#F5F5F5", "White"),
    ("#000000", "Black"),
    ("",        "Accent"),
    ("#2563EB", "Blue"),
    ("#15803D", "Green"),
    ("#DC2626", "Red"),
    ("#FFC800", "Gold"),
];

/// Lock Style panel state. The sliders drag live during draw (via the shared
/// interaction context), so this only needs scroll position.
pub struct LockStyleState {
    pub scroll_offset: f32,
}

impl LockStyleState {
    pub fn new() -> Self {
        Self { scroll_offset: 0.0 }
    }
}

// ── Generic swatch row (custom palette; empty hex = "Accent") ─────────────────

/// Draw a labeled row of circular color swatches from `palette`. Zone ids run
/// `zone_base + 0..palette.len()`. The currently-selected swatch (hex compared
/// case-insensitively, empty matches empty) gets a ring. Returns nothing —
/// click resolution is handled by `handle_lock_style_click`.
#[allow(clippy::too_many_arguments)]
fn draw_swatch_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    label: &str,
    palette: &[(&str, &str)],
    zone_base: u32,
    selected_hex: &str,
    label_x: f32,
    ctrl_x: f32,
    end_x: f32,
    cy: &mut f32,
    row: f32,
    lsz: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

    let (swatch_size, swatch_gap) = fit_swatches(palette.len(), ctrl_x, end_x, s);
    let mut sx = ctrl_x;
    for (i, (hex, _name)) in palette.iter().enumerate() {
        // Empty hex = "use theme accent" — render the accent color.
        let color = if hex.is_empty() {
            fox.accent
        } else {
            Color::from_hex(hex).unwrap_or(fox.text)
        };
        let zone_id = zone_base + i as u32;
        let swatch_rect = Rect::new(sx, *cy + (row - swatch_size) / 2.0, swatch_size, swatch_size);
        let zone = ix.add_zone(zone_id, swatch_rect);

        let cx = sx + swatch_size / 2.0;
        let cy_center = swatch_rect.y + swatch_size / 2.0;
        let radius = swatch_size / 2.0;
        painter.circle_filled(cx, cy_center, radius, color);

        let is_selected = if hex.is_empty() {
            selected_hex.is_empty()
        } else {
            selected_hex.eq_ignore_ascii_case(hex)
        };
        if is_selected {
            painter.circle_stroke(cx, cy_center, radius + 3.0 * s, 2.0 * s, fox.text);
        } else if zone.is_hovered() {
            painter.circle_stroke(cx, cy_center, radius + 2.0 * s, 1.5 * s, fox.text_secondary);
        }
        sx += swatch_size + swatch_gap;
    }
    *cy += row;
}

/// Draw a labeled slider row. `frac` is the 0..1 fill; the value text is shown
/// to the right. Returns the dragged 0..1 fraction if the user is dragging
/// this slider this frame (caller maps it back into the real value range).
#[allow(clippy::too_many_arguments)]
fn draw_slider_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    label: &str,
    zone_id: u32,
    frac: f32,
    value_str: &str,
    label_x: f32,
    ctrl_x: f32,
    ctrl_w: f32,
    value_x: f32,
    cy: &mut f32,
    row: f32,
    lsz: f32,
    vsz: f32,
    slider_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> Option<f32> {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

    let rect = Rect::new(ctrl_x, *cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
    let zone = ix.add_zone(zone_id, rect);
    let dragged = slider_value_from_cursor(ix, zone_id, &rect);

    Slider::new(rect)
        .value(frac)
        .hovered(zone.is_hovered())
        .active(zone.is_active())
        .draw(painter, fox);
    text.queue(value_str, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);

    *cy += row;
    dragged
}

// ── Draw ───────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_lock_style_panel(
    config: &mut LanternConfig,
    lss: &mut LockStyleState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    s: f32,
    sw: u32,
    sh: u32,
    scroll_delta: f32,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    // ── Card geometry ──────────────────────────────────────────────
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let end_x = card_inner_x + card_inner_w; // inner-right edge for swatches
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;
    let border_card_h = card_chrome_h + row * 2.0;
    let field_card_h = card_chrome_h + row * 2.0;
    let dots_card_h = card_chrome_h + row;
    let scrim_card_h = card_chrome_h + row + row * 0.7; // slider + helper line
    let gap = CARD_GAP * s;

    let mut content_height = CARD_OUTER_PAD_V * 2.0 * s;
    content_height += border_card_h + gap + field_card_h + gap + dots_card_h + gap + scrim_card_h;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(&mut lss.scroll_offset, scroll_delta * 20.0, content_height, h);
    }

    let viewport = Rect::new(x, y, w, h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut lss.scroll_offset);
    scroll_area.begin(painter, text);

    let mut card_y = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ── Border card ────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Border",
            card_x, card_y, card_w, border_card_h, s, sw, sh,
        );
        draw_swatch_row(
            painter, text, ix, fox, "Color", BORDER_SWATCHES, ZONE_BORDER_SWATCH_BASE,
            &config.lockscreen.border_color, label_x, ctrl_x, end_x, &mut cy, row, lsz, s, sw, sh,
        );
        let frac = (config.lockscreen.border_thickness / BORDER_THICKNESS_MAX).clamp(0.0, 1.0);
        let val = format!("{} px", config.lockscreen.border_thickness.round() as i32);
        if let Some(f) = draw_slider_row(
            painter, text, ix, fox, "Thickness", ZONE_BORDER_THICKNESS, frac, &val,
            label_x, ctrl_x, ctrl_w, value_x, &mut cy, row, lsz, vsz, slider_h, s, sw, sh,
        ) {
            config.lockscreen.border_thickness = (f * BORDER_THICKNESS_MAX).round();
        }
        card_y += border_card_h + gap;
    }

    // ── Field card ─────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Field",
            card_x, card_y, card_w, field_card_h, s, sw, sh,
        );
        draw_swatch_row(
            painter, text, ix, fox, "Color", FIELD_SWATCHES, ZONE_FIELD_SWATCH_BASE,
            &config.lockscreen.field_color, label_x, ctrl_x, end_x, &mut cy, row, lsz, s, sw, sh,
        );
        let frac = config.lockscreen.field_opacity.clamp(0.0, 1.0);
        let val = format!("{}%", (frac * 100.0).round() as i32);
        if let Some(f) = draw_slider_row(
            painter, text, ix, fox, "Opacity", ZONE_FIELD_OPACITY, frac, &val,
            label_x, ctrl_x, ctrl_w, value_x, &mut cy, row, lsz, vsz, slider_h, s, sw, sh,
        ) {
            config.lockscreen.field_opacity = (f * 100.0).round() / 100.0;
        }
        card_y += field_card_h + gap;
    }

    // ── Dots card ──────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Dots",
            card_x, card_y, card_w, dots_card_h, s, sw, sh,
        );
        draw_swatch_row(
            painter, text, ix, fox, "Color", DOT_SWATCHES, ZONE_DOT_SWATCH_BASE,
            &config.lockscreen.dot_color, label_x, ctrl_x, end_x, &mut cy, row, lsz, s, sw, sh,
        );
        card_y += dots_card_h + gap;
    }

    // ── Scrim card ─────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Scrim",
            card_x, card_y, card_w, scrim_card_h, s, sw, sh,
        );
        let frac = config.lockscreen.scrim_opacity.clamp(0.0, 1.0);
        let val = format!("{}%", (frac * 100.0).round() as i32);
        if let Some(f) = draw_slider_row(
            painter, text, ix, fox, "Opacity", ZONE_SCRIM_OPACITY, frac, &val,
            label_x, ctrl_x, ctrl_w, value_x, &mut cy, row, lsz, vsz, slider_h, s, sw, sh,
        ) {
            config.lockscreen.scrim_opacity = (f * 100.0).round() / 100.0;
        }
        let help_y = cy + (row * 0.7 - lsz) / 2.0;
        text.queue(
            "Darkening over the wallpaper", lsz * 0.85, label_x, help_y,
            fox.text_secondary, card_inner_w, sw, sh,
        );
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, lss.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

// ── Click handling (swatch selection; sliders drag live during draw) ──────────

pub fn handle_lock_style_click(
    config: &mut LanternConfig,
    zone_id: u32,
) {
    let n_border = BORDER_SWATCHES.len() as u32;
    let n_field = FIELD_SWATCHES.len() as u32;
    let n_dot = DOT_SWATCHES.len() as u32;

    if zone_id >= ZONE_BORDER_SWATCH_BASE && zone_id < ZONE_BORDER_SWATCH_BASE + n_border {
        let idx = (zone_id - ZONE_BORDER_SWATCH_BASE) as usize;
        config.lockscreen.border_color = BORDER_SWATCHES[idx].0.to_string();
    } else if zone_id >= ZONE_FIELD_SWATCH_BASE && zone_id < ZONE_FIELD_SWATCH_BASE + n_field {
        let idx = (zone_id - ZONE_FIELD_SWATCH_BASE) as usize;
        config.lockscreen.field_color = FIELD_SWATCHES[idx].0.to_string();
    } else if zone_id >= ZONE_DOT_SWATCH_BASE && zone_id < ZONE_DOT_SWATCH_BASE + n_dot {
        let idx = (zone_id - ZONE_DOT_SWATCH_BASE) as usize;
        config.lockscreen.dot_color = DOT_SWATCHES[idx].0.to_string();
    }
    // Slider zones (ZONE_*_THICKNESS / *_OPACITY) are handled live in draw.
}

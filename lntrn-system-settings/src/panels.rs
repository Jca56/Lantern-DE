//! Shared infrastructure used by every settings panel: layout constants,
//! the GLOW_COLORS accent palette, dropdown menu state, helper draw fns
//! (sliders/swatches/select buttons/section cards) and the Save/Cancel bar.
//!
//! Per-panel rendering and click handling lives in:
//!   - `appearance_panel.rs` — theme + accent + window frame + effects +
//!                             animations + focus/glow (with sibling section
//!                             modules `appearance_layout`,
//!                             `appearance_animations`, `appearance_focus`)
//!   - `display_panel.rs`    — wallpaper + scaling
//!   - `input_panel.rs`      — mouse/keyboard
//!   - `power_panel.rs`      — battery + idle
//!   - `notifications_panel.rs` — toasts + sound

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext, MenuItem,
};

pub const ZONE_SAVE: u32 = 900;
pub const ZONE_CANCEL: u32 = 901;

pub(crate) const ROW_H: f32 = 48.0;
pub(crate) const LABEL_SIZE: f32 = 18.0;
pub(crate) const VALUE_SIZE: f32 = 16.0;
pub(crate) const SLIDER_H: f32 = 36.0;
pub(crate) const SLIDER_W: f32 = 320.0;
pub(crate) const TOGGLE_H: f32 = 36.0;
pub(crate) const PAD_RIGHT: f32 = 32.0;
pub(crate) const LABEL_W: f32 = 200.0;
pub(crate) const VALUE_W: f32 = 60.0;

pub(crate) const GLOW_COLORS: &[(&str, &str)] = &[
    ("#0A0A0A", "Black"),
    ("#FFFFFF", "White"),
    ("#2563EB", "Blue"),
    ("#8B2DEB", "Purple"),
    ("#DB2777", "Pink"),
    ("#0891B2", "Cyan"),
    ("#15803D", "Green"),
    ("#EA580C", "Orange"),
    ("#DC2626", "Red"),
    ("#FFC800", "Gold"),
];

/// Background-color swatches for the Background Color picker. Punchier
/// chromatic tints so the user can actually tell them apart at a glance —
/// still dark enough to function as window/panel backgrounds. The hex
/// values are what gets written to `appearance.background_color` directly.
pub(crate) const BG_COLORS: &[(&str, &str)] = &[
    ("#0E0E0E", "Black"),
    ("#4A2814", "Brown"),
    ("#0F1F5C", "Blue"),
    ("#2C0F5C", "Purple"),
    ("#5C1230", "Pink"),
    ("#0F3358", "Cyan"),
    ("#0F3D24", "Green"),
    ("#4A2608", "Orange"),
    ("#4A0F0F", "Red"),
    ("#4A350A", "Gold"),
];

// ── Shared card layout constants (used by WM, Input, etc.) ──────────────────

pub(crate) const CARD_OUTER_PAD_H: f32 = 36.0; // gutter on left/right of cards
pub(crate) const CARD_OUTER_PAD_V: f32 = 16.0; // top/bottom padding of scroll content
pub(crate) const CARD_INNER_PAD_H: f32 = 24.0; // horizontal padding inside a card
pub(crate) const CARD_INNER_PAD_V: f32 = 16.0; // vertical padding inside a card
pub(crate) const CARD_HEADER_H: f32 = 36.0;    // section header strip height
pub(crate) const CARD_GAP: f32 = 28.0;         // gap between cards
const CARD_RADIUS: f32 = 12.0;
const SECTION_HEADER_SZ: f32 = 18.0;

// ── Panel state ─────────────────────────────────────────────────────────────

pub struct PanelState {
    pub dropdown_menu: ContextMenu,
    pub active_dropdown: Option<u32>,
    pub scroll_offset: f32,
    pub wm_scroll: f32,
}

impl PanelState {
    pub fn new(fox: &FoxPalette) -> Self {
        Self {
            dropdown_menu: ContextMenu::new(ContextMenuStyle::from_palette(fox)),
            active_dropdown: None,
            scroll_offset: 0.0,
            wm_scroll: 0.0,
        }
    }

    pub fn close_dropdown(&mut self) {
        self.dropdown_menu.close();
        self.active_dropdown = None;
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn slider_value_from_cursor(
    ix: &InteractionContext, zone_id: u32, rect: &Rect,
) -> Option<f32> {
    let state = ix.zone_state(zone_id);
    if state.is_active() {
        if let Some((cx, _)) = ix.cursor() {
            return Some(((cx - rect.x) / rect.w).clamp(0.0, 1.0));
        }
    }
    None
}

/// Render a labeled row of color-swatch picker buttons using the shared
/// `GLOW_COLORS` palette. Zone IDs run `zone_base + 0..GLOW_COLORS.len()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_color_swatch_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    label: &str,
    zone_base: u32,
    selected_hex: &str,
    label_x: f32,
    ctrl_x: f32,
    cy: &mut f32,
    row: f32,
    lsz: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);

    let swatch_size = 28.0 * s;
    let swatch_gap = 8.0 * s;
    let mut sx = ctrl_x;
    for (i, (hex, _name)) in GLOW_COLORS.iter().enumerate() {
        let color = Color::from_hex(hex).unwrap();
        let zone_id = zone_base + i as u32;
        let swatch_rect = Rect::new(sx, *cy + (row - swatch_size) / 2.0, swatch_size, swatch_size);
        let zone = ix.add_zone(zone_id, swatch_rect);

        let cx = sx + swatch_size / 2.0;
        let cy_center = swatch_rect.y + swatch_size / 2.0;
        let radius = swatch_size / 2.0;
        painter.circle_filled(cx, cy_center, radius, color);

        let is_selected = selected_hex.eq_ignore_ascii_case(hex);
        if is_selected {
            painter.circle_stroke(cx, cy_center, radius + 3.0 * s, 2.0 * s, fox.text);
        } else if zone.is_hovered() {
            painter.circle_stroke(cx, cy_center, radius + 2.0 * s, 1.5 * s, fox.text_secondary);
        }
        sx += swatch_size + swatch_gap;
    }
    *cy += row;
}

/// Returns true if the rect at (text_x, row_y, text_w, row_h) significantly
/// overlaps the menu. Margin ignores shadow/padding overlap at the edges.
pub(crate) fn hidden_by_menu(text_x: f32, row_y: f32, text_w: f32, row_h: f32, menu: &ContextMenu) -> bool {
    if !menu.is_open() { return false; }
    if let Some(b) = menu.bounds() {
        let margin = 8.0;
        let mx = b.x + margin;
        let my = b.y + margin;
        let mw = (b.w - margin * 2.0).max(0.0);
        let mh = (b.h - margin * 2.0).max(0.0);
        let overlaps_y = row_y < (my + mh) && (row_y + row_h) > my;
        let overlaps_x = text_x < (mx + mw) && (text_x + text_w) > mx;
        overlaps_x && overlaps_y
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_select_button(
    label: &str, current: &str,
    zone_id: u32, is_open: bool,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette,
    label_x: f32, label_w: f32, btn_x: f32, btn_w: f32, btn_h: f32,
    row: f32, lsz: f32, s: f32, sw: u32, sh: u32,
    cy: &mut f32, menu: &ContextMenu,
) {
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, label_w, sw, sh);

    let rect = Rect::new(btn_x, *cy + (row - btn_h) / 2.0, btn_w, btn_h);
    let zone = ix.add_zone(zone_id, rect);

    let bg = if is_open || zone.is_hovered() { fox.surface_2 } else { fox.surface };
    let r = 6.0 * s;
    painter.rect_filled(rect, r, bg);
    painter.rect_stroke_sdf(rect, r, 1.0 * s, fox.muted.with_alpha(0.3));

    let font = 18.0 * s;
    let pad_h = 14.0 * s;

    let skip_text = hidden_by_menu(btn_x, *cy, btn_w, row, menu);
    if !skip_text {
        let text_y = rect.y + (rect.h - font) / 2.0;
        let display: String = current.chars().take(1).flat_map(|c| c.to_uppercase())
            .chain(current.chars().skip(1)).collect();
        text.queue(&display, font, rect.x + pad_h, text_y, fox.text,
            btn_w - pad_h * 2.0 - 12.0 * s, sw, sh);
    }

    let chev_s = 8.0 * s;
    let chev_x = rect.x + rect.w - pad_h - chev_s;
    let chev_cy = rect.y + rect.h * 0.5;
    let chev_c = fox.text_secondary;
    if is_open {
        painter.line(chev_x, chev_cy + chev_s * 0.35, chev_x + chev_s * 0.5, chev_cy - chev_s * 0.35, 1.5 * s, chev_c);
        painter.line(chev_x + chev_s * 0.5, chev_cy - chev_s * 0.35, chev_x + chev_s, chev_cy + chev_s * 0.35, 1.5 * s, chev_c);
    } else {
        painter.line(chev_x, chev_cy - chev_s * 0.35, chev_x + chev_s * 0.5, chev_cy + chev_s * 0.35, 1.5 * s, chev_c);
        painter.line(chev_x + chev_s * 0.5, chev_cy + chev_s * 0.35, chev_x + chev_s, chev_cy - chev_s * 0.35, 1.5 * s, chev_c);
    }

    *cy += row;
}

pub(crate) fn make_menu_items(options: &[&str], base_id: u32, current: &str) -> Vec<MenuItem> {
    options.iter().enumerate().map(|(i, opt)| {
        let selected = opt.to_lowercase() == current.to_lowercase();
        if selected {
            MenuItem::Action {
                id: base_id + i as u32,
                label: format!("• {}", opt),
                shortcut: None, enabled: true, danger: false,
            }
        } else {
            MenuItem::action(base_id + i as u32, *opt)
        }
    }).collect()
}

/// Draw a section card background + header. Returns the y of the first
/// content row inside the card.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_section_card(
    painter: &mut Painter, text: &mut TextRenderer,
    fox: &FoxPalette,
    label: &str,
    x: f32, y: f32, w: f32, h: f32,
    s: f32, sw: u32, sh: u32,
) -> f32 {
    let card_rect = Rect::new(x, y, w, h);
    painter.rect_filled(card_rect, CARD_RADIUS * s, fox.surface.with_alpha(0.45));
    painter.rect_stroke_sdf(
        card_rect, CARD_RADIUS * s, 1.0 * s,
        fox.muted.with_alpha(0.18),
    );
    let header_sz = SECTION_HEADER_SZ * s;
    let header_y = y + CARD_INNER_PAD_V * s;
    text.queue(
        label, header_sz,
        x + CARD_INNER_PAD_H * s, header_y,
        fox.accent, w - CARD_INNER_PAD_H * 2.0 * s, sw, sh,
    );
    let underline_w = label.len() as f32 * header_sz * 0.55;
    painter.rect_filled(
        Rect::new(
            x + CARD_INNER_PAD_H * s,
            header_y + header_sz + 4.0 * s,
            underline_w,
            2.0 * s,
        ),
        1.0 * s,
        fox.accent.with_alpha(0.6),
    );
    y + CARD_HEADER_H * s + CARD_INNER_PAD_V * s
}

// ── Save / Cancel bar ───────────────────────────────────────────────────────

use lntrn_ui::gpu::{Button, ButtonVariant};

pub fn draw_save_cancel_bar(
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, content_x: f32, w: f32, bottom_y: f32,
    s: f32, sw: u32, sh: u32,
) {
    let bar_h = 56.0 * s;
    let bar_y = bottom_y - bar_h;
    let pad = PAD_RIGHT * s;
    let btn_h = 38.0 * s;
    let btn_w = 100.0 * s;
    let gap = 12.0 * s;

    painter.rect_filled(
        Rect::new(content_x + 16.0 * s, bar_y, w - 32.0 * s, 1.0 * s),
        0.0,
        fox.muted.with_alpha(0.2),
    );

    let save_x = content_x + w - pad - btn_w;
    let save_y = bar_y + (bar_h - btn_h) / 2.0;
    let save_rect = Rect::new(save_x, save_y, btn_w, btn_h);
    let save_zone = ix.add_zone(ZONE_SAVE, save_rect);
    Button::new(save_rect, "Save")
        .variant(ButtonVariant::Primary)
        .hovered(save_zone.is_hovered())
        .pressed(save_zone.is_active())
        .scale(s)
        .draw(painter, text, fox, sw, sh);

    let cancel_x = save_x - gap - btn_w;
    let cancel_rect = Rect::new(cancel_x, save_y, btn_w, btn_h);
    let cancel_zone = ix.add_zone(ZONE_CANCEL, cancel_rect);
    Button::new(cancel_rect, "Cancel")
        .variant(ButtonVariant::Ghost)
        .hovered(cancel_zone.is_hovered())
        .pressed(cancel_zone.is_active())
        .scale(s)
        .draw(painter, text, fox, sw, sh);
}

//! Window Sizes card for the Appearance panel.
//!
//! Four percentage-of-work-area pickers: default new-window size + the
//! Small / Medium / Large rungs used by the Super+Shift+Up/Down ladder.
//! Each picker uses the shared `draw_select_button` + `dropdown_menu`.

use lntrn_render::{Painter, TextRenderer};
use lntrn_ui::gpu::FoxPalette;

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, draw_select_button, make_menu_items, PanelState,
    CARD_INNER_PAD_H, LABEL_SIZE, LABEL_W, ROW_H,
};

/// Preset percentages shown in the dropdowns. Strings rather than numbers so
/// the menu code can reuse `make_menu_items` and string-compare current.
pub(crate) const SIZE_PCT_OPTIONS: &[&str] = &[
    "20%", "30%", "40%", "50%", "60%", "70%", "80%", "90%", "95%",
];

pub(crate) struct WindowSizeZones {
    pub default_btn: u32,
    pub small_btn:   u32,
    pub medium_btn:  u32,
    pub large_btn:   u32,
}

/// Number of rows the card renders (used by the panel for sizing).
pub const ROWS: f32 = 4.0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_window_sizes_card(
    config: &LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut lntrn_ui::gpu::InteractionContext,
    fox: &FoxPalette,
    card_x: f32, card_y: f32, card_w: f32, card_h: f32,
    s: f32, sw: u32, sh: u32,
    z: &WindowSizeZones,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let label_w = LABEL_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;

    let mut cy = draw_section_card(
        painter, text, fox, "Window Sizes",
        card_x, card_y, card_w, card_h, s, sw, sh,
    );

    let btn_w = 200.0 * s;
    let btn_h = 40.0 * s;

    let draw_one = |label: &str, current: u32, zone_id: u32,
                    cy: &mut f32,
                    painter: &mut Painter, text: &mut TextRenderer,
                    ix: &mut lntrn_ui::gpu::InteractionContext,
                    panel_state: &PanelState| {
        let cur_label = format!("{}%", current);
        let is_open = panel_state.active_dropdown == Some(zone_id);
        let menu = &panel_state.dropdown_menu;
        draw_select_button(
            label, &cur_label,
            zone_id, is_open,
            painter, text, ix, fox,
            label_x, label_w, ctrl_x, btn_w, btn_h, row, lsz, s, sw, sh,
            cy, menu,
        );
    };

    draw_one("Default Size", config.windows.default_size_pct, z.default_btn,
             &mut cy, painter, text, ix, panel_state);
    draw_one("Small",        config.windows.size_small_pct,   z.small_btn,
             &mut cy, painter, text, ix, panel_state);
    draw_one("Medium",       config.windows.size_medium_pct,  z.medium_btn,
             &mut cy, painter, text, ix, panel_state);
    draw_one("Large",        config.windows.size_large_pct,   z.large_btn,
             &mut cy, painter, text, ix, panel_state);
}

/// Build menu items for the currently-open picker.
pub(crate) fn build_size_menu(base_id: u32, current_pct: u32) -> Vec<lntrn_ui::gpu::MenuItem> {
    let cur_label = format!("{}%", current_pct);
    make_menu_items(SIZE_PCT_OPTIONS, base_id, &cur_label)
}

/// Parse a label like `"60%"` back into a `u32` percentage. Returns 0 on
/// failure, which callers should treat as "ignore the click".
pub(crate) fn parse_pct(label: &str) -> u32 {
    label.trim_end_matches('%').parse::<u32>().unwrap_or(0)
}

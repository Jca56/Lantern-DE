//! Animations card for the Appearance panel.
//!
//! Master toggle, speed slider, preset dropdown (Cinematic/Snappy/Springy/Linear),
//! and per-event toggles (Open/Close, Maximize/Restore, Minimize, Tile/Snap,
//! Workspace Switch). The preset menu opens via `PanelState::dropdown_menu`;
//! the parent panel routes its action events back.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, Slider, Toggle};

use crate::appearance_panel::AnimZones;
use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, draw_select_button, slider_value_from_cursor, PanelState,
    CARD_INNER_PAD_H, LABEL_SIZE, LABEL_W, ROW_H, SLIDER_H, SLIDER_W,
    TOGGLE_H, VALUE_SIZE, VALUE_W,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_animations_card(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    card_x: f32, card_y: f32, card_w: f32, card_h: f32,
    s: f32, sw: u32, sh: u32,
    z: &AnimZones,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;
    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    let mut cy = draw_section_card(
        painter, text, fox, "Animations",
        card_x, card_y, card_w, card_h, s, sw, sh,
    );

    // Row 1: master toggle
    {
        let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.animations.enabled)
            .label("Enable Animations").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(z.enable, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
        cy += row;
    }

    // Row 2: speed slider
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Speed", lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let frac = ((config.animations.speed - 0.25) / 2.75).clamp(0.0, 1.0);
        let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(z.speed, rect);
        if let Some(f) = slider_value_from_cursor(ix, z.speed, &rect) {
            let raw = 0.25 + f * 2.75;
            config.animations.speed = ((raw / 0.05).round() * 0.05).clamp(0.25, 3.0);
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{:.2}x", config.animations.speed);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        cy += row;
    }

    // Row 3: preset picker (uses shared select-button + dropdown menu)
    {
        let btn_w = 240.0 * s;
        let btn_h = 40.0 * s;
        let is_open = panel_state.active_dropdown == Some(z.preset_btn);
        let menu = &panel_state.dropdown_menu;
        draw_select_button(
            "Preset", &config.animations.preset,
            z.preset_btn, is_open,
            painter, text, ix, fox,
            label_x, label_w, ctrl_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu,
        );
    }

    // Row 4: small label "Per-event"
    {
        let label_y = cy + (row * 0.75 - lsz) / 2.0;
        text.queue("Per-event", lsz, label_x, label_y, fox.text_secondary,
            card_inner_w, sw, sh);
        cy += row * 0.75;
    }

    // Rows 5-7: two-column grid of 5 toggles.
    let col_w = card_inner_w / 2.0;
    let toggle_h = TOGGLE_H * s;
    let toggle_row_h = row * 0.85;

    let mut place = |label: &str, value: bool, zone_id: u32, x: f32, y: f32| {
        let rect = Rect::new(x, y, col_w - 12.0 * s, toggle_h);
        let toggle = Toggle::new(rect, value).label(label).scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(zone_id, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
    };

    place("Window Open / Close", config.animations.open_close,
          z.t_open, card_inner_x, cy);
    place("Maximize / Restore", config.animations.state,
          z.t_state, card_inner_x + col_w, cy);
    cy += toggle_row_h;

    place("Minimize", config.animations.minimize,
          z.t_min, card_inner_x, cy);
    place("Tile / Snap", config.animations.tiling,
          z.t_tiling, card_inner_x + col_w, cy);
    cy += toggle_row_h;

    place("Workspace Switch", config.animations.workspace,
          z.t_ws, card_inner_x, cy);
}

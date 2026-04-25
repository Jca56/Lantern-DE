//! Power panel: lid behavior, idle, battery thresholds, and WiFi power.
//!
//! Three cards:
//! 1. **Lid & Idle** — Lid Close (Battery), Lid Close (AC), Dim Screen After,
//!    Idle Timeout, Idle Action.
//! 2. **Battery** — Low Battery Warning %, Critical Battery %, Critical Action.
//! 3. **WiFi Power** — WiFi Power Save toggle and Power Scheme dropdown.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuEvent, ScrollArea, Scrollbar, Slider, Toggle,
};

use crate::config::LanternConfig;
use crate::panels::{
    draw_section_card, draw_select_button, hidden_by_menu, make_menu_items,
    slider_value_from_cursor, PanelState,
    CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V,
};

// ── Zone IDs ────────────────────────────────────────────────────────────────

const ZONE_PWR_LID_BTN: u32 = 400;
const ZONE_PWR_LID_AC_BTN: u32 = 401;
const ZONE_PWR_DIM_SLIDER: u32 = 402;
const ZONE_PWR_IDLE_SLIDER: u32 = 403;
const ZONE_PWR_IDLE_ACT_BTN: u32 = 404;
const ZONE_PWR_LOW_BAT_SLIDER: u32 = 405;
const ZONE_PWR_CRIT_BAT_SLIDER: u32 = 406;
const ZONE_PWR_CRIT_BTN: u32 = 407;
const ZONE_PWR_WIFI_PS: u32 = 408;
const ZONE_PWR_WIFI_SCHEME_BTN: u32 = 409;

// Action ID base values for the dropdown menus.
const ACT_LID: u32 = 500;
const ACT_LID_AC: u32 = 510;
const ACT_IDLE: u32 = 520;
const ACT_CRIT: u32 = 530;
const ACT_WIFI_SCHEME: u32 = 540;

const LID_OPTIONS: &[&str] = &["Suspend", "Hibernate", "Lock", "Nothing"];
const IDLE_ACTION_OPTIONS: &[&str] = &["Suspend", "Lock", "Nothing"];
const CRIT_OPTIONS: &[&str] = &["Suspend", "Hibernate", "Shutdown", "Nothing"];
const WIFI_SCHEME_OPTIONS: &[&str] = &["Active", "Balanced", "Battery"];

// ── Layout constants ────────────────────────────────────────────────────────

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const SLIDER_W: f32 = 320.0;
const BTN_H: f32 = 42.0;
const TOGGLE_H: f32 = 36.0;
const LABEL_W: f32 = 200.0;
const VALUE_W: f32 = 60.0;

// ── Draw ────────────────────────────────────────────────────────────────────

pub fn draw_power_panel(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, x: f32, y: f32, w: f32, panel_h: f32,
    s: f32, sw: u32, sh: u32, scroll_delta: f32,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;
    let btn_h = BTN_H * s;

    // Card geometry
    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    // Inside-card row layout — fixed-width slider, flexible dropdown buttons
    let label_w = LABEL_W * s;
    let value_w = VALUE_W * s;
    let label_x = card_inner_x;
    let ctrl_x = card_inner_x + label_w;
    let avail = (card_inner_w - label_w - value_w - 12.0 * s).max(80.0 * s);
    let ctrl_w = (SLIDER_W * s).min(avail);
    let value_x = ctrl_x + ctrl_w + 8.0 * s;

    // Dropdown buttons take the full right side after the label
    let btn_x = card_inner_x + label_w;
    let btn_w = card_inner_w - label_w;

    // Card row counts
    let lid_idle_rows: f32 = 5.0; // Lid Bat, Lid AC, Dim, Idle Timeout, Idle Action
    let battery_rows: f32 = 3.0;  // Low, Critical %, Critical Action
    let wifi_rows: f32 = 2.0;     // WiFi Power Save, Power Scheme

    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;
    let lid_idle_card_h = card_chrome_h + lid_idle_rows * row;
    let battery_card_h = card_chrome_h + battery_rows * row;
    let wifi_card_h = card_chrome_h + wifi_rows * row;

    let content_height = CARD_OUTER_PAD_V * s
        + lid_idle_card_h + CARD_GAP * s
        + battery_card_h + CARD_GAP * s
        + wifi_card_h + CARD_OUTER_PAD_V * 2.0 * s;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut panel_state.scroll_offset, scroll_delta * 40.0,
            content_height, panel_h,
        );
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut panel_state.scroll_offset);
    scroll_area.begin(painter, text);

    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    let active = panel_state.active_dropdown;
    let menu = &panel_state.dropdown_menu;

    // Helper: should we skip this slider's value text? (Only when it overlaps
    // an open dropdown menu — painter shapes are fine since the menu draws
    // last and covers them.)
    let text_hidden = |tx: f32, ty: f32, tw: f32, th: f32| -> bool {
        hidden_by_menu(tx, ty, tw, th, menu)
    };

    // ─────────────────────────────────────────────────────────────────
    // Card 1: Lid & Idle
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Lid & Idle",
            card_x, cy_top, card_w, lid_idle_card_h, s, sw, sh,
        );

        // Row: Lid Close (Battery)
        draw_select_button("Lid Close (Battery)", &config.power.lid_close_action,
            ZONE_PWR_LID_BTN, active == Some(ZONE_PWR_LID_BTN),
            painter, text, ix, fox,
            label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);

        // Row: Lid Close (AC)
        draw_select_button("Lid Close (AC)", &config.power.lid_close_on_ac,
            ZONE_PWR_LID_AC_BTN, active == Some(ZONE_PWR_LID_AC_BTN),
            painter, text, ix, fox,
            label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);

        // Row: Dim Screen After (0–600s, snapped to 30s)
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Dim Screen After", lsz, label_x, label_y, fox.text, label_w, sw, sh);

            let frac = config.power.dim_after as f32 / 600.0;
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_PWR_DIM_SLIDER, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_DIM_SLIDER, &rect) {
                config.power.dim_after = (f * 600.0).round() as u32;
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            // Snap to nearest 30s
            config.power.dim_after = ((config.power.dim_after + 14) / 30) * 30;
            if !text_hidden(value_x, label_y, value_w, lsz) {
                let val = if config.power.dim_after == 0 {
                    "Never".to_string()
                } else {
                    let mins = config.power.dim_after / 60;
                    let secs = config.power.dim_after % 60;
                    if mins > 0 && secs > 0 { format!("{}m {}s", mins, secs) }
                    else if mins > 0 { format!("{}m", mins) }
                    else { format!("{}s", secs) }
                };
                text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            }
            cy += row;
        }

        // Row: Idle Timeout (60–1800s, snapped to 60s)
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Idle Timeout", lsz, label_x, label_y, fox.text, label_w, sw, sh);

            let frac = (config.power.idle_timeout as f32 - 60.0) / (1800.0 - 60.0);
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_PWR_IDLE_SLIDER, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_IDLE_SLIDER, &rect) {
                config.power.idle_timeout = (60.0 + f * (1800.0 - 60.0)).round() as u32;
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            config.power.idle_timeout = ((config.power.idle_timeout + 29) / 60) * 60;
            if !text_hidden(value_x, label_y, value_w, lsz) {
                let val = format!("{}m", config.power.idle_timeout / 60);
                text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            }
            cy += row;
        }

        // Row: Idle Action
        draw_select_button("Idle Action", &config.power.idle_action,
            ZONE_PWR_IDLE_ACT_BTN, active == Some(ZONE_PWR_IDLE_ACT_BTN),
            painter, text, ix, fox,
            label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);
    }

    cy_top += lid_idle_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 2: Battery
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "Battery",
            card_x, cy_top, card_w, battery_card_h, s, sw, sh,
        );

        // Row: Low Battery Warning %
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Low Battery Warning", lsz, label_x, label_y, fox.text, label_w, sw, sh);

            let frac = (config.power.low_battery_threshold as f32 - 5.0) / 25.0;
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_PWR_LOW_BAT_SLIDER, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_LOW_BAT_SLIDER, &rect) {
                config.power.low_battery_threshold = (5.0 + f * 25.0).round() as u32;
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            if !text_hidden(value_x, label_y, value_w, lsz) {
                let val = format!("{}%", config.power.low_battery_threshold);
                text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            }
            cy += row;
        }

        // Row: Critical Battery %
        {
            let label_y = cy + (row - lsz) / 2.0;
            text.queue("Critical Battery", lsz, label_x, label_y, fox.text, label_w, sw, sh);

            let frac = (config.power.critical_battery_threshold as f32 - 2.0) / 13.0;
            let rect = Rect::new(ctrl_x, cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
            let zone = ix.add_zone(ZONE_PWR_CRIT_BAT_SLIDER, rect);
            if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_CRIT_BAT_SLIDER, &rect) {
                config.power.critical_battery_threshold = (2.0 + f * 13.0).round() as u32;
            }
            Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
                .draw(painter, fox);
            if !text_hidden(value_x, label_y, value_w, lsz) {
                let val = format!("{}%", config.power.critical_battery_threshold);
                text.queue(&val, vsz, value_x, label_y, fox.text_secondary, value_w, sw, sh);
            }
            cy += row;
        }

        // Row: Critical Battery Action
        draw_select_button("Critical Action", &config.power.critical_battery_action,
            ZONE_PWR_CRIT_BTN, active == Some(ZONE_PWR_CRIT_BTN),
            painter, text, ix, fox,
            label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);
    }

    cy_top += battery_card_h + CARD_GAP * s;

    // ─────────────────────────────────────────────────────────────────
    // Card 3: WiFi Power
    // ─────────────────────────────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter, text, fox, "WiFi Power",
            card_x, cy_top, card_w, wifi_card_h, s, sw, sh,
        );

        // Row: WiFi Power Save toggle
        {
            let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
            let toggle = Toggle::new(rect, config.power.wifi_power_save)
                .label("WiFi Power Save").scale(s);
            let track = toggle.track_rect();
            let zone = ix.add_zone(ZONE_PWR_WIFI_PS, track);
            toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
            cy += row;
        }

        // Row: Power Scheme dropdown
        draw_select_button("Power Scheme", &config.power.wifi_power_scheme,
            ZONE_PWR_WIFI_SCHEME_BTN, active == Some(ZONE_PWR_WIFI_SCHEME_BTN),
            painter, text, ix, fox,
            label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, panel_state.scroll_offset);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }

    // ── Draw context menu LAST so it's on top ───────────────────────
    panel_state.dropdown_menu.set_scale(s);
    panel_state.dropdown_menu.update(0.016);
    if let Some(evt) = panel_state.dropdown_menu.draw(painter, text, ix, sw, sh) {
        if let MenuEvent::Action(id) = evt {
            match id {
                id if id >= ACT_LID && id < ACT_LID + LID_OPTIONS.len() as u32 => {
                    config.power.lid_close_action = LID_OPTIONS[(id - ACT_LID) as usize].to_lowercase();
                }
                id if id >= ACT_LID_AC && id < ACT_LID_AC + LID_OPTIONS.len() as u32 => {
                    config.power.lid_close_on_ac = LID_OPTIONS[(id - ACT_LID_AC) as usize].to_lowercase();
                }
                id if id >= ACT_IDLE && id < ACT_IDLE + IDLE_ACTION_OPTIONS.len() as u32 => {
                    config.power.idle_action = IDLE_ACTION_OPTIONS[(id - ACT_IDLE) as usize].to_lowercase();
                }
                id if id >= ACT_CRIT && id < ACT_CRIT + CRIT_OPTIONS.len() as u32 => {
                    config.power.critical_battery_action = CRIT_OPTIONS[(id - ACT_CRIT) as usize].to_lowercase();
                }
                id if id >= ACT_WIFI_SCHEME && id < ACT_WIFI_SCHEME + WIFI_SCHEME_OPTIONS.len() as u32 => {
                    config.power.wifi_power_scheme = WIFI_SCHEME_OPTIONS[(id - ACT_WIFI_SCHEME) as usize].to_lowercase();
                }
                _ => {}
            }
            panel_state.close_dropdown();
        }
    }
}

// ── Click handling ──────────────────────────────────────────────────────────

pub fn handle_power_click(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    zone_id: u32,
    cursor_x: f32,
    cursor_y: f32,
) {
    if zone_id == ZONE_PWR_WIFI_PS {
        config.power.wifi_power_save = !config.power.wifi_power_save;
        return;
    }

    let dropdown_defs: &[(u32, &[&str], &str, u32)] = &[
        (ZONE_PWR_LID_BTN,         LID_OPTIONS,         &config.power.lid_close_action,        ACT_LID),
        (ZONE_PWR_LID_AC_BTN,      LID_OPTIONS,         &config.power.lid_close_on_ac,         ACT_LID_AC),
        (ZONE_PWR_IDLE_ACT_BTN,    IDLE_ACTION_OPTIONS, &config.power.idle_action,             ACT_IDLE),
        (ZONE_PWR_CRIT_BTN,        CRIT_OPTIONS,        &config.power.critical_battery_action, ACT_CRIT),
        (ZONE_PWR_WIFI_SCHEME_BTN, WIFI_SCHEME_OPTIONS, &config.power.wifi_power_scheme,       ACT_WIFI_SCHEME),
    ];

    for (btn_zone, options, current, base_id) in dropdown_defs {
        if zone_id == *btn_zone {
            if panel_state.active_dropdown == Some(*btn_zone) {
                panel_state.close_dropdown();
            } else {
                // Open the menu directly under the cursor — works regardless of
                // which card the dropdown lives in.
                let items = make_menu_items(options, *base_id, current);
                panel_state.dropdown_menu.open(cursor_x, cursor_y + 16.0, items);
                panel_state.active_dropdown = Some(*btn_zone);
            }
            return;
        }
    }

    if panel_state.dropdown_menu.is_open() {
        panel_state.close_dropdown();
    }
}

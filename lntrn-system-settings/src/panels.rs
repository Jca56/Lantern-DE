use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext,
    MenuEvent, MenuItem, Slider, Toggle,
};

use crate::config::LanternConfig;

pub const ZONE_SAVE: u32 = 900;
pub const ZONE_CANCEL: u32 = 901;

const ZONE_WM_BORDER: u32 = 300;
const ZONE_WM_TITLEBAR: u32 = 301;
const ZONE_WM_GAP: u32 = 302;
const ZONE_WM_CORNER: u32 = 303;
const ZONE_WM_FOCUS: u32 = 304;

const ZONE_PWR_LID_BTN: u32 = 400;
const ZONE_PWR_LID_AC_BTN: u32 = 401;
const ZONE_PWR_DIM_SLIDER: u32 = 402;
const ZONE_PWR_IDLE_SLIDER: u32 = 403;
const ZONE_PWR_IDLE_ACT_BTN: u32 = 404;
const ZONE_PWR_LOW_BAT_SLIDER: u32 = 405;
const ZONE_PWR_CRIT_BAT_SLIDER: u32 = 406;
const ZONE_PWR_CRIT_BTN: u32 = 407;

const ACT_LID: u32 = 500;
const ACT_LID_AC: u32 = 510;
const ACT_IDLE: u32 = 520;
const ACT_CRIT: u32 = 530;

const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const VALUE_SIZE: f32 = 16.0;
const SLIDER_H: f32 = 36.0;
const BTN_H: f32 = 42.0;
const TOGGLE_H: f32 = 36.0;
const PAD_LEFT: f32 = 24.0;
const PAD_RIGHT: f32 = 32.0;
const LABEL_W: f32 = 200.0;
const VALUE_W: f32 = 60.0;

const LID_OPTIONS: &[&str] = &["Suspend", "Hibernate", "Lock", "Nothing"];
const IDLE_ACTION_OPTIONS: &[&str] = &["Suspend", "Lock", "Nothing"];
const CRIT_OPTIONS: &[&str] = &["Suspend", "Hibernate", "Shutdown", "Nothing"];

// ── Panel state ─────────────────────────────────────────────────────────────

pub struct PanelState {
    pub dropdown_menu: ContextMenu,
    pub active_dropdown: Option<u32>,
}

impl PanelState {
    pub fn new(fox: &FoxPalette) -> Self {
        Self {
            dropdown_menu: ContextMenu::new(ContextMenuStyle::from_palette(fox)),
            active_dropdown: None,
        }
    }

    pub fn close_dropdown(&mut self) {
        self.dropdown_menu.close();
        self.active_dropdown = None;
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn slider_value_from_cursor(
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

fn layout(x: f32, w: f32, s: f32) -> (f32, f32, f32, f32) {
    let pad_l = PAD_LEFT * s;
    let pad_r = PAD_RIGHT * s;
    let val_w = VALUE_W * s;
    let label_x = x + pad_l;
    let label_w = LABEL_W * s;
    let ctrl_x = label_x + label_w;
    let ctrl_w = w - pad_l - pad_r - label_w - val_w - 12.0 * s;
    let value_x = ctrl_x + ctrl_w + 8.0 * s;
    (label_x, ctrl_x, ctrl_w.max(80.0 * s), value_x)
}

/// Returns true if the rect at (text_x, row_y, text_w, row_h) significantly overlaps the menu.
/// Uses a margin to ignore shadow/padding overlap at the edges.
fn hidden_by_menu(text_x: f32, row_y: f32, text_w: f32, row_h: f32, menu: &ContextMenu) -> bool {
    if !menu.is_open() { return false; }
    if let Some(b) = menu.bounds() {
        // Shrink menu bounds by margin to ignore shadow overlap
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

fn draw_select_button(
    label: &str, current: &str,
    zone_id: u32, is_open: bool,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette,
    label_x: f32, label_w: f32, btn_x: f32, btn_w: f32, btn_h: f32,
    row: f32, lsz: f32, s: f32, sw: u32, sh: u32,
    cy: &mut f32, menu: &ContextMenu,
) {
    // Always draw the label on the left
    let label_y = *cy + (row - lsz) / 2.0;
    text.queue(label, lsz, label_x, label_y, fox.text, label_w, sw, sh);

    let rect = Rect::new(btn_x, *cy + (row - btn_h) / 2.0, btn_w, btn_h);
    let zone = ix.add_zone(zone_id, rect);

    // Always draw the button shape (painter z-order handles it)
    let bg = if is_open || zone.is_hovered() { fox.surface_2 } else { fox.surface };
    let r = 6.0 * s;
    painter.rect_filled(rect, r, bg);
    painter.rect_stroke_sdf(rect, r, 1.0 * s, fox.muted.with_alpha(0.3));

    let font = 18.0 * s;
    let pad_h = 14.0 * s;

    // Only skip button TEXT if it overlaps the menu
    let skip_text = hidden_by_menu(btn_x, *cy, btn_w, row, menu);
    if !skip_text {
        let text_y = rect.y + (rect.h - font) / 2.0;
        let display: String = current.chars().take(1).flat_map(|c| c.to_uppercase())
            .chain(current.chars().skip(1)).collect();
        text.queue(&display, font, rect.x + pad_h, text_y, fox.text,
            btn_w - pad_h * 2.0 - 12.0 * s, sw, sh);
    }

    // Always draw chevron (painter shape, not text)
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

fn make_menu_items(options: &[&str], base_id: u32, current: &str) -> Vec<MenuItem> {
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

// ── Window Manager panel ────────────────────────────────────────────────────

pub fn draw_wm_panel(
    config: &mut LanternConfig,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, x: f32, y: f32, w: f32, s: f32, sw: u32, sh: u32,
) {
    let (label_x, ctrl_x, ctrl_w, value_x) = layout(x, w, s);
    let mut cy = y;
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    let mut slider_row = |label: &str, frac: f32, zone_id: u32, cy: &mut f32,
                          min: f32, max: f32, suffix: &str, config_val: &mut u32| {
        let label_y = *cy + (row - lsz) / 2.0;
        text.queue(label, lsz, label_x, label_y, fox.text, ctrl_x - label_x, sw, sh);
        let rect = Rect::new(ctrl_x, *cy + (row - slider_h) / 2.0, ctrl_w, slider_h);
        let zone = ix.add_zone(zone_id, rect);
        if let Some(f) = slider_value_from_cursor(ix, zone_id, &rect) {
            *config_val = (min + f * (max - min)).round() as u32;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        let val = format!("{}{}", *config_val, suffix);
        text.queue(&val, vsz, value_x, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        *cy += row;
    };

    let frac = config.window_manager.border_width as f32 / 10.0;
    let mut bw = config.window_manager.border_width;
    slider_row("Border Width", frac, ZONE_WM_BORDER, &mut cy, 0.0, 10.0, "", &mut bw);
    config.window_manager.border_width = bw;

    let frac = (config.window_manager.titlebar_height as f32 - 20.0) / 40.0;
    let mut th = config.window_manager.titlebar_height;
    slider_row("Titlebar Height", frac, ZONE_WM_TITLEBAR, &mut cy, 20.0, 60.0, "px", &mut th);
    config.window_manager.titlebar_height = th;

    let frac = config.window_manager.gap as f32 / 32.0;
    let mut gap = config.window_manager.gap;
    slider_row("Window Gap", frac, ZONE_WM_GAP, &mut cy, 0.0, 32.0, "px", &mut gap);
    config.window_manager.gap = gap;

    let frac = config.window_manager.corner_radius as f32 / 20.0;
    let mut cr = config.window_manager.corner_radius;
    slider_row("Corner Radius", frac, ZONE_WM_CORNER, &mut cy, 0.0, 20.0, "px", &mut cr);
    config.window_manager.corner_radius = cr;

    {
        let rect = Rect::new(label_x, cy, w - PAD_LEFT * s - PAD_RIGHT * s, TOGGLE_H * s);
        let toggle = Toggle::new(rect, config.window_manager.focus_follows_mouse)
            .label("Focus Follows Mouse").scale(s);
        let track = toggle.track_rect();
        let zone = ix.add_zone(ZONE_WM_FOCUS, track);
        toggle.hovered(zone.is_hovered()).draw(painter, text, fox, sw, sh);
    }
}

// ── Power panel ─────────────────────────────────────────────────────────────

pub fn draw_power_panel(
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    painter: &mut Painter, text: &mut TextRenderer, ix: &mut InteractionContext,
    fox: &FoxPalette, x: f32, y: f32, w: f32, s: f32, sw: u32, sh: u32,
) {
    let pad_l = PAD_LEFT * s;
    let pad_r = PAD_RIGHT * s;
    let label_x = x + pad_l;
    let label_w = LABEL_W * s;
    let btn_w = 200.0 * s;
    let btn_x = x + w - pad_r - btn_w;
    let btn_h = BTN_H * s;
    let (_, slider_cx, slider_cw, slider_vx) = layout(x, w, s);

    let mut cy = y;
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;
    let slider_h = SLIDER_H * s;

    let active = panel_state.active_dropdown;
    let menu = &panel_state.dropdown_menu;

    // Helper: should we skip this text? Only skip text that overlaps the menu.
    // Painter shapes are fine — the menu draws last and covers them.
    let text_hidden = |tx: f32, ty: f32, tw: f32, th: f32| -> bool {
        hidden_by_menu(tx, ty, tw, th, menu)
    };

    // Row 0: Lid Close (Battery)
    draw_select_button("Lid Close (Battery)", &config.power.lid_close_action,
        ZONE_PWR_LID_BTN, active == Some(ZONE_PWR_LID_BTN),
        painter, text, ix, fox, label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);

    // Row 1: Lid Close (AC)
    draw_select_button("Lid Close (AC)", &config.power.lid_close_on_ac,
        ZONE_PWR_LID_AC_BTN, active == Some(ZONE_PWR_LID_AC_BTN),
        painter, text, ix, fox, label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);

    // Row 2: Dim Screen After slider (0–600 seconds, 0 = never)
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Dim Screen After", lsz, label_x, label_y, fox.text, label_w, sw, sh);

        let frac = config.power.dim_after as f32 / 600.0;
        let rect = Rect::new(slider_cx, cy + (row - slider_h) / 2.0, slider_cw, slider_h);
        let zone = ix.add_zone(ZONE_PWR_DIM_SLIDER, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_DIM_SLIDER, &rect) {
            config.power.dim_after = (f * 600.0).round() as u32;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        // Round to nearest 30s for cleaner values
        config.power.dim_after = ((config.power.dim_after + 14) / 30) * 30;
        if !text_hidden(slider_vx, label_y, VALUE_W * s, lsz) {
            let val = if config.power.dim_after == 0 {
                "Never".to_string()
            } else {
                let mins = config.power.dim_after / 60;
                let secs = config.power.dim_after % 60;
                if mins > 0 && secs > 0 { format!("{}m {}s", mins, secs) }
                else if mins > 0 { format!("{}m", mins) }
                else { format!("{}s", secs) }
            };
            text.queue(&val, vsz, slider_vx, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        }
        cy += row;
    }

    // Row 3: Idle Timeout slider
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Idle Timeout", lsz, label_x, label_y, fox.text, label_w, sw, sh);

        let frac = (config.power.idle_timeout as f32 - 60.0) / (1800.0 - 60.0);
        let rect = Rect::new(slider_cx, cy + (row - slider_h) / 2.0, slider_cw, slider_h);
        let zone = ix.add_zone(ZONE_PWR_IDLE_SLIDER, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_IDLE_SLIDER, &rect) {
            config.power.idle_timeout = (60.0 + f * (1800.0 - 60.0)).round() as u32;
        }
        // Always draw slider (painter z-order is fine)
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        config.power.idle_timeout = ((config.power.idle_timeout + 29) / 60) * 60;
        // Only skip the value text if it overlaps the menu
        if !text_hidden(slider_vx, label_y, VALUE_W * s, lsz) {
            let val = format!("{}m", config.power.idle_timeout / 60);
            text.queue(&val, vsz, slider_vx, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        }
        cy += row;
    }

    // Row 3: Idle Action
    draw_select_button("Idle Action", &config.power.idle_action,
        ZONE_PWR_IDLE_ACT_BTN, active == Some(ZONE_PWR_IDLE_ACT_BTN),
        painter, text, ix, fox, label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);

    // Row 4: Low Battery Warning slider
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Low Battery Warning", lsz, label_x, label_y, fox.text, label_w, sw, sh);

        let frac = (config.power.low_battery_threshold as f32 - 5.0) / 25.0;
        let rect = Rect::new(slider_cx, cy + (row - slider_h) / 2.0, slider_cw, slider_h);
        let zone = ix.add_zone(ZONE_PWR_LOW_BAT_SLIDER, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_LOW_BAT_SLIDER, &rect) {
            config.power.low_battery_threshold = (5.0 + f * 25.0).round() as u32;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        if !text_hidden(slider_vx, label_y, VALUE_W * s, lsz) {
            let val = format!("{}%", config.power.low_battery_threshold);
            text.queue(&val, vsz, slider_vx, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        }
        cy += row;
    }

    // Row 5: Critical Battery % slider
    {
        let label_y = cy + (row - lsz) / 2.0;
        text.queue("Critical Battery %", lsz, label_x, label_y, fox.text, label_w, sw, sh);

        let frac = (config.power.critical_battery_threshold as f32 - 2.0) / 13.0;
        let rect = Rect::new(slider_cx, cy + (row - slider_h) / 2.0, slider_cw, slider_h);
        let zone = ix.add_zone(ZONE_PWR_CRIT_BAT_SLIDER, rect);
        if let Some(f) = slider_value_from_cursor(ix, ZONE_PWR_CRIT_BAT_SLIDER, &rect) {
            config.power.critical_battery_threshold = (2.0 + f * 13.0).round() as u32;
        }
        Slider::new(rect).value(frac).hovered(zone.is_hovered()).active(zone.is_active())
            .draw(painter, fox);
        if !text_hidden(slider_vx, label_y, VALUE_W * s, lsz) {
            let val = format!("{}%", config.power.critical_battery_threshold);
            text.queue(&val, vsz, slider_vx, label_y, fox.text_secondary, VALUE_W * s, sw, sh);
        }
        cy += row;
    }

    // Row 6: Critical Battery Action
    draw_select_button("Critical Battery", &config.power.critical_battery_action,
        ZONE_PWR_CRIT_BTN, active == Some(ZONE_PWR_CRIT_BTN),
        painter, text, ix, fox, label_x, label_w, btn_x, btn_w, btn_h, row, lsz, s, sw, sh, &mut cy, menu);

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
                _ => {}
            }
            panel_state.close_dropdown();
        }
    }
}

// ── Click handling ──────────────────────────────────────────────────────────

pub fn handle_power_click(
    config: &LanternConfig, panel_state: &mut PanelState, zone_id: u32,
    btn_x: f32, _btn_w: f32, btn_h: f32, row: f32, panel_y: f32, _s: f32,
) {
    let dropdown_defs: &[(u32, &[&str], &str, u32, usize)] = &[
        (ZONE_PWR_LID_BTN,      LID_OPTIONS,          &config.power.lid_close_action,       ACT_LID,    0),
        (ZONE_PWR_LID_AC_BTN,   LID_OPTIONS,          &config.power.lid_close_on_ac,        ACT_LID_AC, 1),
        (ZONE_PWR_IDLE_ACT_BTN, IDLE_ACTION_OPTIONS,   &config.power.idle_action,            ACT_IDLE,   4),
        (ZONE_PWR_CRIT_BTN,     CRIT_OPTIONS,          &config.power.critical_battery_action, ACT_CRIT,  7),
    ];

    for (btn_zone, options, current, base_id, row_idx) in dropdown_defs {
        if zone_id == *btn_zone {
            if panel_state.active_dropdown == Some(*btn_zone) {
                panel_state.close_dropdown();
            } else {
                let menu_y = panel_y + *row_idx as f32 * row + (row + btn_h) / 2.0;
                let items = make_menu_items(options, *base_id, current);
                panel_state.dropdown_menu.open(btn_x, menu_y, items);
                panel_state.active_dropdown = Some(*btn_zone);
            }
            return;
        }
    }

    if panel_state.dropdown_menu.is_open() {
        panel_state.close_dropdown();
    }
}

pub fn handle_wm_click(config: &mut LanternConfig, zone_id: u32) {
    if zone_id == ZONE_WM_FOCUS {
        config.window_manager.focus_follows_mouse = !config.window_manager.focus_follows_mouse;
    }
}

// ── Save / Cancel bar ───────────────────────────────────────────────────────

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

    // Subtle separator above
    painter.rect_filled(
        Rect::new(content_x + 16.0 * s, bar_y, w - 32.0 * s, 1.0 * s),
        0.0,
        fox.muted.with_alpha(0.2),
    );

    // Save button (right-aligned, primary)
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

    // Cancel button (left of save)
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

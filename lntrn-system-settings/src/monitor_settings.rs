//! Per-monitor settings UI: resolution, refresh rate, scale dropdowns.
//! Drawn below the monitor arrangement in the Display panel.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::output_manager::OutputManagerClient;

// ── Zone IDs ───────────────────────────────────────────────────────

pub const ZONE_RES_BTN: u32 = 1100;
pub const ZONE_REFRESH_BTN: u32 = 1101;
pub const ZONE_SCALE_BTN: u32 = 1102;

const ZONE_RES_BASE: u32 = 1110;
const ZONE_REFRESH_BASE: u32 = 1140;
const ZONE_SCALE_BASE: u32 = 1170;
const MAX_ITEMS: u32 = 20;

// ── Layout ─────────────────────────────────────────────────────────

const PAD: f32 = 24.0;
const ROW_H: f32 = 48.0;
const LABEL_SIZE: f32 = 18.0;
const LABEL_W: f32 = 160.0;
const BTN_H: f32 = 42.0;
const BTN_W: f32 = 280.0;
const DROPDOWN_ITEM_H: f32 = 40.0;

// ── State ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenDropdown {
    None,
    Resolution,
    RefreshRate,
    Scale,
}

pub struct MonitorSettingsState {
    pub open_dropdown: OpenDropdown,
    /// Selected resolution for the active monitor (w, h).
    pub selected_resolution: Option<(i32, i32)>,
    /// Selected mode index (into the head's modes list).
    pub selected_mode_idx: Option<usize>,
    /// Selected scale value.
    pub selected_scale: Option<f64>,
    /// True when user has made changes that need applying.
    pub dirty: bool,
}

impl MonitorSettingsState {
    pub fn new() -> Self {
        Self {
            open_dropdown: OpenDropdown::None,
            selected_resolution: None,
            selected_mode_idx: None,
            selected_scale: None,
            dirty: false,
        }
    }

    pub fn close_dropdown(&mut self) {
        self.open_dropdown = OpenDropdown::None;
    }

    /// Sync from the output manager's current state for a given head.
    pub fn sync_from_head(&mut self, output_mgr: &OutputManagerClient, head_idx: usize) {
        let Some(head) = output_mgr.heads.get(head_idx) else { return };
        if self.selected_resolution.is_none() {
            if let Some(mi) = head.current_mode {
                if let Some(mode) = head.modes.get(mi) {
                    self.selected_resolution = Some((mode.width, mode.height));
                    self.selected_mode_idx = Some(mi);
                }
            }
        }
        if self.selected_scale.is_none() {
            self.selected_scale = Some(head.scale);
        }
    }

    /// Reset when selected monitor changes.
    pub fn reset(&mut self) {
        self.selected_resolution = None;
        self.selected_mode_idx = None;
        self.selected_scale = None;
        self.open_dropdown = OpenDropdown::None;
        self.dirty = false;
    }
}

// ── Draw ───────────────────────────────────────────────────────────

/// Draw per-monitor settings. Returns height consumed.
pub fn draw_monitor_settings(
    output_mgr: &OutputManagerClient,
    mss: &mut MonitorSettingsState,
    head_idx: usize,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y: f32,
    w: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> f32 {
    let Some(head) = output_mgr.heads.get(head_idx) else { return 0.0 };
    mss.sync_from_head(output_mgr, head_idx);

    let pad = PAD * s;
    let lsz = LABEL_SIZE * s;
    let row_h = ROW_H * s;
    let btn_h = BTN_H * s;
    let btn_w = BTN_W * s;
    let label_w = LABEL_W * s;
    let label_x = x + pad;
    let btn_x = label_x + label_w;

    let mut cy = y;

    // Header
    text.queue(
        &format!("Settings: {}", head.name),
        lsz * 1.1,
        label_x,
        cy + (row_h - lsz) / 2.0,
        fox.accent,
        w - pad * 2.0,
        sw,
        sh,
    );
    cy += row_h;

    // ── Resolution row ─────────────────────────────────────────────
    let res_label_y = cy + (row_h - lsz) / 2.0;
    text.queue("Resolution", lsz, label_x, res_label_y, fox.text, label_w, sw, sh);

    let resolutions = output_mgr.resolutions_for_head(head_idx);
    let cur_res = mss.selected_resolution.unwrap_or((0, 0));
    let res_text = format!("{}x{}", cur_res.0, cur_res.1);

    let btn_rect = Rect::new(btn_x, cy + (row_h - btn_h) / 2.0, btn_w, btn_h);
    let btn_zone = ix.add_zone(ZONE_RES_BTN, btn_rect);
    draw_dropdown_button(painter, text, &btn_rect, &res_text, btn_zone.is_hovered(), fox, s, sw, sh);
    cy += row_h;

    // Resolution dropdown items (if open)
    if mss.open_dropdown == OpenDropdown::Resolution {
        let item_h = DROPDOWN_ITEM_H * s;
        for (i, (rw, rh)) in resolutions.iter().enumerate() {
            if i as u32 >= MAX_ITEMS { break; }
            let item_rect = Rect::new(btn_x, cy, btn_w, item_h);
            let zone = ix.add_zone(ZONE_RES_BASE + i as u32, item_rect);
            let is_current = cur_res == (*rw, *rh);
            draw_dropdown_item(
                painter, text, &item_rect,
                &format!("{}x{}", rw, rh),
                zone.is_hovered(), is_current, fox, s, sw, sh,
            );
            cy += item_h;
        }
        cy += 4.0 * s;
    }

    // ── Refresh Rate row ───────────────────────────────────────────
    let ref_label_y = cy + (row_h - lsz) / 2.0;
    text.queue("Refresh Rate", lsz, label_x, ref_label_y, fox.text, label_w, sw, sh);

    let rates = output_mgr.refresh_rates_for_resolution(head_idx, cur_res.0, cur_res.1);
    let cur_mode = mss.selected_mode_idx.unwrap_or(usize::MAX);
    let rate_text = rates
        .iter()
        .find(|(_, mi)| *mi == cur_mode)
        .map(|(r, _)| format!("{:.1} Hz", *r as f64 / 1000.0))
        .unwrap_or_else(|| "Auto".into());

    let btn_rect = Rect::new(btn_x, cy + (row_h - btn_h) / 2.0, btn_w, btn_h);
    let btn_zone = ix.add_zone(ZONE_REFRESH_BTN, btn_rect);
    draw_dropdown_button(painter, text, &btn_rect, &rate_text, btn_zone.is_hovered(), fox, s, sw, sh);
    cy += row_h;

    // Refresh rate dropdown items
    if mss.open_dropdown == OpenDropdown::RefreshRate {
        let item_h = DROPDOWN_ITEM_H * s;
        for (i, (refresh, mode_idx)) in rates.iter().enumerate() {
            if i as u32 >= MAX_ITEMS { break; }
            let item_rect = Rect::new(btn_x, cy, btn_w, item_h);
            let zone = ix.add_zone(ZONE_REFRESH_BASE + i as u32, item_rect);
            let is_current = *mode_idx == cur_mode;
            draw_dropdown_item(
                painter, text, &item_rect,
                &format!("{:.1} Hz", *refresh as f64 / 1000.0),
                zone.is_hovered(), is_current, fox, s, sw, sh,
            );
            cy += item_h;
        }
        cy += 4.0 * s;
    }

    // ── Scale row ──────────────────────────────────────────────────
    let scale_label_y = cy + (row_h - lsz) / 2.0;
    text.queue("Scale", lsz, label_x, scale_label_y, fox.text, label_w, sw, sh);

    let scales = [1.0, 1.25, 1.5, 1.75, 2.0];
    let cur_scale = mss.selected_scale.unwrap_or(1.25);
    let scale_text = format!("{:.2}x", cur_scale);

    let btn_rect = Rect::new(btn_x, cy + (row_h - btn_h) / 2.0, btn_w, btn_h);
    let btn_zone = ix.add_zone(ZONE_SCALE_BTN, btn_rect);
    draw_dropdown_button(painter, text, &btn_rect, &scale_text, btn_zone.is_hovered(), fox, s, sw, sh);
    cy += row_h;

    // Scale dropdown items
    if mss.open_dropdown == OpenDropdown::Scale {
        let item_h = DROPDOWN_ITEM_H * s;
        for (i, scale_val) in scales.iter().enumerate() {
            let item_rect = Rect::new(btn_x, cy, btn_w, item_h);
            let zone = ix.add_zone(ZONE_SCALE_BASE + i as u32, item_rect);
            let is_current = (cur_scale - scale_val).abs() < 0.01;
            draw_dropdown_item(
                painter, text, &item_rect,
                &format!("{:.2}x", scale_val),
                zone.is_hovered(), is_current, fox, s, sw, sh,
            );
            cy += item_h;
        }
        cy += 4.0 * s;
    }

    cy - y
}

// ── Click handling ─────────────────────────────────────────────────

pub fn handle_monitor_settings_click(
    output_mgr: &OutputManagerClient,
    mss: &mut MonitorSettingsState,
    head_idx: usize,
    zone_id: u32,
) -> bool {
    // Dropdown button toggles
    if zone_id == ZONE_RES_BTN {
        mss.open_dropdown = if mss.open_dropdown == OpenDropdown::Resolution {
            OpenDropdown::None
        } else {
            OpenDropdown::Resolution
        };
        return true;
    }
    if zone_id == ZONE_REFRESH_BTN {
        mss.open_dropdown = if mss.open_dropdown == OpenDropdown::RefreshRate {
            OpenDropdown::None
        } else {
            OpenDropdown::RefreshRate
        };
        return true;
    }
    if zone_id == ZONE_SCALE_BTN {
        mss.open_dropdown = if mss.open_dropdown == OpenDropdown::Scale {
            OpenDropdown::None
        } else {
            OpenDropdown::Scale
        };
        return true;
    }

    // Resolution selection
    if zone_id >= ZONE_RES_BASE && zone_id < ZONE_RES_BASE + MAX_ITEMS {
        let idx = (zone_id - ZONE_RES_BASE) as usize;
        let resolutions = output_mgr.resolutions_for_head(head_idx);
        if let Some(&(w, h)) = resolutions.get(idx) {
            mss.selected_resolution = Some((w, h));
            // Auto-pick highest refresh rate at this resolution
            let rates = output_mgr.refresh_rates_for_resolution(head_idx, w, h);
            if let Some(&(_, mode_idx)) = rates.first() {
                mss.selected_mode_idx = Some(mode_idx);
            }
            mss.dirty = true;
        }
        mss.open_dropdown = OpenDropdown::None;
        return true;
    }

    // Refresh rate selection
    if zone_id >= ZONE_REFRESH_BASE && zone_id < ZONE_REFRESH_BASE + MAX_ITEMS {
        let idx = (zone_id - ZONE_REFRESH_BASE) as usize;
        let cur_res = mss.selected_resolution.unwrap_or((0, 0));
        let rates = output_mgr.refresh_rates_for_resolution(head_idx, cur_res.0, cur_res.1);
        if let Some(&(_, mode_idx)) = rates.get(idx) {
            mss.selected_mode_idx = Some(mode_idx);
            mss.dirty = true;
        }
        mss.open_dropdown = OpenDropdown::None;
        return true;
    }

    // Scale selection
    if zone_id >= ZONE_SCALE_BASE && zone_id < ZONE_SCALE_BASE + 5 {
        let scales = [1.0, 1.25, 1.5, 1.75, 2.0];
        let idx = (zone_id - ZONE_SCALE_BASE) as usize;
        if let Some(&val) = scales.get(idx) {
            mss.selected_scale = Some(val);
            mss.dirty = true;
        }
        mss.open_dropdown = OpenDropdown::None;
        return true;
    }

    false
}

// ── Drawing helpers ────────────────────────────────────────────────

fn draw_dropdown_button(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: &Rect,
    label: &str,
    hovered: bool,
    fox: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let bg = if hovered { fox.surface } else { fox.bg };
    painter.rect_filled(*rect, 6.0 * s, bg);
    painter.rect_stroke_sdf(*rect, 6.0 * s, 1.0 * s, fox.muted);
    let lsz = 18.0 * s;
    let tx = rect.x + 12.0 * s;
    let ty = rect.y + (rect.h - lsz) / 2.0;
    text.queue(label, lsz, tx, ty, fox.text, rect.w - 24.0 * s, sw, sh);
    // Chevron
    let chev_x = rect.x + rect.w - 20.0 * s;
    let chev_y = rect.y + rect.h / 2.0;
    let cs = 4.0 * s;
    painter.line(chev_x - cs, chev_y - cs * 0.6, chev_x, chev_y + cs * 0.4, 1.5 * s, fox.text_secondary);
    painter.line(chev_x, chev_y + cs * 0.4, chev_x + cs, chev_y - cs * 0.6, 1.5 * s, fox.text_secondary);
}

fn draw_dropdown_item(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: &Rect,
    label: &str,
    hovered: bool,
    is_current: bool,
    fox: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let bg = if hovered {
        fox.accent.with_alpha(0.2)
    } else {
        fox.surface.with_alpha(0.8)
    };
    painter.rect_filled(*rect, 4.0 * s, bg);
    let lsz = 18.0 * s;
    let tx = rect.x + 12.0 * s;
    let ty = rect.y + (rect.h - lsz) / 2.0;
    let color = if is_current { fox.accent } else { fox.text };
    text.queue(label, lsz, tx, ty, color, rect.w - 24.0 * s, sw, sh);
}

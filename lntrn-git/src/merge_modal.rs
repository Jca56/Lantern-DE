//! Merge picker modal — select source and target branches, then merge.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

// Zone IDs
const ZONE_BACKDROP: u32 = 6000;
const ZONE_MERGE_BTN: u32 = 6001;
const ZONE_CANCEL_BTN: u32 = 6002;
const ZONE_SOURCE_BTN: u32 = 6003;
const ZONE_TARGET_BTN: u32 = 6004;
const ZONE_SOURCE_BASE: u32 = 6100;
const ZONE_TARGET_BASE: u32 = 6200;

pub enum MergeAction {
    None,
    Cancel,
    Merge { source: String, target: String },
}

pub struct MergeModal {
    pub visible: bool,
    pub branches: Vec<String>,
    pub current_branch: String,
    source_idx: Option<usize>,
    target_idx: Option<usize>,
    source_open: bool,
    target_open: bool,
}

impl MergeModal {
    pub fn new() -> Self {
        Self {
            visible: false,
            branches: Vec::new(),
            current_branch: String::new(),
            source_idx: None,
            target_idx: None,
            source_open: false,
            target_open: false,
        }
    }

    pub fn open(&mut self, branches: Vec<String>, current: &str) {
        self.visible = true;
        self.current_branch = current.to_string();
        // Default target = current branch, source = first other branch
        self.target_idx = branches.iter().position(|b| b == current);
        self.source_idx = branches.iter().position(|b| b != current);
        self.branches = branches;
        self.source_open = false;
        self.target_open = false;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.source_open = false;
        self.target_open = false;
    }

    pub fn on_click(&mut self, ix: &InteractionContext, px: f32, py: f32) -> MergeAction {
        if !self.visible { return MergeAction::None; }

        let Some(zone) = ix.zone_at(px, py) else {
            self.close();
            return MergeAction::Cancel;
        };

        if zone == ZONE_BACKDROP {
            self.close();
            return MergeAction::Cancel;
        }
        if zone == ZONE_CANCEL_BTN {
            self.close();
            return MergeAction::Cancel;
        }
        if zone == ZONE_MERGE_BTN {
            if let (Some(si), Some(ti)) = (self.source_idx, self.target_idx) {
                if si != ti {
                    let source = self.branches[si].clone();
                    let target = self.branches[ti].clone();
                    self.close();
                    return MergeAction::Merge { source, target };
                }
            }
            return MergeAction::None;
        }

        // Dropdown toggles
        if zone == ZONE_SOURCE_BTN { self.source_open = !self.source_open; self.target_open = false; return MergeAction::None; }
        if zone == ZONE_TARGET_BTN { self.target_open = !self.target_open; self.source_open = false; return MergeAction::None; }

        // Source dropdown items
        if zone >= ZONE_SOURCE_BASE && zone < ZONE_SOURCE_BASE + 32 {
            self.source_idx = Some((zone - ZONE_SOURCE_BASE) as usize);
            self.source_open = false;
            return MergeAction::None;
        }
        // Target dropdown items
        if zone >= ZONE_TARGET_BASE && zone < ZONE_TARGET_BASE + 32 {
            self.target_idx = Some((zone - ZONE_TARGET_BASE) as usize);
            self.target_open = false;
            return MergeAction::None;
        }

        MergeAction::None
    }

    pub fn draw(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        s: f32, wf: f32, hf: f32, sw: u32, sh: u32,
    ) {
        if !self.visible { return; }

        let font = 22.0 * s;
        let small = 18.0 * s;
        let pad = 24.0 * s;
        let btn_h = 40.0 * s;
        let row_h = 38.0 * s;
        let dropdown_h = 44.0 * s;
        let panel_w = 420.0 * s;

        // Count items to size panel
        let dropdown_items = if self.source_open || self.target_open { self.branches.len() as f32 * row_h + 8.0 * s } else { 0.0 };
        let panel_h = pad * 2.0 + font + pad + dropdown_h + pad * 0.5 + dropdown_h + pad + btn_h + dropdown_items + pad;

        let px = (wf - panel_w) * 0.5;
        let py = (hf - panel_h) * 0.5;

        // Backdrop
        let backdrop = Rect::new(0.0, 0.0, wf, hf);
        ix.add_zone(ZONE_BACKDROP, backdrop);
        painter.rect_filled(backdrop, 0.0, Color::BLACK.with_alpha(0.4));

        // Panel
        let panel = Rect::new(px, py, panel_w, panel_h);
        painter.rect_filled(panel, 12.0 * s, palette.surface);
        painter.rect_stroke(panel, 12.0 * s, 1.0 * s, palette.muted.with_alpha(0.3));

        let mut y = py + pad;

        // Title
        text.queue("Merge Branches", font, px + pad, y, palette.text, panel_w, sw, sh);
        y += font + pad;

        // Source dropdown
        text.queue("From:", small, px + pad, y + (dropdown_h - small) * 0.5, palette.muted, 60.0 * s, sw, sh);
        let dd_x = px + pad + 70.0 * s;
        let dd_w = panel_w - pad * 2.0 - 70.0 * s;
        let source_label = self.source_idx.and_then(|i| self.branches.get(i)).map(|s| s.as_str()).unwrap_or("Select...");
        self.draw_dropdown(painter, text, ix, palette, s, sw, sh, dd_x, y, dd_w, dropdown_h, row_h,
            source_label, self.source_open, ZONE_SOURCE_BTN, ZONE_SOURCE_BASE);
        y += dropdown_h + pad * 0.5;

        // Direction arrow
        let arrow_x = px + panel_w * 0.5;
        text.queue("↓ merges into ↓", small, arrow_x - 70.0 * s, y - pad * 0.3, palette.muted, 200.0 * s, sw, sh);

        // Target dropdown
        text.queue("Into:", small, px + pad, y + (dropdown_h - small) * 0.5, palette.muted, 60.0 * s, sw, sh);
        let target_label = self.target_idx.and_then(|i| self.branches.get(i)).map(|s| s.as_str()).unwrap_or("Select...");
        self.draw_dropdown(painter, text, ix, palette, s, sw, sh, dd_x, y, dd_w, dropdown_h, row_h,
            target_label, self.target_open, ZONE_TARGET_BTN, ZONE_TARGET_BASE);
        y += dropdown_h + pad;

        // Buttons
        let merge_w = 120.0 * s;
        let cancel_w = 100.0 * s;
        let btn_gap = 12.0 * s;
        let can_merge = self.source_idx.is_some() && self.target_idx.is_some() && self.source_idx != self.target_idx;

        let cancel_rect = Rect::new(px + panel_w - pad - cancel_w - btn_gap - merge_w, y, cancel_w, btn_h);
        let cancel_state = ix.add_zone(ZONE_CANCEL_BTN, cancel_rect);
        let cancel_c = if cancel_state.is_hovered() { palette.muted.with_alpha(0.3) } else { palette.muted.with_alpha(0.15) };
        painter.rect_filled(cancel_rect, 8.0 * s, cancel_c);
        let cty = y + (btn_h - small) * 0.5;
        let ctw = text.measure_width("Cancel", small);
        text.queue("Cancel", small, cancel_rect.x + (cancel_w - ctw) * 0.5, cty, palette.text, cancel_w, sw, sh);

        let merge_rect = Rect::new(px + panel_w - pad - merge_w, y, merge_w, btn_h);
        let merge_state = ix.add_zone(ZONE_MERGE_BTN, merge_rect);
        let merge_c = if !can_merge { palette.muted.with_alpha(0.3) } else if merge_state.is_hovered() { palette.accent } else { palette.accent.with_alpha(0.8) };
        painter.rect_filled(merge_rect, 8.0 * s, merge_c);
        let mtw = text.measure_width("Merge", small);
        text.queue("Merge", small, merge_rect.x + (merge_w - mtw) * 0.5, cty, palette.text, merge_w, sw, sh);
    }

    fn draw_dropdown(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        s: f32, sw: u32, sh: u32,
        x: f32, y: f32, w: f32, h: f32, row_h: f32,
        label: &str, open: bool, btn_zone: u32, item_zone_base: u32,
    ) {
        let font = 20.0 * s;
        let pad = 10.0 * s;

        // Button
        let btn_rect = Rect::new(x, y, w, h);
        let btn_state = ix.add_zone(btn_zone, btn_rect);
        let bg = if open || btn_state.is_hovered() { palette.surface_2 } else { palette.surface };
        painter.rect_filled(btn_rect, 6.0 * s, bg);
        painter.rect_stroke(btn_rect, 6.0 * s, 1.0 * s, palette.muted.with_alpha(0.3));
        let ty = y + (h - font) * 0.5;
        text.queue(label, font, x + pad, ty, palette.text, w - pad * 2.0, sw, sh);
        let arrow = if open { "▲" } else { "▼" };
        text.queue(arrow, font * 0.7, x + w - pad - 12.0 * s, ty + 2.0 * s, palette.muted, 20.0 * s, sw, sh);

        // Dropdown list
        if !open { return; }
        let list_h = self.branches.len() as f32 * row_h + 8.0 * s;
        let list_y = y + h + 4.0 * s;
        let list_rect = Rect::new(x, list_y, w, list_h);
        painter.rect_filled(list_rect, 6.0 * s, palette.surface);
        painter.rect_stroke(list_rect, 6.0 * s, 1.0 * s, palette.muted.with_alpha(0.2));

        for (i, branch) in self.branches.iter().enumerate() {
            let iy = list_y + 4.0 * s + i as f32 * row_h;
            let item_rect = Rect::new(x + 4.0 * s, iy, w - 8.0 * s, row_h);
            let item_state = ix.add_zone(item_zone_base + i as u32, item_rect);
            if item_state.is_hovered() {
                painter.rect_filled(item_rect, 4.0 * s, palette.accent.with_alpha(0.12));
            }
            text.queue(branch, font * 0.9, x + pad, iy + (row_h - font * 0.9) * 0.5,
                palette.text, w - pad * 2.0, sw, sh);
        }
    }
}

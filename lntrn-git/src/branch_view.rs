//! Branch dropdown — lists branches, create new, switch on click.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::git;
use crate::keys;

// Zone IDs (must not collide with other views)
const ZONE_BRANCH_BASE: u32 = 4000;
const ZONE_NEW_INPUT: u32 = 4500;
const ZONE_NEW_BTN: u32 = 4501;
const ZONE_SCROLLBAR: u32 = 4502;

/// Actions the branch dropdown can produce.
pub enum BranchAction {
    None,
    /// User clicked a branch to switch to it.
    Switch(String),
    /// User wants to create a new branch with this name.
    Create(String),
}

pub struct BranchDropdown {
    pub open: bool,
    pub branches: Vec<git::BranchInfo>,
    // New branch input
    pub input: String,
    pub input_focused: bool,
    pub cursor_pos: usize,
    // Scroll
    scroll_offset: f32,
    /// The panel rect from the last draw, used for text clipping.
    pub panel_rect: Option<Rect>,
}

impl BranchDropdown {
    pub fn new() -> Self {
        Self {
            open: false,
            branches: Vec::new(),
            input: String::new(),
            input_focused: false,
            cursor_pos: 0,
            scroll_offset: 0.0,
            panel_rect: None,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.input_focused = false;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input_focused = false;
    }

    pub fn wants_keyboard(&self) -> bool {
        self.open && self.input_focused
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if !self.open { return; }
        let count = self.branches.len() as f32;
        let content_h = count * 40.0 + 60.0; // rough estimate
        ScrollArea::apply_scroll(&mut self.scroll_offset, delta, content_h, 300.0);
    }

    pub fn on_key(&mut self, key: u32, shift: bool) -> BranchAction {
        if !self.input_focused { return BranchAction::None; }
        match key {
            keys::KEY_ESC => {
                self.input_focused = false;
            }
            keys::KEY_BACKSPACE => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                }
            }
            keys::KEY_ENTER => {
                if !self.input.trim().is_empty() {
                    let name = self.input.trim().to_string();
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.input_focused = false;
                    return BranchAction::Create(name);
                }
            }
            _ => {
                if let Some(ch) = keys::keycode_to_char(key, shift) {
                    // Branch names: allow alphanumeric, dash, underscore, dot, slash
                    if ch.is_alphanumeric() || "-_./ ".contains(ch) {
                        let ch = if ch == ' ' { '-' } else { ch }; // auto-replace spaces
                        self.input.insert(self.cursor_pos, ch);
                        self.cursor_pos += 1;
                    }
                }
            }
        }
        BranchAction::None
    }

    /// Returns (action, consumed). If consumed is false, the caller should
    /// process the click normally (the dropdown just closed itself).
    pub fn on_click(&mut self, ix: &InteractionContext, px: f32, py: f32) -> (BranchAction, bool) {
        if !self.open { return (BranchAction::None, false); }

        // Check if click is inside the dropdown panel area
        let Some(zone) = ix.zone_at(px, py) else {
            // Clicked outside — close and let the click pass through
            self.close();
            return (BranchAction::None, false);
        };

        // If the zone isn't one of ours, close and pass through
        let is_ours = zone == ZONE_NEW_INPUT
            || zone == ZONE_NEW_BTN
            || zone == ZONE_SCROLLBAR
            || (zone >= ZONE_BRANCH_BASE && zone < ZONE_BRANCH_BASE + 256);
        if !is_ours {
            self.close();
            return (BranchAction::None, false);
        }

        if zone == ZONE_NEW_INPUT {
            self.input_focused = true;
            return (BranchAction::None, true);
        }

        if zone == ZONE_NEW_BTN {
            if !self.input.trim().is_empty() {
                let name = self.input.trim().to_string();
                self.input.clear();
                self.cursor_pos = 0;
                return (BranchAction::Create(name), true);
            }
            return (BranchAction::None, true);
        }

        if zone >= ZONE_BRANCH_BASE && zone < ZONE_BRANCH_BASE + 256 {
            let idx = (zone - ZONE_BRANCH_BASE) as usize;
            if let Some(branch) = self.branches.get(idx) {
                if !branch.is_current {
                    let name = branch.name.clone();
                    self.close();
                    return (BranchAction::Switch(name), true);
                }
            }
            return (BranchAction::None, true);
        }

        (BranchAction::None, true)
    }

    /// Draw the dropdown anchored below `anchor_rect`.
    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        anchor_rect: Rect,
        s: f32, sw: u32, sh: u32,
    ) {
        if !self.open { return; }

        let body_font = 20.0 * s;
        let small_font = 16.0 * s;
        let row_h = 40.0 * s;
        let pad = 12.0 * s;
        let input_h = 40.0 * s;
        let btn_h = 34.0 * s;
        let dropdown_w = 300.0 * s;

        // New branch input row height
        let header_h = input_h + pad * 2.0;

        // Branch list area
        let branch_count = self.branches.len();
        let list_content_h = branch_count as f32 * row_h;
        let max_list_h = 300.0 * s;
        let list_h = list_content_h.min(max_list_h);

        let total_h = header_h + list_h;

        // Position dropdown below the anchor, left-aligned
        let dx = anchor_rect.x;
        let dy = anchor_rect.y + anchor_rect.h + 4.0 * s;

        // Background panel
        let panel_rect = Rect::new(dx, dy, dropdown_w, total_h);
        self.panel_rect = Some(panel_rect);
        painter.rect_filled(panel_rect, 8.0 * s, palette.surface);
        painter.rect_stroke(panel_rect, 8.0 * s, 1.0 * s, palette.muted.with_alpha(0.3));

        // Clip dropdown text to the panel
        text.push_clip([dx, dy, dropdown_w, total_h]);

        // ── New branch input ────────────────────────────────────────────────
        let input_y = dy + pad;
        let input_w = dropdown_w - pad * 2.0 - 60.0 * s;
        let input_rect = Rect::new(dx + pad, input_y, input_w, input_h);
        ix.add_zone(ZONE_NEW_INPUT, input_rect);

        lntrn_ui::gpu::TextInput::new(input_rect)
            .text(&self.input)
            .placeholder("New branch...")
            .focused(self.input_focused)
            .scale(s)
            .cursor_pos(self.cursor_pos)
            .draw(painter, text, palette, sw, sh);

        // Create button
        let btn_w = 50.0 * s;
        let btn_rect = Rect::new(
            dx + dropdown_w - pad - btn_w,
            input_y + (input_h - btn_h) / 2.0,
            btn_w, btn_h,
        );
        let btn_state = ix.add_zone(ZONE_NEW_BTN, btn_rect);
        let btn_color = if self.input.trim().is_empty() {
            palette.muted.with_alpha(0.3)
        } else if btn_state.is_hovered() {
            palette.accent
        } else {
            palette.accent.with_alpha(0.7)
        };
        painter.rect_filled(btn_rect, 6.0 * s, btn_color);
        let ty = btn_rect.y + (btn_h - small_font) / 2.0;
        text.queue("+", body_font,
            btn_rect.x + (btn_w - body_font * 0.5) / 2.0, ty,
            palette.text, btn_w, sw, sh);

        // ── Branch list ─────────────────────────────────────────────────────
        let list_top = dy + header_h;
        let viewport = Rect::new(dx, list_top, dropdown_w, list_h);
        let scroll = ScrollArea::new(viewport, list_content_h, &mut self.scroll_offset);

        scroll.begin(painter, text);

        let base_y = scroll.content_y();
        for (i, branch) in self.branches.iter().enumerate() {
            let fy = base_y + i as f32 * row_h;

            if fy + row_h < list_top || fy > list_top + list_h {
                continue;
            }

            let row_rect = Rect::new(dx, fy, dropdown_w, row_h);
            let zone_id = ZONE_BRANCH_BASE + i as u32;
            let state = ix.add_zone(zone_id, row_rect);

            if branch.is_current {
                painter.rect_filled(row_rect, 4.0 * s, palette.accent.with_alpha(0.15));
            } else if state.is_hovered() {
                painter.rect_filled(row_rect, 4.0 * s, palette.muted.with_alpha(0.12));
            }

            let ty = fy + (row_h - body_font) / 2.0;

            // Current branch indicator
            let indicator = if branch.is_current { "●" } else { "  " };
            let ind_color = if branch.is_current { palette.accent } else { palette.muted };
            text.queue(indicator, small_font, dx + pad, ty + 2.0 * s, ind_color,
                20.0 * s, sw, sh);

            text.queue(&branch.name, body_font, dx + pad + 22.0 * s, ty,
                if branch.is_current { palette.accent } else { palette.text },
                dropdown_w - pad * 2.0 - 22.0 * s, sw, sh);
        }

        scroll.end(painter, text);

        // Scrollbar if needed
        if list_content_h > list_h {
            let scrollbar = Scrollbar::new(&viewport, list_content_h, self.scroll_offset);
            let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
            scrollbar.draw(painter, sb_state, palette);
        }

        text.pop_clip();
    }
}

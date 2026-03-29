//! App menu rendering — draw methods split out from mod.rs.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::scroll::{ScrollArea, Scrollbar};
use lntrn_ui::gpu::text_input::TextInput;
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::svg_icon::IconCache;

use super::{
    launch_app, uninstall_app, AppMenu, MenuTab,
    CELL_SIZE, FOOTER_ICON_SZ,
    ICON_SIZE, LABEL_FONT, PADDING, SEARCH_FLOAT_GAP, SEARCH_HEIGHT,
    TAB_GAP, TAB_SIZE,
    ZONE_BASE, ZONE_CTX, ZONE_POWER, ZONE_TAB,
};

/// Power footer button definitions: (key_name, label, svg_file).
pub(crate) const POWER_ICONS: &[(&str, &str, &str)] = &[
    ("power", "Power", "spark-menu-shutdown.svg"),
    ("reboot", "Reboot", "spark-menu-restart.svg"),
    ("suspend", "Suspend", "spark-menu-sleep.svg"),
    ("lock", "Lock", "spark-menu-lockscreen.svg"),
    ("logout", "Log Out", "spark-menu-logout.svg"),
];

pub(crate) fn initial_color(hash: u32) -> Color {
    let colors = [
        Color::from_rgb8(200, 134, 10),
        Color::from_rgb8(59, 130, 246),
        Color::from_rgb8(34, 197, 94),
        Color::from_rgb8(239, 68, 68),
        Color::from_rgb8(168, 85, 247),
        Color::from_rgb8(236, 72, 153),
    ];
    colors[(hash as usize) % colors.len()]
}

impl AppMenu {
    /// Draw the app menu popup with floating tabs and search.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        icon_cache: &IconCache,
        palette: &FoxPalette,
        bar_x: f32,
        bar_y: f32,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
        icon_draws: &mut Vec<(String, f32, f32, f32, f32, Option<[f32; 4]>)>,
    ) {
        if !self.open { return; }

        let w = self.menu_w * scale;
        let h = self.menu_h * scale;
        let pad = PADDING * scale;

        // Floating row height (search bar a tad taller than tabs)
        let float_h = (TAB_SIZE + 6.0) * scale;
        let float_gap = SEARCH_FLOAT_GAP * scale;

        // Panel position — shifted down to make room for floating row
        let mx = bar_x;
        let my = bar_y - h - pad - float_h - float_gap;
        let panel_y = my + float_h + float_gap;
        self.bounds = Rect::new(mx, panel_y, w, h);

        // Panel background (darker)
        painter.rect_filled(Rect::new(mx, panel_y, w, h), 0.0, palette.bg);
        painter.rect_stroke(Rect::new(mx, panel_y, w, h), 0.0, 2.0 * scale, Color::BLACK);

        // Floating row: search bar (left) + tabs (right), above the panel
        self.draw_floating_row(painter, text, ix, palette, mx, my, w, float_h, scale, screen_w, screen_h);

        // Right-side tabs (Notes, etc.) aligned with panel top
        self.draw_right_tabs(painter, text, ix, palette, mx, panel_y, w, scale, screen_w, screen_h);

        // Floating power icons (bottom-right, below right tabs)
        self.draw_power_icons(painter, ix, icon_cache, palette, mx, panel_y, w, h, scale, icon_draws);

        // Tab content (full panel height)
        match self.active_tab {
            MenuTab::Apps => {
                self.draw_grid(
                    painter, text, ix, icon_cache, palette,
                    mx, panel_y, w, h, pad, scale, screen_w, screen_h, icon_draws,
                );
            }
            MenuTab::SystemMonitor => {
                let area = Rect::new(mx, panel_y, w, h);
                self.sysmon.draw(painter, text, ix, palette, area, scale, screen_w, screen_h);
            }
            MenuTab::Notes => {
                let area = Rect::new(mx, panel_y, w, h);
                self.notes.draw(painter, text, ix, palette, area, scale, screen_w, screen_h);
            }
        }

        // Context menu overlay
        if self.ctx_open {
            self.draw_context_menu(painter, text, ix, palette, scale, screen_w, screen_h);
        }
    }

    /// Draw the floating row above the panel: search bar (left) + tab buttons (right).
    fn draw_floating_row(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        mx: f32, row_y: f32, w: f32, row_h: f32,
        scale: f32, screen_w: u32, screen_h: u32,
    ) {
        let pad = PADDING * scale;
        let tab_sz = TAB_SIZE * scale;
        let gap = TAB_GAP * scale;

        let tabs = MenuTab::TOP;
        let num_tabs = tabs.len();
        let tabs_total_w = num_tabs as f32 * tab_sz + (num_tabs - 1).max(0) as f32 * gap;

        // Search text input (no background rect, just the input widget)
        let search_w = w - tabs_total_w - gap;
        let input_rect = Rect::new(mx + 4.0 * scale, row_y + 3.0 * scale, search_w - 4.0 * scale, row_h - 6.0 * scale);
        TextInput::new(input_rect)
            .text(&self.search)
            .placeholder("Search...")
            .focused(true)
            .draw(painter, text, palette, screen_w, screen_h);

        // Tab buttons: right-aligned in the row
        let tabs_start_x = mx + w - tabs_total_w;
        let tab_cr = 10.0 * scale;

        for (i, &tab) in tabs.iter().enumerate() {
            let tx = tabs_start_x + i as f32 * (tab_sz + gap);
            let ty = row_y + (row_h - tab_sz) * 0.5; // vertically center in row
            let tab_rect = Rect::new(tx, ty, tab_sz, tab_sz);

            let zone_id = ZONE_TAB + i as u32;
            let state = ix.add_zone(zone_id, tab_rect);
            let is_active = self.active_tab == tab;

            if is_active {
                painter.rect_filled(tab_rect, 0.0, palette.surface_2);
                painter.rect_stroke(tab_rect, 0.0, 2.0 * scale, palette.accent);
            } else {
                painter.rect_filled(tab_rect, 0.0, palette.bg);
                painter.rect_stroke(tab_rect, 0.0, 2.0 * scale, Color::BLACK);
                if state.is_hovered() {
                    painter.rect_filled(tab_rect, 0.0, palette.surface_2);
                }
            }

            // Procedural icon
            let color = if is_active { palette.accent } else { palette.text_secondary };
            match tab {
                MenuTab::Apps => {
                    let dot_r = 2.5 * scale;
                    let dot_gap = tab_sz * 0.22;
                    let cx = tx + tab_sz * 0.5;
                    let cy = ty + tab_sz * 0.5;
                    for row in 0..3 {
                        for col in 0..3 {
                            let dx = cx + (col as f32 - 1.0) * dot_gap;
                            let dy = cy + (row as f32 - 1.0) * dot_gap;
                            painter.circle_filled(dx, dy, dot_r, color);
                        }
                    }
                }
                MenuTab::SystemMonitor => {
                    let bar_w = 4.0 * scale;
                    let bar_gap = 3.0 * scale;
                    let base_y = ty + tab_sz * 0.75;
                    let cx = tx + tab_sz * 0.5;
                    let heights = [0.35, 0.55, 0.25, 0.65];
                    let total_w = heights.len() as f32 * bar_w + (heights.len() - 1) as f32 * bar_gap;
                    let start_x = cx - total_w * 0.5;
                    for (j, &frac) in heights.iter().enumerate() {
                        let bx = start_x + j as f32 * (bar_w + bar_gap);
                        let bh = tab_sz * 0.5 * frac;
                        let by = base_y - bh;
                        painter.rect_filled(Rect::new(bx, by, bar_w, bh), 1.5 * scale, color);
                    }
                }
                _ => {} // Right-side tabs drawn separately
            }

            // Tooltip on hover
            if state.is_hovered() {
                let label = match tab {
                    MenuTab::Apps => "Apps",
                    MenuTab::SystemMonitor => "System",
                    _ => "",
                };
                let font = 14.0 * scale;
                let lw = text.measure_width(label, font);
                let lx = tx + (tab_sz - lw) * 0.5;
                let ly = ty - font - 4.0 * scale;
                let tip_rect = Rect::new(lx - 4.0 * scale, ly - 2.0 * scale, lw + 8.0 * scale, font + 4.0 * scale);
                painter.rect_filled(tip_rect, 4.0 * scale, palette.surface_2);
                text.queue(label, font, lx, ly, palette.text, lw + 8.0 * scale, screen_w, screen_h);
            }

            if state == InteractionState::Pressed {
                self.active_tab = tab;
                self.scroll_offset = 0.0;
            }
        }
    }

    /// Draw right-side tabs (Notes, Clipboard, etc.) aligned with panel top.
    fn draw_right_tabs(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        mx: f32, panel_y: f32, w: f32,
        scale: f32, screen_w: u32, screen_h: u32,
    ) {
        let tab_sz = TAB_SIZE * scale;
        let gap = TAB_GAP * scale;
        let bx = mx + w + gap;

        for (i, &tab) in MenuTab::RIGHT.iter().enumerate() {
            let by = panel_y + i as f32 * (tab_sz + gap);
            let tab_rect = Rect::new(bx, by, tab_sz, tab_sz);
            let zone_id = ZONE_TAB + (MenuTab::TOP.len() + i) as u32;
            let state = ix.add_zone(zone_id, tab_rect);
            let is_active = self.active_tab == tab;

            if is_active {
                painter.rect_filled(tab_rect, 0.0, palette.surface_2);
                painter.rect_stroke(tab_rect, 0.0, 2.0 * scale, palette.accent);
            } else {
                painter.rect_filled(tab_rect, 0.0, palette.bg);
                painter.rect_stroke(tab_rect, 0.0, 2.0 * scale, Color::BLACK);
                if state.is_hovered() {
                    painter.rect_filled(tab_rect, 0.0, palette.surface_2);
                }
            }

            let color = if is_active { palette.accent } else { palette.text_secondary };
            match tab {
                MenuTab::Notes => {
                    // Notepad icon: lines of text
                    let lx = bx + tab_sz * 0.25;
                    let lw = tab_sz * 0.5;
                    for j in 0..4 {
                        let ly = by + tab_sz * 0.22 + j as f32 * tab_sz * 0.15;
                        let w = if j == 3 { lw * 0.6 } else { lw };
                        painter.rect_filled(Rect::new(lx, ly, w, 2.0 * scale), 1.0, color);
                    }
                }
                _ => {}
            }

            if state.is_hovered() {
                let label = match tab {
                    MenuTab::Notes => "Notes",
                    _ => "",
                };
                if !label.is_empty() {
                    let font = 14.0 * scale;
                    let lw = text.measure_width(label, font);
                    let lx = bx - lw - 6.0 * scale;
                    let ly = by + (tab_sz - font) * 0.5;
                    let tip_rect = Rect::new(lx - 4.0 * scale, ly - 2.0 * scale, lw + 8.0 * scale, font + 4.0 * scale);
                    painter.rect_filled(tip_rect, 4.0 * scale, palette.surface_2);
                    text.queue(label, font, lx, ly, palette.text, lw + 8.0 * scale, screen_w, screen_h);
                }
            }

            if state == InteractionState::Pressed {
                self.active_tab = tab;
                self.scroll_offset = 0.0;
            }
        }
    }

    /// Floating power icons along the bottom-right edge of the panel.
    #[allow(clippy::too_many_arguments)]
    fn draw_power_icons(
        &self,
        painter: &mut Painter,
        ix: &mut InteractionContext,
        icon_cache: &IconCache,
        palette: &FoxPalette,
        mx: f32, panel_y: f32, w: f32, h: f32,
        scale: f32,
        icon_draws: &mut Vec<(String, f32, f32, f32, f32, Option<[f32; 4]>)>,
    ) {
        let btn_sz = TAB_SIZE * scale;
        let gap = TAB_GAP * scale;
        let icon_sz = FOOTER_ICON_SZ * scale;
        // Position: stacked vertically along the right edge
        let bx = mx + w + gap;
        let start_y = panel_y + h - (POWER_ICONS.len() as f32 * (btn_sz + gap) - gap);

        for (i, (key_name, _label, _svg)) in POWER_ICONS.iter().enumerate() {
            let by = start_y + i as f32 * (btn_sz + gap);
            let btn_rect = Rect::new(bx, by, btn_sz, btn_sz);
            let zone_id = ZONE_POWER + i as u32;
            let state = ix.add_zone(zone_id, btn_rect);

            // Background
            painter.rect_filled(btn_rect, 0.0, palette.surface);
            painter.rect_stroke(btn_rect, 0.0, 2.0 * scale, Color::BLACK);
            if state.is_hovered() {
                painter.rect_filled(btn_rect, 0.0, palette.surface_2);
                painter.rect_stroke(btn_rect, 0.0, 2.0 * scale, Color::BLACK);
            }

            // Icon centered in button
            let icon_key = format!("power_{key_name}");
            let ix_pos = bx + (btn_sz - icon_sz) * 0.5;
            let iy_pos = by + (btn_sz - icon_sz) * 0.5;
            if icon_cache.get(&icon_key).is_some() {
                icon_draws.push((icon_key, ix_pos, iy_pos, icon_sz, icon_sz, None));
            }

            if state == InteractionState::Pressed {
                match *key_name {
                    "power" => { let _ = std::process::Command::new("systemctl").arg("poweroff").spawn(); }
                    "reboot" => { let _ = std::process::Command::new("systemctl").arg("reboot").spawn(); }
                    "suspend" => { let _ = std::process::Command::new("systemctl").arg("suspend").spawn(); }
                    "lock" => { let _ = std::process::Command::new("loginctl").arg("lock-session").spawn(); }
                    "logout" => { let _ = std::process::Command::new("loginctl").arg("terminate-session").arg("self").spawn(); }
                    _ => {}
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_grid(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        icon_cache: &IconCache,
        palette: &FoxPalette,
        mx: f32, my: f32, w: f32, content_h: f32,
        pad: f32,
        scale: f32, screen_w: u32, screen_h: u32,
        icon_draws: &mut Vec<(String, f32, f32, f32, f32, Option<[f32; 4]>)>,
    ) {
        let grid_top = my + pad;
        let grid_h = content_h - pad * 2.0;
        let grid_w = w - pad * 2.0;

        let cell = CELL_SIZE * scale;
        let cols = (grid_w / cell).floor().max(1.0) as usize;
        let filtered = self.filtered_indices();
        let rows = (filtered.len() + cols - 1).max(1) / cols.max(1);
        let total_content_h = rows as f32 * cell;

        let grid_rect = Rect::new(mx + pad, grid_top, grid_w, grid_h);
        let scroll_area = ScrollArea::new(grid_rect, total_content_h, &mut self.scroll_offset);
        scroll_area.begin(painter, text);

        let icon_sz = ICON_SIZE * scale;
        let label_font = LABEL_FONT * scale;
        let clip = [grid_rect.x, grid_rect.y, grid_rect.w, grid_rect.h];

        if filtered.is_empty() {
            let msg = "No apps found";
            let msg_w = text.measure_width(msg, label_font * 1.5);
            let msg_x = grid_rect.x + (grid_w - msg_w) * 0.5;
            let msg_y = grid_rect.y + grid_h * 0.4;
            text.queue(msg, label_font * 1.5, msg_x, msg_y, palette.muted, grid_w, screen_w, screen_h);
        }

        for (i, &entry_idx) in filtered.iter().enumerate() {
            let entry = &self.entries[entry_idx];
            let col = i % cols;
            let row = i / cols;
            let cx = mx + pad + col as f32 * cell;
            let cy = scroll_area.content_y() + row as f32 * cell;

            if cy + cell < grid_top || cy > grid_top + grid_h { continue; }

            let cell_rect = Rect::new(cx, cy, cell, cell);
            let zone_id = ZONE_BASE + i as u32;
            let state = ix.add_zone(zone_id, cell_rect);

            if state.is_hovered() {
                painter.rect_filled(cell_rect, 8.0 * scale, palette.surface_2);
            }

            // Favorite star
            if self.favorites.contains(&entry.app_id) {
                let star_x = cx + cell - 16.0 * scale;
                let star_y = cy + 4.0 * scale;
                text.queue_clipped("*", 14.0 * scale, star_x, star_y, palette.accent, 16.0 * scale, clip);
            }

            let icon_x = cx + (cell - icon_sz) * 0.5;
            let icon_y = cy + 10.0 * scale;

            let icon_key = format!("appmenu_{}", entry.app_id);
            if icon_cache.get(&icon_key).is_some() {
                icon_draws.push((icon_key, icon_x, icon_y, icon_sz, icon_sz, Some(clip)));
            } else {
                let initial = entry.name.chars().next().unwrap_or('?');
                let hue = entry.app_id.bytes().fold(0u32, |a, b| a.wrapping_add(b as u32));
                let bg = initial_color(hue);
                let circ_r = icon_sz * 0.4;
                let circ_cx = icon_x + icon_sz * 0.5;
                let circ_cy = icon_y + icon_sz * 0.5;
                painter.circle_filled(circ_cx, circ_cy, circ_r, bg);
                let init_str = initial.to_uppercase().to_string();
                let init_font = icon_sz * 0.45;
                let init_x = circ_cx - init_font * 0.26;
                let init_y = circ_cy - init_font * 0.5;
                text.queue_clipped(&init_str, init_font, init_x, init_y, Color::WHITE, init_font, clip);
            }

            // Label
            let label = if entry.name.chars().count() > 14 {
                let truncated: String = entry.name.chars().take(12).collect();
                format!("{truncated}...")
            } else {
                entry.name.clone()
            };
            let label_w = text.measure_width(&label, label_font).min(cell - 4.0 * scale);
            let label_x = cx + (cell - label_w) * 0.5;
            let label_y = icon_y + icon_sz + 6.0 * scale;
            text.queue_clipped(
                &label, label_font, label_x, label_y,
                if state.is_hovered() { palette.text } else { palette.text_secondary },
                cell - 4.0 * scale, clip,
            );

            if state == InteractionState::Pressed {
                launch_app(&entry.exec);
                self.close();
            }
        }

        scroll_area.end(painter, text);

        if scroll_area.is_scrollable() {
            let sb = Scrollbar::new(&grid_rect, total_content_h, self.scroll_offset);
            sb.draw(painter, InteractionState::Idle, palette);
        }
    }

    fn draw_context_menu(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        scale: f32,
        screen_w: u32, screen_h: u32,
    ) {
        let Some(app_id) = self.ctx_app_id.clone() else { return; };
        let is_fav = self.favorites.contains(&app_id);

        let item_h = 40.0 * scale;
        let menu_w = 200.0 * scale;
        let menu_h = item_h * 2.0 + 8.0 * scale;
        let cr = 10.0 * scale;
        let pad = 4.0 * scale;

        let ctx_x = self.ctx_pos.0.min(self.bounds.x + self.bounds.w - menu_w - pad);
        let ctx_y = self.ctx_pos.1.min(self.bounds.y + self.bounds.h - menu_h - pad);

        painter.rect_filled(Rect::new(ctx_x, ctx_y, menu_w, menu_h), 0.0, palette.surface_2);
        painter.rect_stroke(Rect::new(ctx_x, ctx_y, menu_w, menu_h), 0.0, 2.0 * scale, Color::BLACK);

        // Punch a hole in underlying text so it doesn't bleed through
        text.occlude_rect([ctx_x, ctx_y, menu_w, menu_h]);

        let font = 20.0 * scale;
        let items: [(&str, u32); 2] = [
            (if is_fav { "Remove Favorite" } else { "Add to Favorites" }, ZONE_CTX),
            ("Uninstall", ZONE_CTX + 1),
        ];

        let mut iy = ctx_y + pad;
        for (label, zone_id) in &items {
            let item_rect = Rect::new(ctx_x + pad, iy, menu_w - pad * 2.0, item_h);
            let state = ix.add_zone(*zone_id, item_rect);

            if state.is_hovered() {
                painter.rect_filled(item_rect, 6.0 * scale, Color::rgba(0.45, 0.30, 0.05, 0.35));
            }

            let text_x = ctx_x + 14.0 * scale;
            let text_y = iy + (item_h - font) * 0.5;
            let color = if *label == "Uninstall" { Color::from_rgb8(239, 68, 68) } else { palette.text };
            text.queue(label, font, text_x, text_y, color, menu_w - 28.0 * scale, screen_w, screen_h);

            if state == InteractionState::Pressed {
                match *zone_id {
                    ZONE_CTX => {
                        self.toggle_favorite(&app_id);
                    }
                    z if z == ZONE_CTX + 1 => {
                        uninstall_app(&app_id);
                    }
                    _ => {}
                }
                self.ctx_open = false;
            }

            iy += item_h;
        }
    }

    /// Draw the launcher button (waffle grid icon) in the bar.
    pub fn draw_button(
        &self,
        painter: &mut Painter,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        x: f32, y: f32, bar_h: f32, scale: f32,
    ) -> f32 {
        let pad_left = 10.0 * scale;
        let btn_size = bar_h;
        let btn_rect = Rect::new(x + pad_left, y, btn_size, btn_size);
        let zone_id = ZONE_BASE + 0xFFFF;
        let state = ix.add_zone(zone_id, btn_rect);

        if state.is_hovered() || self.open {
            painter.rect_filled(btn_rect, 8.0 * scale, palette.muted.with_alpha(0.35));
        }

        let grid_size = bar_h * 0.65;
        let dot_r = grid_size / 6.0;
        let gap = grid_size / 3.0;
        let cx = x + pad_left + btn_size * 0.5;
        let cy = y + bar_h * 0.5;
        let color = if self.open { palette.accent } else { palette.text_secondary };

        for row in 0..3 {
            for col in 0..3 {
                let dx = cx + (col as f32 - 1.0) * gap;
                let dy = cy + (row as f32 - 1.0) * gap;
                painter.circle_filled(dx, dy, dot_r, color);
            }
        }

        pad_left + btn_size
    }
}

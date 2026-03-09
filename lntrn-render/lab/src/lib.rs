mod layout;
mod sections;

use std::time::Instant;

use lntrn_render::{Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, Fill, FoxPalette, GradientTopBar, InteractionContext,
    MenuEvent, MenuItem, Modal, ModalButton, Panel, ScrollArea, Scrollbar, TextLabel, FontSize,
    TitleBar, ToastAnchor, ToastItem, ToastStack,
};

use layout::*;
use sections::*;

pub enum LabCommand {
    ResetSlider,
}

pub struct RendererLab {
    ix: InteractionContext,
    scroll_offset: f32,

    // Global controls
    bg_alpha: f32,
    font_scale: f32,

    // Widget state
    slider_value: f32,
    checkbox_states: [bool; 3],
    toggle_states: [bool; 2],
    radio_selected: usize,
    text_input_value: String,
    focused_input: Option<u32>,
    dropdown_open: bool,
    dropdown_selected: usize,

    // Overlays
    show_modal: bool,
    toasts: Vec<ToastItem>,
    context_menu: ContextMenu,

    // Scroll demo (inner)
    scroll_demo_offset: f32,

    // Animation
    start_time: Instant,
    last_frame: Instant,
    toast_auto_timer: f32,
}

impl RendererLab {
    pub fn new() -> Self {
        let style = ContextMenuStyle::from_palette(&FoxPalette::dark());
        let now = Instant::now();
        Self {
            ix: InteractionContext::new(),
            scroll_offset: 0.0,
            bg_alpha: 1.0,
            font_scale: 1.0,
            slider_value: 0.62,
            checkbox_states: [true, false, false],
            toggle_states: [true, false],
            radio_selected: 1,
            text_input_value: String::from("Hello Lantern"),
            focused_input: None,
            dropdown_open: false,
            dropdown_selected: 0,
            show_modal: false,
            toasts: Vec::new(),
            context_menu: ContextMenu::new(style),
            scroll_demo_offset: 0.0,
            start_time: now,
            last_frame: now,
            toast_auto_timer: 0.0,
        }
    }

    pub fn render(
        &mut self,
        size: (u32, u32),
        gpu: &GpuContext,
        painter: &mut Painter,
        text: &mut TextRenderer,
    ) -> Result<(), SurfaceError> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }

        // ── Timing ─────────────────────────────────────────────────────
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let anim_time = self.start_time.elapsed().as_secs_f32();

        // Update toasts
        self.toasts.retain_mut(|t| {
            t.progress -= dt / 3.0; // 3 second lifetime
            t.progress > 0.0
        });

        // Auto-spawn toast every 5s
        self.toast_auto_timer += dt;
        if self.toast_auto_timer > 5.0 {
            self.toast_auto_timer = 0.0;
            if self.toasts.len() < 4 {
                let msgs = ["File saved!", "Connection OK", "Build complete", "Synced"];
                let variants = [
                    ToastItem::success, ToastItem::info,
                    ToastItem::success, ToastItem::info,
                ];
                let idx = (anim_time as usize / 5) % msgs.len();
                self.toasts.push(variants[idx](msgs[idx]));
            }
        }

        // ── Setup ──────────────────────────────────────────────────────
        self.ix.begin_frame();
        let (sw, sh) = size;
        let fox = FoxPalette::dark();
        let panel = panel_rect(size);
        let viewport = viewport_rect(panel);
        let cw = panel.w - CONTENT_PAD * 2.0;
        let cx = panel.x + CONTENT_PAD;

        painter.clear();

        // Window background (transparency!)
        painter.rect_filled(
            Rect::new(0.0, 0.0, sw as f32, sh as f32),
            0.0,
            fox.bg.with_alpha(self.bg_alpha),
        );

        // Panel
        Panel::new(panel)
            .fill(Fill::Solid(fox.surface.with_alpha(self.bg_alpha * 0.85)))
            .radius(18.0)
            .draw(painter);
        painter.rect_stroke(panel, 18.0, 1.0, fox.muted.with_alpha(0.3));

        // Accent strip
        GradientTopBar::new(panel.x, panel.y + TITLE_BAR_H, panel.w).draw(painter);

        // Title bar
        let tb = TitleBar::new(title_bar_rect(panel), "");
        let min_s = self.ix.add_zone(Z_MINIMIZE, tb.minimize_button_rect());
        let max_s = self.ix.add_zone(Z_MAXIMIZE, tb.maximize_button_rect());
        let cls_s = self.ix.add_zone(Z_CLOSE, tb.close_button_rect());
        TitleBar::new(title_bar_rect(panel), "Lantern Lab")
            .minimize_hovered(min_s.is_hovered())
            .maximize_hovered(max_s.is_hovered())
            .close_hovered(cls_s.is_hovered())
            .draw(painter, text, &fox, sw, sh);

        // Re-draw strip on top
        GradientTopBar::new(panel.x, panel.y + TITLE_BAR_H, panel.w).draw(painter);

        // ── Scrollable content ─────────────────────────────────────────
        let total_h = total_content_height();
        let vp_top = viewport.y;
        let vp_bot = viewport.y + viewport.h;

        // Main scroll from wheel
        let scroll_delta = self.ix.scroll_delta();
        // Only scroll main if cursor is NOT in scroll demo area
        let in_scroll_demo = false; // simplified - scroll demo uses its own zone
        if scroll_delta != 0.0 && !in_scroll_demo {
            ScrollArea::apply_scroll(
                &mut self.scroll_offset, scroll_delta * 40.0,
                total_h, viewport.h,
            );
        }

        painter.push_clip(viewport);

        let mut y = viewport.y + CONTENT_PAD - self.scroll_offset;

        y += draw_global_controls(
            painter, text, &fox, &mut self.ix,
            cx, y, cw, self.bg_alpha, self.font_scale,
            sw, sh, vp_top, vp_bot,
        );
        y += SECTION_GAP;

        y += draw_typography(painter, text, &fox, cx, y, cw, sw, sh, vp_top, vp_bot);
        y += SECTION_GAP;

        y += draw_buttons(painter, text, &fox, &mut self.ix, cx, y, cw, sw, sh, vp_top, vp_bot);
        y += SECTION_GAP;

        y += draw_slider(
            painter, text, &fox, &mut self.ix,
            cx, y, cw, self.slider_value, sw, sh, vp_top, vp_bot,
        );
        y += SECTION_GAP;

        y += draw_selection(
            painter, text, &fox, &mut self.ix,
            cx, y, cw,
            self.checkbox_states, self.toggle_states, self.radio_selected,
            sw, sh, vp_top, vp_bot,
        );
        y += SECTION_GAP;

        y += draw_inputs(
            painter, text, &fox, &mut self.ix,
            cx, y, cw,
            &self.text_input_value, self.focused_input,
            self.dropdown_open, self.dropdown_selected,
            sw, sh, vp_top, vp_bot,
        );
        y += SECTION_GAP;

        y += draw_badges_progress(painter, text, &fox, cx, y, cw, sw, sh, vp_top, vp_bot);
        y += SECTION_GAP;

        y += draw_scroll_demo(
            painter, text, &fox, &mut self.ix,
            cx, y, cw, &mut self.scroll_demo_offset,
            sw, sh, vp_top, vp_bot,
        );
        y += SECTION_GAP;

        y += draw_actions(painter, text, &fox, &mut self.ix, cx, y, cw, sw, sh, vp_top, vp_bot);
        y += SECTION_GAP;

        draw_animations(painter, text, &fox, cx, y, cw, anim_time, sw, sh, vp_top, vp_bot);

        painter.pop_clip();

        // ── Main scrollbar ─────────────────────────────────────────────
        let scrollbar = Scrollbar::new(&viewport, total_h, self.scroll_offset);
        let sb_state = self.ix.add_zone(Z_MAIN_SCROLL, scrollbar.thumb);
        if sb_state.is_active() {
            if let Some((_, sy)) = self.ix.cursor() {
                self.scroll_offset =
                    scrollbar.offset_for_thumb_y(sy, total_h, viewport.h);
            }
        }
        scrollbar.draw(painter, sb_state, &fox);

        // ── Footer hint ────────────────────────────────────────────────
        TextLabel::new(
            "Right-click for context menu · Scroll to explore · ESC to quit",
            panel.x + CONTENT_PAD,
            panel.y + panel.h - 30.0,
        )
            .size(FontSize::Label)
            .color(fox.muted.with_alpha(0.5))
            .max_width(panel.w - CONTENT_PAD * 2.0)
            .draw(text, sw, sh);

        // ── Context menu (overlay) ─────────────────────────────────────
        if let Some(evt) = self.context_menu.draw(painter, text, &mut self.ix, sw, sh) {
            match evt {
                MenuEvent::Action(1) => { self.slider_value = 0.5; self.context_menu.close(); }
                MenuEvent::Action(2) => { self.bg_alpha = 1.0; self.context_menu.close(); }
                _ => self.context_menu.close(),
            }
        }

        // ── Modal (overlay) ────────────────────────────────────────────
        if self.show_modal {
            let m = Modal::new(sw as f32, sh as f32)
                .title("Delete file?")
                .body("This action cannot be undone. The file will be permanently removed.")
                .button(ModalButton::new(Z_MODAL_CANCEL, "Cancel"))
                .button(ModalButton::new(Z_MODAL_CONFIRM, "Delete").primary());

            // Register button zones
            let cancel_s = m.button_rect(Z_MODAL_CANCEL)
                .map(|r| self.ix.add_zone(Z_MODAL_CANCEL, r));
            let confirm_s = m.button_rect(Z_MODAL_CONFIRM)
                .map(|r| self.ix.add_zone(Z_MODAL_CONFIRM, r));

            let hovered = if confirm_s.map_or(false, |s| s.is_hovered()) {
                Some(Z_MODAL_CONFIRM)
            } else if cancel_s.map_or(false, |s| s.is_hovered()) {
                Some(Z_MODAL_CANCEL)
            } else {
                None
            };

            Modal::new(sw as f32, sh as f32)
                .title("Delete file?")
                .body("This action cannot be undone. The file will be permanently removed.")
                .button(ModalButton::new(Z_MODAL_CANCEL, "Cancel"))
                .button(ModalButton::new(Z_MODAL_CONFIRM, "Delete").primary())
                .hovered_button(hovered)
                .draw(painter, text, &fox, sw, sh);
        }

        // ── Toasts (overlay) ──────────────────────────────────────────
        if !self.toasts.is_empty() {
            ToastStack::new(&self.toasts)
                .anchor(ToastAnchor::BottomRight)
                .margin(40.0)
                .draw(painter, text, &fox, sw, sh);
        }

        // ── GPU frame ──────────────────────────────────────────────────
        let mut frame = gpu.begin_frame("Lab")?;
        let view = frame.view().clone();
        painter.render_pass(gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
        text.render_queued(gpu, frame.encoder_mut(), &view);
        frame.submit(&gpu.queue);
        Ok(())
    }

    // ── Event handlers ───────────────────────────────────────────────────

    pub fn on_cursor_moved(&mut self, size: (u32, u32), x: f32, y: f32) {
        self.ix.on_cursor_moved(x, y);

        // Slider drags
        if self.ix.active_zone_id() == Some(Z_SLIDER) {
            let panel = panel_rect(size);
            let cx = panel.x + CONTENT_PAD;
            let cw = panel.w - CONTENT_PAD * 2.0;
            self.slider_value = slider_value_for_x(x, cx, cw);
        }
        if self.ix.active_zone_id() == Some(Z_TRANSPARENCY) {
            let panel = panel_rect(size);
            let pad = SECTION_PAD;
            let slider_w = (panel.w - CONTENT_PAD * 2.0 - pad * 2.0) * 0.45;
            let sr_x = panel.x + CONTENT_PAD + pad;
            self.bg_alpha = ((x - sr_x) / slider_w.max(1.0)).clamp(0.0, 1.0);
        }
        if self.ix.active_zone_id() == Some(Z_TEXT_SIZE) {
            let panel = panel_rect(size);
            let pad = SECTION_PAD;
            let slider_w = (panel.w - CONTENT_PAD * 2.0 - pad * 2.0) * 0.45;
            let col_offset = slider_w + 40.0;
            let sr_x = panel.x + CONTENT_PAD + pad + col_offset;
            let frac = ((x - sr_x) / slider_w.max(1.0)).clamp(0.0, 1.0);
            self.font_scale = 0.5 + frac * 1.5; // 0.5x to 2.0x
        }
    }

    pub fn on_right_pressed(&mut self, size: (u32, u32)) {
        if let Some((x, y)) = self.ix.cursor() {
            self.context_menu.open(x, y, vec![
                MenuItem::action(1, "Reset Slider"),
                MenuItem::action(2, "Reset Transparency"),
                MenuItem::separator(),
                MenuItem::submenu(100, "View", vec![
                    MenuItem::action(101, "Zoom In"),
                    MenuItem::action(102, "Zoom Out"),
                    MenuItem::action(103, "Reset Zoom"),
                ]),
                MenuItem::submenu(200, "Theme", vec![
                    MenuItem::action(201, "Dark"),
                    MenuItem::action(202, "Light"),
                    MenuItem::submenu(210, "Accent", vec![
                        MenuItem::action(211, "Gold"),
                        MenuItem::action(212, "Blue"),
                        MenuItem::action(213, "Red"),
                    ]),
                ]),
                MenuItem::separator(),
                MenuItem::action(99, "Close"),
            ]);
            self.context_menu.clamp_to_screen(size.0 as f32, size.1 as f32);
        }
    }

    pub fn on_left_pressed(&mut self, size: (u32, u32)) -> bool {
        // Dismiss context menu on click outside
        if self.context_menu.is_open() {
            if let Some((x, y)) = self.ix.cursor() {
                if !self.context_menu.contains(x, y) {
                    self.context_menu.close();
                    return false;
                }
            }
        }

        // Dismiss dropdown on click outside
        if self.dropdown_open {
            if let Some((_px, _py)) = self.ix.cursor() {
                let panel = panel_rect(size);
                let _cx = panel.x + CONTENT_PAD;
                // Simple: clicking anywhere that's not a dropdown zone closes it
                let hit = self.ix.on_left_pressed();
                if let Some(id) = hit {
                    if id == Z_DROPDOWN {
                        self.dropdown_open = !self.dropdown_open;
                        self.ix.on_left_released();
                        return false;
                    }
                    let opt_count = dropdown_option_count();
                    if id >= Z_DROPDOWN_OPT && id < Z_DROPDOWN_OPT + opt_count as u32 {
                        self.dropdown_selected = (id - Z_DROPDOWN_OPT) as usize;
                        self.dropdown_open = false;
                        self.ix.on_left_released();
                        return false;
                    }
                }
                // Click outside dropdown — close it
                self.dropdown_open = false;
                // Fall through to handle other clicks
                // (hit was already computed above)
                return self.handle_hit(hit, size);
            }
        }

        let hit = self.ix.on_left_pressed();

        if self.context_menu.is_open() {
            return false;
        }

        self.handle_hit(hit, size)
    }

    fn handle_hit(&mut self, hit: Option<u32>, _size: (u32, u32)) -> bool {
        // Modal interactions
        if self.show_modal {
            if hit == Some(Z_MODAL_CANCEL) || hit == Some(Z_MODAL_CONFIRM) {
                self.show_modal = false;
                self.ix.on_left_released();
                return false;
            }
            // Click on backdrop dismisses modal
            self.show_modal = false;
            self.ix.on_left_released();
            return false;
        }

        if hit == Some(Z_CLOSE) { return true; }

        // Dropdown toggle
        if hit == Some(Z_DROPDOWN) {
            self.dropdown_open = !self.dropdown_open;
            self.ix.on_left_released();
            return false;
        }

        // Dropdown option
        let opt_count = dropdown_option_count();
        if let Some(id) = hit {
            if id >= Z_DROPDOWN_OPT && id < Z_DROPDOWN_OPT + opt_count as u32 {
                self.dropdown_selected = (id - Z_DROPDOWN_OPT) as usize;
                self.dropdown_open = false;
                self.ix.on_left_released();
                return false;
            }
        }

        // Checkboxes
        for i in 0..3u32 {
            if hit == Some(Z_CB_BASE + i) && i < 2 {
                self.checkbox_states[i as usize] = !self.checkbox_states[i as usize];
                self.ix.on_left_released();
                return false;
            }
        }

        // Toggles
        for i in 0..2u32 {
            if hit == Some(Z_TOGGLE_BASE + i) {
                self.toggle_states[i as usize] = !self.toggle_states[i as usize];
                self.ix.on_left_released();
                return false;
            }
        }

        // Radio buttons
        for i in 0..3u32 {
            if hit == Some(Z_RADIO_BASE + i) {
                self.radio_selected = i as usize;
                self.ix.on_left_released();
                return false;
            }
        }

        // Text input focus
        for i in 0..3u32 {
            if hit == Some(Z_INPUT_BASE + i) {
                self.focused_input = hit;
                self.ix.on_left_released();
                return false;
            }
        }

        // Modal open
        if hit == Some(Z_MODAL_OPEN) {
            self.show_modal = true;
            self.ix.on_left_released();
            return false;
        }

        // Toast spawn
        if hit == Some(Z_TOAST_SPAWN) {
            let variants = [
                ToastItem::success("Manually spawned!"),
                ToastItem::warning("Watch out!"),
                ToastItem::error("Something broke!"),
                ToastItem::info("Did you know?"),
            ];
            let idx = self.toasts.len() % variants.len();
            if self.toasts.len() < 5 {
                self.toasts.push(variants[idx].clone());
            }
            self.ix.on_left_released();
            return false;
        }

        // Clear input focus on click elsewhere
        if self.focused_input.is_some() {
            self.focused_input = None;
        }

        false
    }

    pub fn on_left_released(&mut self) {
        self.ix.on_left_released();
    }

    pub fn on_scroll(&mut self, delta: f32) {
        self.ix.on_scroll(delta);
    }

    pub fn on_cursor_left(&mut self) {
        self.ix.on_cursor_left();
    }

    pub fn on_command(&mut self, command: LabCommand) {
        match command {
            LabCommand::ResetSlider => self.slider_value = 0.5,
        }
    }
}

fn total_content_height() -> f32 {
    SEC_GLOBAL_H + SEC_TYPO_H + SEC_BUTTONS_H + SEC_SLIDER_H
        + SEC_SELECTION_H + SEC_INPUTS_H + SEC_BADGES_H
        + SEC_SCROLL_H + SEC_ACTIONS_H + SEC_ANIMS_H
        + SECTION_GAP * 9.0
        + CONTENT_PAD * 2.0
}

impl Default for RendererLab {
    fn default() -> Self {
        Self::new()
    }
}

mod layout;
mod sections;

use std::time::Instant;

use lntrn_render::{GpuContext, Painter, Rect, SurfaceError, TextRenderer};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext,
    MenuEvent, MenuItem, Modal, ModalButton, ScrollArea, Scrollbar, TitleBar,
};

use layout::*;
use sections::*;

pub struct RendererLab {
    ix: InteractionContext,
    scroll_offset: f32,

    // Widget state
    text_input_value: String,
    input_focused: bool,
    dropdown_open: bool,
    dropdown_selected: usize,
    toggles: [bool; 2],
    checkboxes: [bool; 2],
    radio_selected: u32,
    slider_value: f32,

    // Overlays
    show_modal: bool,
    context_menu: ContextMenu,

    // Scroll demo
    scroll_demo_offset: f32,

    // Animation
    start_time: Instant,
    last_frame: Instant,
}

impl RendererLab {
    pub fn new() -> Self {
        let style = ContextMenuStyle::from_palette(&FoxPalette::dark());
        let now = Instant::now();
        Self {
            ix: InteractionContext::new(),
            scroll_offset: 0.0,
            text_input_value: String::from("Hello Lantern"),
            input_focused: false,
            dropdown_open: false,
            dropdown_selected: 0,
            toggles: [true, false],
            checkboxes: [false, false],
            radio_selected: 1,
            slider_value: 0.65,
            show_modal: false,
            context_menu: ContextMenu::new(style),
            scroll_demo_offset: 0.0,
            start_time: now,
            last_frame: now,
        }
    }

    pub fn render(
        &mut self,
        size: (u32, u32),
        gpu: &GpuContext,
        painter: &mut Painter,
        text: &mut TextRenderer,
    ) -> Result<(), SurfaceError> {
        if size.0 == 0 || size.1 == 0 { return Ok(()); }

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let anim_time = self.start_time.elapsed().as_secs_f32();

        self.ix.begin_frame();
        let (sw, sh) = size;
        let w = sw as f32;
        let h = sh as f32;
        let fox = FoxPalette::dark();
        let viewport = viewport_rect(w, h);

        painter.clear();
        painter.rect_filled(Rect::new(0.0, 0.0, w, h), 0.0, fox.bg);

        // ── Title bar ────────────────────────────────────────────────────
        let tb_rect = Rect::new(0.0, 0.0, w, layout::TITLE_BAR_H);
        let tb_close = self.ix.add_zone(Z_TB_CLOSE, TitleBar::new(tb_rect, "").close_button_rect());
        let tb_max = self.ix.add_zone(Z_TB_MAXIMIZE, TitleBar::new(tb_rect, "").maximize_button_rect());
        let tb_min = self.ix.add_zone(Z_TB_MINIMIZE, TitleBar::new(tb_rect, "").minimize_button_rect());
        TitleBar::new(tb_rect, "Lantern Lab")
            .close_hovered(tb_close.is_hovered())
            .maximize_hovered(tb_max.is_hovered())
            .minimize_hovered(tb_min.is_hovered())
            .draw(painter, text, &fox, sw, sh);

        // ── Scroll ─────────────────────────────────────────────────────
        let total_h = self.total_content_height(w);
        let scroll_delta = self.ix.scroll_delta();
        if scroll_delta != 0.0 {
            ScrollArea::apply_scroll(
                &mut self.scroll_offset, scroll_delta * 40.0,
                total_h, viewport.h,
            );
        }

        let vt = viewport.y;
        let vb = viewport.y + viewport.h;

        painter.push_clip(viewport);

        // ── Grid layout ────────────────────────────────────────────────
        let lx = col_left();
        let rx = col_right(w);
        let cw = col_w(w);
        let fw = full_w(w);
        let mut y = viewport.y + CONTENT_PAD - self.scroll_offset;

        // Row 1: Buttons | Controls
        let row1_h = CARD_BUTTONS_H.max(CARD_CONTROLS_H);
        draw_buttons(painter, text, &fox, &mut self.ix, lx, y, cw, sw, sh, vt, vb);
        draw_controls(
            painter, text, &fox, &mut self.ix, rx, y, cw,
            self.toggles, self.checkboxes, self.radio_selected,
            sw, sh, vt, vb,
        );
        y += row1_h + CARD_GAP;

        // Row 2: Text Input | Dropdown
        let row2_h = CARD_INPUT_H.max(CARD_DROPDOWN_H);
        draw_text_input(
            painter, text, &fox, &mut self.ix, lx, y, cw,
            &self.text_input_value, self.input_focused,
            sw, sh, vt, vb,
        );
        draw_dropdown(
            painter, text, &fox, &mut self.ix, rx, y, cw,
            self.dropdown_open, self.dropdown_selected,
            sw, sh, vt, vb,
        );
        y += row2_h + CARD_GAP;

        // Row 3: Slider & Progress | Swatches
        let row3_h = CARD_SLIDER_H.max(CARD_SWATCHES_H);
        draw_slider_progress(
            painter, text, &fox, &mut self.ix, lx, y, cw,
            self.slider_value, anim_time,
            sw, sh, vt, vb,
        );
        draw_swatches(painter, text, &fox, rx, y, cw, sw, sh, vt, vb);
        y += row3_h + CARD_GAP;

        // Row 4: Badges | Scroll Area
        let row4_h = CARD_BADGES_H.max(CARD_SCROLL_H);
        draw_badges(painter, text, &fox, lx, y, cw, sw, sh, vt, vb);
        draw_scroll_demo(
            painter, text, &fox, &mut self.ix, rx, y, cw,
            &mut self.scroll_demo_offset, sw, sh, vt, vb,
        );
        y += row4_h + CARD_GAP;

        // Row 5: Modal (full width)
        draw_modal_trigger(
            painter, text, &fox, &mut self.ix,
            lx, y, fw, sw, sh, vt, vb,
        );

        painter.pop_clip();

        // ── Scrollbar ──────────────────────────────────────────────────
        let scrollbar = Scrollbar::new(&viewport, total_h, self.scroll_offset);
        let sb_state = self.ix.add_zone(Z_MAIN_SCROLL, scrollbar.thumb);
        if sb_state.is_active() {
            if let Some((_, sy)) = self.ix.cursor() {
                self.scroll_offset =
                    scrollbar.offset_for_thumb_y(sy, total_h, viewport.h);
            }
        }
        scrollbar.draw(painter, sb_state, &fox);

        // ── Base render pass ───────────────────────────────────────────
        let mut frame = gpu.begin_frame("Lab")?;
        let view = frame.view().clone();
        painter.render_pass(gpu, frame.encoder_mut(), &view, fox.bg);
        text.render_queued(gpu, frame.encoder_mut(), &view);

        // ── Overlays ───────────────────────────────────────────────────
        painter.clear();

        self.context_menu.update(dt);
        if let Some(evt) = self.context_menu.draw(painter, text, &mut self.ix, sw, sh) {
            if let MenuEvent::Action(_) = evt {
                self.context_menu.close();
            }
        }

        if self.show_modal {
            let m = Modal::new(w, h)
                .title("Delete file?")
                .body("This action cannot be undone.")
                .button(ModalButton::new(Z_MODAL_CANCEL, "Cancel"))
                .button(ModalButton::new(Z_MODAL_CONFIRM, "Delete").primary());

            let cancel_s = m.button_rect(Z_MODAL_CANCEL)
                .map(|r| self.ix.add_zone(Z_MODAL_CANCEL, r));
            let confirm_s = m.button_rect(Z_MODAL_CONFIRM)
                .map(|r| self.ix.add_zone(Z_MODAL_CONFIRM, r));

            let hovered = if confirm_s.map_or(false, |s| s.is_hovered()) {
                Some(Z_MODAL_CONFIRM)
            } else if cancel_s.map_or(false, |s| s.is_hovered()) {
                Some(Z_MODAL_CANCEL)
            } else { None };

            Modal::new(w, h)
                .title("Delete file?")
                .body("This action cannot be undone.")
                .button(ModalButton::new(Z_MODAL_CANCEL, "Cancel"))
                .button(ModalButton::new(Z_MODAL_CONFIRM, "Delete").primary())
                .hovered_button(hovered)
                .draw(painter, text, &fox, sw, sh);
        }

        painter.render_pass_overlay(gpu, frame.encoder_mut(), &view);
        text.render_queued(gpu, frame.encoder_mut(), &view);
        frame.submit(&gpu.queue);
        self.ix.clear_scroll();
        Ok(())
    }

    fn total_content_height(&self, _w: f32) -> f32 {
        let row1 = CARD_BUTTONS_H.max(CARD_CONTROLS_H);
        let row2 = CARD_INPUT_H.max(CARD_DROPDOWN_H);
        let row3 = CARD_SLIDER_H.max(CARD_SWATCHES_H);
        let row4 = CARD_BADGES_H.max(CARD_SCROLL_H);
        let row5 = CARD_MODAL_H;
        row1 + row2 + row3 + row4 + row5
            + CARD_GAP * 4.0
            + CONTENT_PAD * 2.0
    }

    // ── Events ─────────────────────────────────────────────────────────

    pub fn on_cursor_moved(&mut self, size: (u32, u32), x: f32, y: f32) {
        self.ix.on_cursor_moved(x, y);

        if self.ix.active_zone_id() == Some(Z_SLIDER) {
            let w = size.0 as f32;
            let lx = col_left();
            let cw = col_w(w);
            let pad = CARD_PAD;
            let track_w = cw - pad * 2.0;
            let sr_x = lx + pad;
            self.slider_value = ((x - sr_x) / track_w.max(1.0)).clamp(0.0, 1.0);
        }
    }

    pub fn on_right_pressed(&mut self, size: (u32, u32)) {
        if let Some((x, y)) = self.ix.cursor() {
            self.context_menu.open(x, y, vec![
                MenuItem::header("Actions"),
                MenuItem::action(1, "Reset All"),
                MenuItem::separator(),
                MenuItem::submenu(100, "Theme", vec![
                    MenuItem::action(101, "Dark"),
                    MenuItem::action(102, "Light"),
                ]),
                MenuItem::separator(),
                MenuItem::toggle(10, "Auto-save", true),
                MenuItem::checkbox(11, "Show Grid", false),
                MenuItem::separator(),
                MenuItem::radio(20, 1, "Normal", true),
                MenuItem::radio(21, 1, "Compact", false),
                MenuItem::separator(),
                MenuItem::slider(30, "Opacity", 0.8),
            ]);
            self.context_menu.clamp_to_screen(size.0 as f32, size.1 as f32);
        }
    }

    pub fn on_left_pressed(&mut self, _size: (u32, u32)) -> bool {
        if self.context_menu.is_open() {
            if let Some((x, y)) = self.ix.cursor() {
                if !self.context_menu.contains(x, y) {
                    self.context_menu.close();
                    return false;
                }
            }
        }

        if self.dropdown_open {
            let hit = self.ix.on_left_pressed();
            if let Some(id) = hit {
                if id == Z_DROPDOWN {
                    self.dropdown_open = !self.dropdown_open;
                    self.ix.on_left_released();
                    return false;
                }
                let n = dropdown_option_count();
                if id >= Z_DROPDOWN_OPT && id < Z_DROPDOWN_OPT + n as u32 {
                    self.dropdown_selected = (id - Z_DROPDOWN_OPT) as usize;
                    self.dropdown_open = false;
                    self.ix.on_left_released();
                    return false;
                }
            }
            self.dropdown_open = false;
            return self.handle_hit(hit);
        }

        let hit = self.ix.on_left_pressed();
        if self.context_menu.is_open() { return false; }
        self.handle_hit(hit)
    }

    fn handle_hit(&mut self, hit: Option<u32>) -> bool {
        if self.show_modal {
            self.show_modal = false;
            self.ix.on_left_released();
            return false;
        }

        match hit {
            Some(Z_TB_CLOSE) => { self.ix.on_left_released(); return true; }
            Some(Z_DROPDOWN) => {
                self.dropdown_open = !self.dropdown_open;
                self.ix.on_left_released();
            }
            Some(id) if id >= Z_DROPDOWN_OPT
                && id < Z_DROPDOWN_OPT + dropdown_option_count() as u32 =>
            {
                self.dropdown_selected = (id - Z_DROPDOWN_OPT) as usize;
                self.dropdown_open = false;
                self.ix.on_left_released();
            }
            Some(Z_TOGGLE_A) => { self.toggles[0] = !self.toggles[0]; self.ix.on_left_released(); }
            Some(Z_TOGGLE_B) => { self.toggles[1] = !self.toggles[1]; self.ix.on_left_released(); }
            Some(Z_CHECKBOX_A) => { self.checkboxes[0] = !self.checkboxes[0]; self.ix.on_left_released(); }
            Some(Z_CHECKBOX_B) => { self.checkboxes[1] = !self.checkboxes[1]; self.ix.on_left_released(); }
            Some(Z_RADIO_A) => { self.radio_selected = 0; self.ix.on_left_released(); }
            Some(Z_RADIO_B) => { self.radio_selected = 1; self.ix.on_left_released(); }
            Some(Z_RADIO_C) => { self.radio_selected = 2; self.ix.on_left_released(); }
            Some(Z_INPUT) => { self.input_focused = true; self.ix.on_left_released(); }
            Some(Z_MODAL_OPEN) => { self.show_modal = true; self.ix.on_left_released(); }
            _ => {
                if self.input_focused { self.input_focused = false; }
            }
        }
        false
    }

    pub fn on_left_released(&mut self) {
        self.ix.on_left_released();
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if self.context_menu.is_open() {
            self.context_menu.on_scroll(delta * 40.0);
        }
        self.ix.on_scroll(delta);
    }

    pub fn on_cursor_left(&mut self) {
        self.ix.on_cursor_left();
    }

    pub fn on_key_input(&mut self, key_text: Option<&str>, backspace: bool) {
        if !self.input_focused { return; }
        if backspace {
            self.text_input_value.pop();
            return;
        }
        if let Some(txt) = key_text {
            self.text_input_value.push_str(txt);
        }
    }
}

impl Default for RendererLab {
    fn default() -> Self { Self::new() }
}

mod containers;
mod layout;
mod sections;

use lntrn_render::{
    Color, GpuContext, GpuTexture, Painter, Rect, SurfaceError, TextRenderer, TextureDraw,
    TexturePass,
};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, Fill, FontSize, FoxPalette, GradientTopBar, InteractionContext,
    MenuEvent, MenuItem, Panel, TabBar, TextLabel, TitleBar,
};

use containers::*;
use layout::*;
use sections::*;

// ── Tab names ────────────────────────────────────────────────────────────────

const TAB_LABELS: &[&str] = &["Typography", "Controls", "Inputs", "Containers"];

// ── Zone IDs ─────────────────────────────────────────────────────────────────
// 1-9:   title bar
// 10-19: tab bar
// 20-49: controls tab
// 50-79: inputs tab
// 80-99: containers tab
// 100+:  reserved

const ZONE_CLOSE: u32 = 1;
const ZONE_MINIMIZE: u32 = 2;
const ZONE_MAXIMIZE: u32 = 3;
const ZONE_TAB_BASE: u32 = 10; // 10, 11, 12, 13 for four tabs
const ZONE_NESTED_TAB_BASE: u32 = 60; // 60, 61, 62 for nested tabs

pub enum LabCommand {
    ResetSlider,
}

pub struct RendererLab {
    ix: InteractionContext,
    selected_tab: usize,
    hovered_tab: Option<usize>,

    // Controls tab state
    slider_value: f32,
    checkbox_states: [bool; 3],

    // Inputs tab state
    text_input_value: String,
    focused_input: Option<u32>,
    nested_tab: usize,
    nested_tab_hovered: Option<usize>,

    // Containers tab state
    scroll_offset: f32,
    tex_pass: Option<TexturePass>,
    test_texture: Option<GpuTexture>,

    // Global
    context_menu: ContextMenu,
}

impl RendererLab {
    pub fn new() -> Self {
        let style = ContextMenuStyle::from_palette(&FoxPalette::dark());
        Self {
            ix: InteractionContext::new(),
            selected_tab: 0,
            hovered_tab: None,
            slider_value: 0.62,
            checkbox_states: [true, false, false],
            text_input_value: String::from("Hello Lantern"),
            focused_input: None,
            nested_tab: 0,
            nested_tab_hovered: None,
            scroll_offset: 0.0,
            tex_pass: None,
            test_texture: None,
            context_menu: ContextMenu::new(style),
        }
    }

    pub fn render(
        &mut self,
        size: (u32, u32),
        gpu: &GpuContext,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
    ) -> Result<(), SurfaceError> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }

        self.ix.begin_frame();

        let width = size.0 as f32;
        let height = size.1 as f32;
        let fox = FoxPalette::dark();
        let border = fox.muted.with_alpha(0.3);
        let panel = panel_rect(size);

        painter.clear();

        // Full-window background
        painter.rect_filled(Rect::new(0.0, 0.0, width, height), 0.0, fox.bg);

        // Multicolor accent strip at the very top of the panel
        let strip_y = panel.y + TITLE_BAR_H;
        GradientTopBar::new(panel.x, strip_y, panel.w).draw(painter);

        // Main panel background
        Panel::new(panel)
            .fill(Fill::vertical(fox.surface, fox.bg))
            .radius(18.0)
            .draw(painter);
        painter.rect_stroke(panel, 18.0, 1.0, border);

        // ── Title bar ────────────────────────────────────────────────────
        let tb_rect = title_bar_rect(panel);
        let tb = TitleBar::new(tb_rect, "");
        let min_state = self.ix.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());
        let max_state = self.ix.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
        let close_state = self.ix.add_zone(ZONE_CLOSE, tb.close_button_rect());

        TitleBar::new(tb_rect, "Lantern Lab")
            .minimize_hovered(min_state.is_hovered())
            .maximize_hovered(max_state.is_hovered())
            .close_hovered(close_state.is_hovered())
            .draw(painter, text_renderer, &fox, size.0, size.1);

        // Re-draw gradient strip on top of panel (after panel background)
        GradientTopBar::new(panel.x, strip_y, panel.w).draw(painter);

        // ── Tab bar ──────────────────────────────────────────────────────
        let tab_rect = tab_bar_rect(panel);
        self.update_tab_hover(&tab_rect);

        // Register tab zones for hit testing
        let tab_bar_widget = TabBar::new(tab_rect).tabs(TAB_LABELS).selected(self.selected_tab);
        let tab_rects = tab_bar_widget.tab_rects();
        for (i, tr) in tab_rects.iter().enumerate() {
            self.ix.add_zone(ZONE_TAB_BASE + i as u32, *tr);
        }

        TabBar::new(tab_rect)
            .tabs(TAB_LABELS)
            .selected(self.selected_tab)
            .hovered(self.hovered_tab)
            .draw(painter, text_renderer, &fox, size.0, size.1);

        // ── Content area ─────────────────────────────────────────────────
        let content = content_rect(panel);
        let mut tex_clip: Option<Rect> = None;

        match self.selected_tab {
            0 => draw_typography_tab(painter, text_renderer, &fox, content, size),
            1 => draw_controls_tab(
                painter,
                text_renderer,
                &fox,
                &mut self.ix,
                content,
                self.slider_value,
                self.checkbox_states,
                size,
            ),
            2 => {
                // Register nested tab zones
                let nested_area = inputs_nested_tabs_rect(content);
                let nested_bar = Rect::new(
                    nested_area.x + 18.0,
                    nested_area.y + 48.0,
                    nested_area.w - 36.0,
                    38.0,
                );
                let nested_labels: &[&str] = &["Alpha", "Beta", "Gamma"];
                let nested_widget = TabBar::new(nested_bar).tabs(nested_labels);
                let nested_rects = nested_widget.tab_rects();
                for (i, tr) in nested_rects.iter().enumerate() {
                    self.ix.add_zone(ZONE_NESTED_TAB_BASE + i as u32, *tr);
                }

                draw_inputs_tab(
                    painter,
                    text_renderer,
                    &fox,
                    &mut self.ix,
                    content,
                    &self.text_input_value,
                    self.focused_input,
                    self.nested_tab,
                    self.nested_tab_hovered,
                    size,
                );
            }
            3 => {
                // Ensure texture pass is created
                if self.tex_pass.is_none() {
                    let tp = TexturePass::new(gpu);
                    let checker = generate_checkerboard(64, 8);
                    self.test_texture = Some(tp.upload(gpu, &checker, 64, 64));
                    self.tex_pass = Some(tp);
                }
                tex_clip = draw_containers_tab(
                    painter,
                    text_renderer,
                    &fox,
                    &mut self.ix,
                    content,
                    &mut self.scroll_offset,
                    size,
                );
            }
            _ => {}
        }

        // ── Footer ──────────────────────────────────────────────────────
        let footer_text = match self.selected_tab {
            0 => "Font sizes and palette colors from the theme",
            1 => "Click buttons \u{2022} Drag slider \u{2022} Toggle checkboxes",
            2 => "Click inputs to focus \u{2022} Nested tabs switch independently",
            3 => "Scroll list \u{2022} Drag scrollbar \u{2022} Texture clipping demo",
            _ => "",
        };
        TextLabel::new(
            footer_text,
            panel.x + CONTENT_PAD,
            panel.y + panel.h - FOOTER_H + 8.0,
        )
            .size(FontSize::Caption)
            .color(fox.muted)
            .max_width(panel.w - CONTENT_PAD * 2.0)
            .draw(text_renderer, size.0, size.1);

        // ── Context menu (drawn last, on top) ────────────────────────────
        if let Some(clicked) = self.context_menu.draw(
            painter,
            text_renderer,
            &mut self.ix,
            size.0,
            size.1,
        ) {
            match clicked {
                MenuEvent::Action(1) => {
                    self.slider_value = 0.5;
                    self.context_menu.close();
                }
                MenuEvent::Action(2) => {
                    self.checkbox_states = [false, false, false];
                    self.context_menu.close();
                }
                MenuEvent::Action(3) => {
                    self.selected_tab = 0;
                    self.context_menu.close();
                }
                _ => {
                    self.context_menu.close();
                }
            }
        }

        // ── GPU frame: shapes -> textures -> text ────────────────────────
        let mut frame = gpu.begin_frame("Lab")?;
        let view = frame.view().clone();
        painter.render_pass(gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);

        // Render checkerboard texture if on containers tab
        if self.selected_tab == 3 {
            if let (Some(tp), Some(tex), Some(clip)) =
                (&self.tex_pass, &self.test_texture, tex_clip)
            {
                tp.render_pass(
                    gpu,
                    frame.encoder_mut(),
                    &view,
                    &[TextureDraw::new(tex, clip.x + 10.0, clip.y + 10.0, 120.0, 120.0)],
                );
            }
        }

        text_renderer.render_queued(gpu, frame.encoder_mut(), &view);
        frame.submit(&gpu.queue);
        Ok(())
    }

    // ── Event handlers ───────────────────────────────────────────────────

    pub fn on_cursor_moved(&mut self, size: (u32, u32), x: f32, y: f32) {
        self.ix.on_cursor_moved(x, y);

        // Slider dragging (controls tab)
        if self.ix.active_zone_id() == Some(ZONE_SLIDER) {
            let content = content_rect(panel_rect(size));
            self.slider_value = slider_value_for_x(content, x);
        }

        // Scrollbar dragging handled in render via InteractionContext

        // Update tab hover
        let panel = panel_rect(size);
        let tab_rect = tab_bar_rect(panel);
        self.update_tab_hover(&tab_rect);

        // Update nested tab hover
        if self.selected_tab == 2 {
            let content = content_rect(panel);
            self.update_nested_tab_hover(content);
        }
    }

    pub fn on_right_pressed(&mut self, size: (u32, u32)) {
        if let Some((x, y)) = self.ix.cursor() {
            self.context_menu.open(
                x,
                y,
                vec![
                    MenuItem::action(1, "Reset Slider"),
                    MenuItem::action(2, "Reset Checkboxes"),
                    MenuItem::separator(),
                    MenuItem::action(3, "Go to Typography"),
                    MenuItem::separator(),
                    MenuItem::action(99, "Close Menu"),
                ],
            );
            self.context_menu
                .clamp_to_screen(size.0 as f32, size.1 as f32);
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

        let hit = self.ix.on_left_pressed();

        // Context menu item clicks handled in render()
        if self.context_menu.is_open() {
            return false;
        }

        // Close button
        if hit == Some(ZONE_CLOSE) {
            return true;
        }

        // Tab switching
        if let Some(id) = hit {
            if id >= ZONE_TAB_BASE && id < ZONE_TAB_BASE + TAB_LABELS.len() as u32 {
                self.selected_tab = (id - ZONE_TAB_BASE) as usize;
                self.ix.on_left_released(); // Don't capture tabs
                return false;
            }
        }

        // Nested tab switching (inputs tab)
        if let Some(id) = hit {
            if id >= ZONE_NESTED_TAB_BASE && id < ZONE_NESTED_TAB_BASE + 3 {
                self.nested_tab = (id - ZONE_NESTED_TAB_BASE) as usize;
                self.ix.on_left_released();
                return false;
            }
        }

        // Slider click (controls tab)
        if hit == Some(ZONE_SLIDER) {
            if let Some((x, _)) = self.ix.cursor() {
                let content = content_rect(panel_rect(size));
                self.slider_value = slider_value_for_x(content, x);
            }
            return false;
        }

        // Checkbox clicks (controls tab)
        if hit == Some(ZONE_CB_ONE) {
            self.checkbox_states[0] = !self.checkbox_states[0];
            self.ix.on_left_released();
            return false;
        }
        if hit == Some(ZONE_CB_TWO) {
            self.checkbox_states[1] = !self.checkbox_states[1];
            self.ix.on_left_released();
            return false;
        }
        // CB_THREE is disabled, don't toggle

        // Text input focus (inputs tab)
        if hit == Some(ZONE_INPUT_EMPTY) || hit == Some(ZONE_INPUT_FILLED) {
            self.focused_input = hit;
            self.ix.on_left_released();
            return false;
        }
        // Click on focused input (always focused) -- no-op
        if hit == Some(ZONE_INPUT_FOCUSED) {
            self.ix.on_left_released();
            return false;
        }

        // Click outside inputs clears focus
        if self.selected_tab == 2 && self.focused_input.is_some() {
            if hit != Some(ZONE_INPUT_EMPTY)
                && hit != Some(ZONE_INPUT_FILLED)
                && hit != Some(ZONE_INPUT_FOCUSED)
            {
                self.focused_input = None;
            }
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
        self.hovered_tab = None;
        self.nested_tab_hovered = None;
    }

    pub fn on_command(&mut self, command: LabCommand) {
        match command {
            LabCommand::ResetSlider => {
                self.slider_value = 0.5;
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn update_tab_hover(&mut self, tab_rect: &Rect) {
        let Some((x, y)) = self.ix.cursor() else {
            self.hovered_tab = None;
            return;
        };

        let widget = TabBar::new(*tab_rect).tabs(TAB_LABELS).selected(self.selected_tab);
        let rects = widget.tab_rects();
        self.hovered_tab = rects.iter().position(|r| r.contains(x, y));
    }

    fn update_nested_tab_hover(&mut self, content: Rect) {
        let Some((x, y)) = self.ix.cursor() else {
            self.nested_tab_hovered = None;
            return;
        };

        let nested_area = inputs_nested_tabs_rect(content);
        let nested_bar = Rect::new(
            nested_area.x + 18.0,
            nested_area.y + 48.0,
            nested_area.w - 36.0,
            38.0,
        );
        let labels: &[&str] = &["Alpha", "Beta", "Gamma"];
        let widget = TabBar::new(nested_bar).tabs(labels);
        let rects = widget.tab_rects();
        self.nested_tab_hovered = rects.iter().position(|r| r.contains(x, y));
    }
}

impl Default for RendererLab {
    fn default() -> Self {
        Self::new()
    }
}

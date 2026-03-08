mod layout;
mod sections;

use lntrn_render::{
    Color, GpuContext, GpuTexture, Painter, Rect, SurfaceError, TextRenderer, TextureDraw,
    TexturePass,
};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, Fill, FontSize, FoxPalette, GradientTopBar, InteractionContext,
    MenuItem, Panel, ScrollArea, Scrollbar, TextLabel, TitleBar,
};

use layout::{
    generate_checkerboard, panel_rect, scroll_demo_rect, slider_control_rect, slider_panel_rect,
    slider_value_for_x, swatch_rect, swatches_origin_y, tex_demo_rect, text_reference_rect,
    ORB_RADIUS, SCROLL_DEMO_ITEMS, SCROLL_DEMO_ITEM_H,
};
use sections::{draw_scroll_demo, draw_slider_section, draw_text_reference, draw_texture_demo};

pub enum LabCommand {
    ResetOrb,
}

pub struct RendererLab {
    ix: InteractionContext,
    hovered_swatch: Option<usize>,
    selected_swatch: usize,
    drag_grab_offset: (f32, f32),
    orb_offset: (f32, f32),
    slider_value: f32,
    scroll_offset: f32,
    tex_pass: Option<TexturePass>,
    test_texture: Option<GpuTexture>,
    context_menu: ContextMenu,
}

// Zone IDs for the interaction context
const ZONE_CLOSE: u32 = 1;
const ZONE_MINIMIZE: u32 = 2;
const ZONE_MAXIMIZE: u32 = 3;
const ZONE_SLIDER: u32 = 4;
const ZONE_ORB: u32 = 5;
const ZONE_SCROLLBAR: u32 = 6;

impl RendererLab {
    pub fn new() -> Self {
        let style = ContextMenuStyle::from_palette(&FoxPalette::dark());
        Self {
            ix: InteractionContext::new(),
            hovered_swatch: None,
            selected_swatch: 0,
            drag_grab_offset: (0.0, 0.0),
            orb_offset: (0.0, 0.0),
            slider_value: 0.62,
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

        let bg = Color::from_rgb8(24, 24, 24);
        let fox = FoxPalette::dark();
        let border = Color::from_rgb8(62, 62, 62);
        let text = fox.text;
        let muted = fox.muted;
        let palette = self.palette();
        let accent = palette[self.selected_swatch];
        let panel = panel_rect(size);

        painter.clear();

        // Full-window background
        painter.rect_filled(Rect::new(0.0, 0.0, width, height), 0.0, bg);

        // Multicolor accent strip across the top
        GradientTopBar::new(0.0, 0.0, width).draw(painter);

        // Main panel
        Panel::new(panel)
            .fill(Fill::vertical(fox.surface, fox.bg))
            .radius(18.0)
            .draw(painter);
        painter.rect_stroke(panel, 18.0, 1.0, border);

        // Title bar — register zones for hit testing
        let title_bar_rect = Rect::new(panel.x, panel.y, panel.w, 52.0);
        let tb = TitleBar::new(title_bar_rect, "");
        let min_state = self.ix.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());
        let max_state = self.ix.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
        let close_state = self.ix.add_zone(ZONE_CLOSE, tb.close_button_rect());

        TitleBar::new(title_bar_rect, "Lantern Renderer Lab")
            .minimize_hovered(min_state.is_hovered())
            .maximize_hovered(max_state.is_hovered())
            .close_hovered(close_state.is_hovered())
            .draw(painter, text_renderer, &fox, size.0, size.1);

        // Subtitle
        TextLabel::new(
            "Gradients \u{2022} Widgets \u{2022} Interactions",
            panel.x + 28.0,
            panel.y + 58.0,
        )
            .size(FontSize::Small)
            .color(muted)
            .draw(text_renderer, size.0, size.1);

        // Separator
        painter.line(
            panel.x + 28.0,
            panel.y + 92.0,
            panel.x + panel.w - 28.0,
            panel.y + 92.0,
            1.0,
            border,
        );

        let text_rect = text_reference_rect(panel);
        draw_text_reference(painter, text_renderer, &fox, border, text_rect, size);

        let slider_panel = slider_panel_rect(panel);
        let slider_rect = slider_control_rect(panel);
        let slider_state = self.ix.add_zone(ZONE_SLIDER, slider_rect);
        draw_slider_section(
            painter,
            text_renderer,
            &fox,
            border,
            slider_panel,
            slider_rect,
            self.slider_value,
            slider_state.is_hovered(),
            slider_state.is_active(),
            size,
        );

        // ── Color swatches ───────────────────────────────────────────────
        let swatch_y = swatches_origin_y(panel);
        let swatch_x = panel.x + 28.0;
        for (index, color) in palette.into_iter().enumerate() {
            let rect = swatch_rect(swatch_x, swatch_y, index);
            painter.rect_filled(rect, 10.0, color);

            if self.hovered_swatch == Some(index) {
                painter.rect_stroke(rect.expand(3.0), 13.0, 2.0, Color::WHITE.with_alpha(0.55));
            }
            if self.selected_swatch == index {
                painter.rect_stroke(rect.expand(6.0), 16.0, 2.0, accent.with_alpha(0.95));
            }
        }

        // Orb
        let orb_rect = self.orb_rect(size);
        let orb_state = self.ix.add_zone(ZONE_ORB, orb_rect);
        let orb_center = (orb_rect.center_x(), orb_rect.center_y());
        painter.circle_filled(
            orb_center.0,
            orb_center.1,
            ORB_RADIUS,
            accent.with_alpha(if orb_state.is_active() { 0.95 } else { 0.82 }),
        );
        painter.circle_stroke(
            orb_center.0,
            orb_center.1,
            ORB_RADIUS + 14.0,
            3.0,
            accent.with_alpha(0.35),
        );

        // ── Texture + Clipping demo ────────────────────────────────────────
        if self.tex_pass.is_none() {
            let tp = TexturePass::new(gpu);
            let checker = generate_checkerboard(64, 8);
            self.test_texture = Some(tp.upload(gpu, &checker, 64, 64));
            self.tex_pass = Some(tp);
        }

        let tex_panel = tex_demo_rect(panel);
        let clip = draw_texture_demo(painter, text_renderer, border, text, tex_panel, size);

        let scroll_panel = scroll_demo_rect(panel);

        let scroll_viewport = Rect::new(
            scroll_panel.x + 8.0,
            scroll_panel.y + 40.0,
            scroll_panel.w - 16.0,
            scroll_panel.h - 48.0,
        );
        let content_height = SCROLL_DEMO_ITEMS as f32 * SCROLL_DEMO_ITEM_H;

        // Apply wheel scroll if cursor is inside viewport
        if self.ix.is_hovered(&scroll_viewport) {
            let delta = self.ix.scroll_delta() * 40.0;
            ScrollArea::apply_scroll(&mut self.scroll_offset, delta, content_height, scroll_viewport.h);
        }

        let area = ScrollArea::new(scroll_viewport, content_height, &mut self.scroll_offset);
        let scrollbar = Scrollbar::new(&scroll_viewport, content_height, self.scroll_offset);
        let sb_state = self.ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);

        // Handle scrollbar thumb drag
        if sb_state.is_active() {
            if let Some((_, y)) = self.ix.cursor() {
                self.scroll_offset = scrollbar.offset_for_thumb_y(y, content_height, scroll_viewport.h);
            }
        }

        draw_scroll_demo(
            painter,
            text_renderer,
            &fox,
            border,
            scroll_panel,
            &area,
            &scrollbar,
            sb_state,
            size,
        );

        // ── Footer ────────────────────────────────────────────────────────
        TextLabel::new(
            "Click swatches \u{2022} Drag slider \u{2022} Drag orb \u{2022} Scroll list \u{2022} R: reset",
            panel.x + 28.0,
            panel.y + panel.h - 42.0,
        )
            .size(FontSize::Caption)
            .color(muted)
            .max_width(panel.w - 56.0)
            .draw(text_renderer, size.0, size.1);

        // Context menu (drawn last so it's on top of everything)
        if let Some(clicked_id) = self.context_menu.draw(
            painter,
            text_renderer,
            &mut self.ix,
            size.0,
            size.1,
        ) {
            match clicked_id {
                1 => self.orb_offset = (0.0, 0.0),
                2 => self.slider_value = 0.5,
                3 => self.selected_swatch = 0,
                _ => {}
            }
            self.context_menu.close();
        }

        // Manual frame management: shapes → textures → text
        let mut frame = gpu.begin_frame("Lab")?;
        let view = frame.view().clone();
        painter.render_pass(gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);

        if let (Some(tp), Some(tex)) = (&self.tex_pass, &self.test_texture) {
            tp.render_pass(
                gpu,
                frame.encoder_mut(),
                &view,
                &[TextureDraw::new(tex, clip.x + 10.0, clip.y + 10.0, 120.0, 120.0)],
            );
        }

        text_renderer.render_queued(gpu, frame.encoder_mut(), &view);
        frame.submit(&gpu.queue);
        Ok(())
    }

    pub fn on_cursor_moved(&mut self, size: (u32, u32), x: f32, y: f32) {
        self.ix.on_cursor_moved(x, y);

        // Orb dragging
        if self.ix.active_zone_id() == Some(ZONE_ORB) {
            let panel = panel_rect(size);
            let base_x = panel.x + panel.w - 110.0;
            let base_y = swatches_origin_y(panel) + 104.0;
            self.orb_offset = (x - self.drag_grab_offset.0 - base_x, y - self.drag_grab_offset.1 - base_y);
        }

        // Slider dragging
        if self.ix.active_zone_id() == Some(ZONE_SLIDER) {
            self.slider_value = slider_value_for_x(size, x);
        }

        self.update_swatch_hover(size);
    }

    pub fn on_right_pressed(&mut self, size: (u32, u32)) {
        if let Some((x, y)) = self.ix.cursor() {
            self.context_menu.open(x, y, vec![
                MenuItem::action(1, "Reset Orb"),
                MenuItem::action(2, "Reset Slider"),
                MenuItem::separator(),
                MenuItem::action(3, "Default Swatch"),
                MenuItem::separator(),
                MenuItem::action(99, "Close Menu"),
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

        let hit = self.ix.on_left_pressed();

        // Handle context menu item clicks
        if self.context_menu.is_open() {
            // The draw() call already handles returning clicked IDs,
            // but we need the press event to register in InteractionContext first.
            // The actual click handling happens in render() via draw().
            return false;
        }

        if hit == Some(ZONE_CLOSE) {
            return true;
        }

        if hit == Some(ZONE_SLIDER) {
            if let Some((x, _)) = self.ix.cursor() {
                self.slider_value = slider_value_for_x(size, x);
            }
            return false;
        }

        if hit == Some(ZONE_ORB) {
            if let Some((x, y)) = self.ix.cursor() {
                let orb = self.orb_rect(size);
                self.drag_grab_offset = (x - orb.center_x(), y - orb.center_y());
            }
            return false;
        }

        // Check swatch click (not in zone system — simple index-based)
        if let Some(index) = self.hovered_swatch {
            self.selected_swatch = index;
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
        self.hovered_swatch = None;
    }

    pub fn on_command(&mut self, command: LabCommand) {
        match command {
            LabCommand::ResetOrb => {
                self.orb_offset = (0.0, 0.0);
                self.ix.on_left_released();
            }
        }
    }

    fn update_swatch_hover(&mut self, size: (u32, u32)) {
        let Some((x, y)) = self.ix.cursor() else {
            self.hovered_swatch = None;
            return;
        };

        let panel = panel_rect(size);
        let swatch_y = swatches_origin_y(panel);
        let swatch_x = panel.x + 28.0;
        self.hovered_swatch = (0..4).find(|index| swatch_rect(swatch_x, swatch_y, *index).contains(x, y));
    }

    fn orb_rect(&self, size: (u32, u32)) -> Rect {
        let panel = panel_rect(size);
        let base_x = panel.x + panel.w - 110.0;
        let base_y = swatches_origin_y(panel) + 104.0;
        let min_x = panel.x + 72.0;
        let max_x = panel.x + panel.w - 72.0;
        let min_y = swatches_origin_y(panel) + 72.0;
        let max_y = panel.y + panel.h - 120.0;

        let cx = (base_x + self.orb_offset.0).clamp(min_x, max_x);
        let cy = (base_y + self.orb_offset.1).clamp(min_y, max_y);
        Rect::new(cx - ORB_RADIUS, cy - ORB_RADIUS, ORB_RADIUS * 2.0, ORB_RADIUS * 2.0)
    }

    fn palette(&self) -> [Color; 4] {
        [
            Color::from_rgb8(200, 134, 10),
            Color::from_rgb8(88, 166, 255),
            Color::from_rgb8(46, 160, 67),
            Color::from_rgb8(218, 54, 51),
        ]
    }
}

impl Default for RendererLab {
    fn default() -> Self {
        Self::new()
    }
}
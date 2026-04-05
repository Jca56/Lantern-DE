use lntrn_render::{Color, Frame, GpuContext, Painter};
use lntrn_ui::gpu::MenuEvent;

use crate::render;
use crate::sidebar;
use crate::tab_bar;
use crate::ui_chrome;

use crate::app::{App, SplitDir, Tab, SPLIT_DIVIDER};

impl App {
    pub(crate) fn render_frame(&mut self) {
        if self.tabs.is_empty() {
            return;
        }

        // Sync smooth scroll state to terminal before rendering
        let sub_pixel_y = self.sync_scroll_to_terminal();
        let sb_offset = self.sidebar_offset();

        let font_size = self.effective_font_size();
        let chrome_h = self.chrome_height();
        let tab_bar_visible = self.tab_bar_visible;
        let gpu = match self.gpu.as_ref() {
            Some(g) => g,
            None => return,
        };
        let painter = match self.painter.as_mut() {
            Some(p) => p,
            None => return,
        };
        let text = match self.text.as_mut() {
            Some(t) => t,
            None => return,
        };

        let screen_w = gpu.width();
        let screen_h = gpu.height();

        let opacity = self.config.window.opacity;
        let bg_alpha = (opacity * 255.0).round() as u8;
        let bg = Color::from_rgba8(self.theme.bg.r, self.theme.bg.g, self.theme.bg.b, bg_alpha);

        painter.clear();
        self.input.begin_frame();

        // Draw window background — square corners when maximized
        let title_bar_color = Color::from_rgba8(51, 51, 51, 255);
        let maximized = self
            .window
            .as_ref()
            .map_or(false, |w| w.is_maximized() || w.fullscreen().is_some());
        render::draw_window_bg(
            painter,
            title_bar_color,
            bg,
            screen_w as f32,
            screen_h as f32,
            maximized,
            tab_bar_visible,
        );

        // Draw sidebar file browser
        sidebar::draw_sidebar(
            painter,
            text,
            &self.sidebar,
            chrome_h,
            screen_w,
            screen_h,
            self.cursor_pos,
        );

        // Render all panes in the active tab
        let tab_ref = &self.tabs[self.active_tab];
        let rects = Self::pane_rects_for_tab(tab_ref, screen_w, screen_h, sb_offset, chrome_h);
        let tab = &self.tabs[self.active_tab];
        let cell_h = render::measure_cell(font_size).1;
        for (i, pane) in tab.panes.iter().enumerate() {
            if i >= rects.len() {
                break;
            }
            let (gx, gy, gw, gh) =
                Self::pane_grid_bounds(pane, rects[i], font_size);
            let is_focused = i == tab.active_pane;
            let is_active_pane = i == tab.active_pane;

            // For the active pane, apply sub-pixel scroll offset with clipping
            let pane_sub_pixel = if is_active_pane { sub_pixel_y } else { 0.0 };

            // Clip to the pane rect so shifted content doesn't bleed
            let clip = lntrn_render::Rect::new(gx, gy, gw, gh);
            painter.push_clip(clip);

            let extra = if pane_sub_pixel > 0.0 { 1 } else { 0 };
            render::draw_terminal_ex(
                painter,
                text,
                &pane.terminal,
                font_size,
                (gx, gy - pane_sub_pixel),
                screen_w,
                screen_h,
                self.cursor_visible && is_focused,
                bg,
                extra,
            );

            painter.pop_clip();

            // Draw scrollbar for active pane when scrolled
            if is_active_pane {
                let total_lines = pane.terminal.scrollback.len() + pane.terminal.rows;
                let content_height = total_lines as f32 * cell_h;
                let viewport = lntrn_render::Rect::new(gx, gy, gw, gh);
                let max_scroll = (content_height - gh).max(0.0);
                let inverted_offset = max_scroll - self.scroll_current_px.min(max_scroll);
                let scrollbar = lntrn_ui::gpu::scroll::Scrollbar::new(
                    &viewport,
                    content_height,
                    inverted_offset,
                );
                let sb_state = if self.scrollbar_dragging {
                    lntrn_ui::gpu::input::InteractionState::Pressed
                } else if self
                    .cursor_pos
                    .map_or(false, |(cx, cy)| scrollbar.hover_zone().contains(cx, cy))
                {
                    lntrn_ui::gpu::input::InteractionState::Hovered
                } else {
                    lntrn_ui::gpu::input::InteractionState::Idle
                };
                let palette = lntrn_ui::gpu::palette::FoxPalette::dark();
                scrollbar.draw(painter, sb_state, &palette);
            }
        }

        // Draw dividers between panes
        if tab.panes.len() > 1 {
            draw_pane_dividers(painter, &rects, tab);
        }

        // Build tab display info
        let tab_displays: Vec<tab_bar::TabDisplay> = self
            .tabs
            .iter()
            .map(|t| {
                let title = t.custom_name.as_deref().unwrap_or_else(|| {
                    t.panes
                        .get(t.active_pane)
                        .map_or("Shell", |p| p.title.as_str())
                });
                tab_bar::TabDisplay {
                    title,
                    pinned: t.pinned,
                }
            })
            .collect();

        // Draw title bar (menus + window controls + gradient strip)
        ui_chrome::draw_chrome(
            painter,
            text,
            &mut self.chrome,
            &mut self.input,
            screen_w,
            screen_h,
            font_size,
            self.config.window.opacity,
            self.sidebar.visible,
            maximized,
            1.0,
        );

        // Draw tab bar (auto-hides, appears on hover)
        if tab_bar_visible {
            tab_bar::draw_tab_bar(
                painter,
                text,
                &self.tab_bar,
                &tab_displays,
                self.active_tab,
                screen_w,
                screen_h,
                self.cursor_pos,
            );
        }

        let has_overlay = self.chrome.has_overlay() || self.tab_bar.has_overlay() || self.sidebar.has_overlay();

        if has_overlay {
            // Two-pass rendering: menus must appear ABOVE terminal text.
            // Uses a SEPARATE overlay_painter to avoid GPU buffer conflicts.
            let overlay_painter = match self.overlay_painter.as_mut() {
                Some(p) => p,
                None => {
                    if let Err(e) = painter.render_with_text(gpu, text, bg) {
                        Self::handle_render_error(e, &mut self.gpu);
                    }
                    return;
                }
            };
            overlay_painter.clear();

            let overlay_text = match self.overlay_text.as_mut() {
                Some(t) => t,
                None => {
                    if let Err(e) = painter.render_with_text(gpu, text, bg) {
                        Self::handle_render_error(e, &mut self.gpu);
                    }
                    return;
                }
            };

            // Queue overlay geometry + text into separate painter/text
            let menu_event = ui_chrome::draw_overlay(
                overlay_painter,
                overlay_text,
                &mut self.chrome,
                &mut self.input,
                screen_w,
                screen_h,
            );

            // Process menu events from overlay
            if let Some(ref event) = menu_event {
                self.pending_menu_event = Some(ui_chrome::menu_event_to_action(event));
                if let MenuEvent::SliderChanged { id, value } = event {
                    match *id {
                        ui_chrome::MENU_FONT_SLIDER => {
                            self.config.font.size = ui_chrome::font_size_from_slider(*value);
                        }
                        ui_chrome::MENU_OPACITY_SLIDER => {
                            self.config.window.opacity = ui_chrome::opacity_from_slider(*value);
                        }
                        _ => {}
                    }
                }
            }

            // Tab context menu overlay
            tab_bar::draw_tab_context_menu(
                overlay_painter,
                overlay_text,
                &self.tab_bar,
                &tab_displays,
                screen_w,
                screen_h,
                self.cursor_pos,
            );

            // Sidebar context menu overlay
            sidebar::draw_sidebar_context_menu(
                overlay_painter,
                overlay_text,
                &self.sidebar,
                screen_w,
                screen_h,
                self.cursor_pos,
            );

            let result: Result<(), lntrn_render::SurfaceError> = (|| {
                let mut frame: Frame = gpu.begin_frame("Lantern 2D+Text+Overlay")?;
                let view = frame.view().clone();

                // Pass 1: base shapes + base text
                painter.render_pass(gpu, frame.encoder_mut(), &view, bg);
                text.render_queued(gpu, frame.encoder_mut(), &view);

                // Pass 2: overlay shapes + overlay text
                overlay_painter.render_pass_overlay(gpu, frame.encoder_mut(), &view);
                overlay_text.render_queued(gpu, frame.encoder_mut(), &view);

                frame.submit(&gpu.queue);
                Ok(())
            })();
            if let Err(e) = result {
                Self::handle_render_error(e, &mut self.gpu);
            }
        } else {
            // Single-pass: no menus open
            if let Err(e) = painter.render_with_text(gpu, text, bg) {
                Self::handle_render_error(e, &mut self.gpu);
            }
        }

        // If the base font size changed (e.g. via slider), the effective size
        // may now differ from what update_grid_size last used — resync.
        self.update_grid_size();
    }

    pub(crate) fn handle_render_error(
        e: lntrn_render::SurfaceError,
        gpu: &mut Option<GpuContext>,
    ) {
        match e {
            lntrn_render::SurfaceError::Lost | lntrn_render::SurfaceError::Outdated => {
                if let Some(ref mut g) = gpu {
                    g.resize(g.width(), g.height());
                }
            }
            lntrn_render::SurfaceError::Timeout => {}
            _ => eprintln!("[lntrn-terminal] render error: {e:?}"),
        }
    }
}

fn draw_pane_dividers(painter: &mut Painter, rects: &[(f32, f32, f32, f32)], tab: &Tab) {
    let divider_color = Color::from_rgba8(80, 80, 80, 255);
    match tab.split {
        Some(SplitDir::Horizontal) => {
            for i in 1..rects.len() {
                let (x, y, _, h) = rects[i];
                painter.rect_filled(
                    lntrn_render::Rect::new(x - SPLIT_DIVIDER, y, SPLIT_DIVIDER, h),
                    0.0,
                    divider_color,
                );
            }
        }
        Some(SplitDir::Vertical) => {
            for i in 1..rects.len() {
                let (x, y, w, _) = rects[i];
                painter.rect_filled(
                    lntrn_render::Rect::new(x, y - SPLIT_DIVIDER, w, SPLIT_DIVIDER),
                    0.0,
                    divider_color,
                );
            }
        }
        None => {}
    }

    // Highlight active pane border
    let (ax, ay, aw, ah) = rects[tab.active_pane];
    let accent = Color::from_rgba8(200, 134, 10, 80);
    let b = 2.0;
    painter.rect_filled(lntrn_render::Rect::new(ax, ay, aw, b), 0.0, accent);
    painter.rect_filled(
        lntrn_render::Rect::new(ax, ay + ah - b, aw, b),
        0.0,
        accent,
    );
    painter.rect_filled(lntrn_render::Rect::new(ax, ay, b, ah), 0.0, accent);
    painter.rect_filled(
        lntrn_render::Rect::new(ax + aw - b, ay, b, ah),
        0.0,
        accent,
    );
}

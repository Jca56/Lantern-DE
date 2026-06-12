use lntrn_render::{Color, Frame, GpuContext, Painter, TextureDraw};
use lntrn_ui::gpu::{draw_window_gradient_overlay, MenuEvent};

use crate::git_sidebar;
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

        // Upload pending images and collect placements before borrowing painter
        self.upload_pending_images();
        let image_placements = self.collect_image_placements(font_size, sb_offset, chrome_h);

        let mode = crate::config::WindowMode::current();
        let cursor_pos = self.cursor_pos;
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

        // System-wide [windows].background_opacity is the single source of
        // truth — read inline so System Settings changes apply on next paint.
        let opacity = lntrn_theme::background_opacity();
        let bg_alpha = (opacity * 255.0).round() as u8;
        let bg = Color::from_rgba8(self.theme.bg.r, self.theme.bg.g, self.theme.bg.b, bg_alpha);
        // Render-pass clear color: must be fully transparent. `bg` is drawn by
        // `draw_window_bg` further down — clearing to the same colour would
        // double-paint the alpha (0.95 + 0.95 ≈ 0.9975) and make the window
        // effectively opaque. See System Settings + File Manager for the
        // identical pattern.
        let clear = Color::rgba(0.0, 0.0, 0.0, 0.0);

        painter.clear();
        self.input.begin_frame();

        // Draw window background — square corners when maximized
        let title_bar_color = match mode {
            crate::config::WindowMode::Lantern => Color::from_rgba8(50, 40, 30, 255),
            crate::config::WindowMode::Fox => Color::from_rgba8(51, 51, 51, 255),
        };
        let maximized = self
            .window
            .as_ref()
            .map_or(false, |w| w.is_maximized() || w.fullscreen().is_some());
        // Solid bg first so the terminal's own theme.bg sits underneath
        // every glyph cell, then layer the optional System Settings window
        // gradient on top with per-stop alphas (transparent stops reveal
        // the solid bg, not the wallpaper).
        let win_r = if maximized { 0.0 } else { render::CORNER_RADIUS };
        let win_rect = lntrn_render::Rect::new(0.0, 0.0, screen_w as f32, screen_h as f32);
        render::draw_window_bg(
            painter,
            title_bar_color,
            bg,
            screen_w as f32,
            screen_h as f32,
            maximized,
            &mode,
        );
        draw_window_gradient_overlay(painter, win_rect, win_r, opacity);

        // Draw sidebar (file browser or git panel). Works in rice mode too —
        // rice only hides the title/tab bar; the sidebar follows its own
        // visible flag (draw_sidebar no-ops when closed).
        {
            sidebar::draw_sidebar(
                painter,
                text,
                &self.sidebar,
                chrome_h,
                screen_w,
                screen_h,
                self.cursor_pos,
            );
            if self.sidebar.visible && self.sidebar.mode == sidebar::SidebarMode::Git {
                git_sidebar::draw_git_sidebar(
                    painter,
                    text,
                    &self.git_sidebar,
                    self.sidebar.width,
                    chrome_h + sidebar::TOGGLE_H * self.sidebar.scale,
                    screen_w,
                    screen_h,
                    self.cursor_pos,
                );
            }
        }

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
            text.push_clip([gx, gy, gw, gh]);

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
                self.config.general.cursor_style,
            );

            painter.pop_clip();
            text.pop_clip();

            // Draw scrollbar for active pane when scrolled
            if is_active_pane {
                let total_lines = pane.terminal.active_scrollback().len() + pane.terminal.rows;
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
        let tab = &self.tabs[self.active_tab];
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

        // Draw title bar (bg + menus + divider + window controls) and tabs —
        // skipped entirely in rice mode for a perfectly clean window.
        if !self.chrome_hidden {
            let layout = ui_chrome::draw_chrome(
                painter,
                text,
                &mut self.chrome,
                &mut self.input,
                screen_w,
                screen_h,
                font_size,
                self.sidebar.visible,
                self.config.general.cursor_style,
                self.config.general.open_chrome_hidden,
                maximized,
                self.scale,
                &mode,
                cursor_pos,
            );

            // Draw tabs inside the title bar, in the region between the menu
            // divider and the window controls.
            let tabs_bounds = lntrn_render::Rect::new(
                layout.tabs_left,
                0.0,
                (layout.tabs_right - layout.tabs_left).max(0.0),
                layout.bar_h,
            );
            tab_bar::draw_tab_bar(
                painter,
                text,
                &self.tab_bar,
                &tab_displays,
                self.active_tab,
                tabs_bounds,
                screen_w,
                screen_h,
                self.cursor_pos,
                &mode,
            );
        }

        // The right-click context menu and the sidebar stay usable in rice
        // mode (which only hides the title/tab bar); the menu-bar/tab-bar
        // overlays belong to chrome that isn't drawn there.
        let has_overlay = self.chrome.context_menu.is_open()
            || (!self.chrome_hidden
                && (self.chrome.has_overlay() || self.tab_bar.has_overlay()));

        if has_overlay {
            // Two-pass rendering: menus must appear ABOVE terminal text.
            // Uses a SEPARATE overlay_painter to avoid GPU buffer conflicts.
            let overlay_painter = match self.overlay_painter.as_mut() {
                Some(p) => p,
                None => {
                    if let Err(e) = painter.render_with_text(gpu, text, clear) {
                        Self::handle_render_error(e, &mut self.gpu);
                    }
                    return;
                }
            };
            overlay_painter.clear();

            let overlay_text = match self.overlay_text.as_mut() {
                Some(t) => t,
                None => {
                    if let Err(e) = painter.render_with_text(gpu, text, clear) {
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
                // Plain actions (Copy, Paste, splits, ...) dismiss the menu
                // immediately — sliders/toggles/radios stay open for live
                // adjustment, and so do the tab-cycle chevrons so several
                // tabs can be flipped through without reopening the menu.
                let keeps_open = matches!(
                    event,
                    MenuEvent::Action(id)
                        if *id == ui_chrome::CTX_PREV_TAB
                            || *id == ui_chrome::CTX_NEXT_TAB
                            || *id >= ui_chrome::CTX_TAB_DOT_BASE
                );
                if matches!(event, MenuEvent::Action(_)) && !keeps_open {
                    self.chrome.close_all_menus();
                }
                if let MenuEvent::SliderChanged { id, value } = event {
                    match *id {
                        ui_chrome::MENU_FONT_SLIDER => {
                            self.config.font.size = ui_chrome::font_size_from_slider(*value);
                        }
                        _ => {}
                    }
                }
                if let MenuEvent::Toggled { id, checked } = event {
                    if *id == ui_chrome::MENU_OPEN_BAR_HIDDEN {
                        self.config.general.open_chrome_hidden = *checked;
                        self.config.save();
                    }
                }
                if let MenuEvent::RadioSelected { id, group } = event {
                    if *group == ui_chrome::CURSOR_STYLE_GROUP {
                        let new_style = match *id {
                            ui_chrome::MENU_CURSOR_UNDERLINE => {
                                crate::config::CursorStylePref::Underline
                            }
                            ui_chrome::MENU_CURSOR_BEAM => {
                                crate::config::CursorStylePref::Beam
                            }
                            _ => crate::config::CursorStylePref::Block,
                        };
                        if self.config.general.cursor_style != new_style {
                            self.config.general.cursor_style = new_style;
                            self.config.save();
                        }
                    }
                }
                // (Theme radios removed from the menu — now lives in System
                // Settings → Appearance. Terminal reads the active variant at
                // draw time via `WindowMode::current()`.)
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
                &mode,
            );

            let result: Result<(), lntrn_render::SurfaceError> = (|| {
                let mut frame: Frame = gpu.begin_frame("Lantern 2D+Text+Overlay")?;
                let view = frame.view().clone();

                // Pass 1: base shapes + base text
                painter.render_pass(gpu, frame.encoder_mut(), &view, clear);
                text.render_queued(gpu, frame.encoder_mut(), &view);

                // Pass 1.5: inline images
                if !image_placements.is_empty() {
                    if let Some(ref tex_pass) = self.texture_pass {
                        let draws: Vec<TextureDraw> = image_placements.iter().filter_map(|p| {
                            let (_, _, gpu_tex) = self.image_textures.iter().find(|(id, _, _)| *id == p.0)?;
                            Some(TextureDraw::new(gpu_tex, p.1, p.2, p.3, p.4))
                        }).collect();
                        if !draws.is_empty() {
                            tex_pass.render_pass(gpu, frame.encoder_mut(), &view, &draws, None);
                        }
                    }
                }

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
            // Single-pass: no menus open — use manual frame for image support
            let result: Result<(), lntrn_render::SurfaceError> = (|| {
                let mut frame: Frame = gpu.begin_frame("Lantern 2D+Text")?;
                let view = frame.view().clone();

                painter.render_pass(gpu, frame.encoder_mut(), &view, clear);
                text.render_queued(gpu, frame.encoder_mut(), &view);

                // Inline images
                if !image_placements.is_empty() {
                    if let Some(ref tex_pass) = self.texture_pass {
                        let draws: Vec<TextureDraw> = image_placements.iter().filter_map(|p| {
                            let (_, _, gpu_tex) = self.image_textures.iter().find(|(id, _, _)| *id == p.0)?;
                            Some(TextureDraw::new(gpu_tex, p.1, p.2, p.3, p.4))
                        }).collect();
                        if !draws.is_empty() {
                            tex_pass.render_pass(gpu, frame.encoder_mut(), &view, &draws, None);
                        }
                    }
                }

                frame.submit(&gpu.queue);
                Ok(())
            })();
            if let Err(e) = result {
                Self::handle_render_error(e, &mut self.gpu);
            }
        }

        // If the base font size changed (e.g. via slider), the effective size
        // may now differ from what update_grid_size last used — resync.
        self.update_grid_size();
    }

    /// Sync the GPU texture cache with the image_manager state. Each frame:
    ///   1. Removes textures whose image IDs no longer exist (e.g. deleted via
    ///      Kitty `a=d`, or replaced).
    ///   2. Re-uploads textures whose version has bumped (image data changed
    ///      under the same ID — happens when a TUI re-transmits at the same
    ///      ID with different content, like animated map tiles).
    ///   3. Uploads new textures for newly-seen image IDs.
    fn upload_pending_images(&mut self) {
        // Collect current (image_id, version, rgba, width, height) from all panes.
        let mut current: Vec<(u32, u64, Vec<u8>, u32, u32)> = Vec::new();
        for tab in &self.tabs {
            for pane in &tab.panes {
                for img in &pane.terminal.image_manager.images {
                    current.push((
                        img.image_id,
                        img.version,
                        img.rgba.clone(),
                        img.width,
                        img.height,
                    ));
                }
            }
        }

        // 1. Remove textures whose image IDs are no longer in any image_manager
        let live_ids: Vec<u32> = current.iter().map(|(id, _, _, _, _)| *id).collect();
        self.image_textures
            .retain(|(id, _, _)| live_ids.contains(id));

        if current.is_empty() {
            return;
        }

        let Some(ref tex_pass) = self.texture_pass else {
            return;
        };
        let Some(ref gpu) = self.gpu else {
            return;
        };

        // 2 & 3. For each current image, upload if missing or if version changed
        for (id, version, rgba, w, h) in current {
            let existing_idx = self
                .image_textures
                .iter()
                .position(|(eid, _, _)| *eid == id);
            match existing_idx {
                Some(idx) => {
                    let cached_version = self.image_textures[idx].1;
                    if cached_version != version {
                        // Version bumped — re-upload
                        let gpu_tex = tex_pass.upload(gpu, &rgba, w, h);
                        self.image_textures[idx] = (id, version, gpu_tex);
                    }
                }
                None => {
                    let gpu_tex = tex_pass.upload(gpu, &rgba, w, h);
                    self.image_textures.push((id, version, gpu_tex));
                }
            }
        }
    }

    /// Collect image placement info as owned tuples: (image_id, x, y, w, h).
    /// No borrows are held, so this can be called before mutable render closures.
    fn collect_image_placements(
        &self,
        font_size: f32,
        sb_offset: f32,
        chrome_h: f32,
    ) -> Vec<(u32, f32, f32, f32, f32)> {
        if self.tabs.is_empty() {
            return Vec::new();
        }
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        let (cell_w, cell_h) = render::measure_cell(font_size);
        let tab = &self.tabs[self.active_tab];
        let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, sb_offset, chrome_h);

        let mut placements = Vec::new();
        for (i, pane) in tab.panes.iter().enumerate() {
            if i >= rects.len() {
                break;
            }
            let (gx, gy, gw, gh) = Self::pane_grid_bounds(pane, rects[i], font_size);

            for img in &pane.terminal.image_manager.images {
                let x = gx + img.col as f32 * cell_w;
                let y = gy + img.row as f32 * cell_h;
                let w = img.cols_wide as f32 * cell_w;
                let h = img.rows_tall as f32 * cell_h;

                // Skip if entirely outside viewport
                if x + w < gx || x > gx + gw || y + h < gy || y > gy + gh {
                    continue;
                }

                placements.push((img.image_id, x, y, w, h));
            }
        }
        placements
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

/// Live accent from `[appearance].accent` in lantern.toml, theme fallback.
fn accent_color(alpha: u8) -> Color {
    if let Some(c) = lntrn_theme::active_accent() {
        return Color::from_rgba8(c.r, c.g, c.b, alpha);
    }
    let v = lntrn_theme::active_variant().accent();
    Color::from_rgba8(v.r, v.g, v.b, alpha)
}

fn draw_pane_dividers(painter: &mut Painter, rects: &[(f32, f32, f32, f32)], tab: &Tab) {
    let divider_color = accent_color(255);
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
    let accent = accent_color(80);
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

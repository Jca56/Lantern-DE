use std::time::Instant;

use winit::event::{ElementState, MouseScrollDelta};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;

use crate::git_sidebar;
use crate::input;
use crate::render;
use crate::sidebar;
use crate::tab_bar;
use crate::ui_chrome;

use crate::app::{App, SplitDir, CURSOR_BLINK_INTERVAL};

/// Signal from event handlers back to the ApplicationHandler dispatcher.
pub(crate) enum EventResult {
    Continue,
    Exit,
    Handled,
}

impl App {
    pub(crate) fn handle_cursor_moved(&mut self, x: f32, y: f32) -> EventResult {
        self.cursor_pos = Some((x, y));
        self.input.on_cursor_moved(x, y);


        // Tab drag reorder
        if self.tab_bar.dragging.is_some() {
            let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
            let drag_displays: Vec<tab_bar::TabDisplay> = self
                .tabs
                .iter()
                .map(|t| {
                    let title = t.custom_name.as_deref().unwrap_or_else(|| {
                        t.panes.get(t.active_pane).map_or("Shell", |p| p.title.as_str())
                    });
                    tab_bar::TabDisplay { title, pinned: t.pinned }
                })
                .collect();
            let menus = ui_chrome::build_menus(
                self.effective_font_size(),
                self.config.window.opacity,
                self.sidebar.visible,
                &self.config.window.mode,
            );
            let bounds = ui_chrome::tabs_bounds(
                &menus,
                screen_w as f32,
                1.0,
                &self.config.window.mode,
            );
            if let Some(action) =
                tab_bar::handle_drag_move(&mut self.tab_bar, x, &drag_displays, bounds)
            {
                if let tab_bar::TabBarAction::Reorder { from, to } = action {
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                    if self.active_tab == from {
                        self.active_tab = to;
                    } else if from < self.active_tab && to >= self.active_tab {
                        self.active_tab -= 1;
                    } else if from > self.active_tab && to <= self.active_tab {
                        self.active_tab += 1;
                    }
                }
            }
        }

        // Extend selection while dragging
        if self.selecting && !self.tabs.is_empty() {
            if let Some((_pane_idx, row, col)) = self.pixel_to_pane_cell(x, y) {
                let tab = &mut self.tabs[self.active_tab];
                tab.panes[tab.active_pane].terminal.set_selection_end(row, col);
            }
        }

        // Update cursor icon
        let maximized = self
            .window
            .as_ref()
            .map_or(false, |w| w.is_maximized() || w.fullscreen().is_some());
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        if !maximized {
            if let Some(dir) = self.resize_direction(screen_w, screen_h) {
                if let Some(ref window) = self.window {
                    window.set_cursor(CursorIcon::from(dir));
                }
                self.request_redraw();
                return EventResult::Handled;
            }
        }

        // Check if hovering over a hyperlink
        let hovering_link = if !self.tabs.is_empty() {
            if let Some((_pane_idx, row, col)) = self.pixel_to_pane_cell(x, y) {
                let tab = &self.tabs[self.active_tab];
                tab.panes[tab.active_pane].terminal.hyperlink_at(row, col).is_some()
            } else {
                false
            }
        } else {
            false
        };

        if self.selecting {
            if let Some(ref window) = self.window {
                window.set_cursor(CursorIcon::Text);
            }
        } else if hovering_link {
            if let Some(ref window) = self.window {
                window.set_cursor(CursorIcon::Pointer);
            }
        } else if let Some(ref window) = self.window {
            window.set_cursor(CursorIcon::Default);
        }

        self.request_redraw();
        EventResult::Continue
    }

    pub(crate) fn handle_left_press(&mut self, event_loop: &ActiveEventLoop) -> EventResult {
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        self.left_pressed = true;

        // Check resize edges first (not when maximized)
        let maximized = self
            .window
            .as_ref()
            .map_or(false, |w| w.is_maximized() || w.fullscreen().is_some());
        if !maximized {
            if let Some(dir) = self.resize_direction(screen_w, screen_h) {
                if let Some(ref window) = self.window {
                    window.drag_resize_window(dir).ok();
                }
                return EventResult::Handled;
            }
        }

        // When a menu overlay is open, check chrome first so
        // dropdown clicks don't fall through to tabs underneath.
        if !self.chrome.has_overlay() {
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
            let menus = ui_chrome::build_menus(
                self.effective_font_size(),
                self.config.window.opacity,
                self.sidebar.visible,
                &self.config.window.mode,
            );
            let tabs_rect = ui_chrome::tabs_bounds(
                &menus,
                screen_w as f32,
                1.0,
                &self.config.window.mode,
            );
            let tab_action = tab_bar::handle_click(
                &mut self.tab_bar,
                self.cursor_pos,
                self.tabs.len(),
                &tab_displays,
                tabs_rect,
                screen_w,
            );
            if self.handle_tab_bar_action(tab_action, event_loop) {
                self.request_redraw();
                return EventResult::Handled;
            }
        }

        self.input.on_left_pressed();
        let menus = ui_chrome::build_menus(
            self.effective_font_size(),
            self.config.window.opacity,
            self.sidebar.visible,
            &self.config.window.mode,
        );
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width()) as f32;
        let action = ui_chrome::handle_click(
            &mut self.chrome,
            &mut self.input,
            &menus,
            1.0,
            &self.config.window.mode,
            screen_w,
        );

        match self.dispatch_chrome_action(action, event_loop, screen_h) {
            EventResult::Exit => return EventResult::Exit,
            EventResult::Handled => {
                self.request_redraw();
                return EventResult::Handled;
            }
            EventResult::Continue => {}
        }

        self.request_redraw();
        EventResult::Continue
    }

    pub(crate) fn dispatch_chrome_action(
        &mut self,
        action: ui_chrome::ClickAction,
        event_loop: &ActiveEventLoop,
        screen_h: u32,
    ) -> EventResult {
        match action {
            ui_chrome::ClickAction::Close => {
                for tab in &mut self.tabs {
                    for pane in &mut tab.panes {
                        pane.pty.cleanup();
                    }
                }
                event_loop.exit();
                return EventResult::Exit;
            }
            ui_chrome::ClickAction::Minimize => {
                if let Some(ref window) = self.window {
                    window.set_minimized(true);
                }
            }
            ui_chrome::ClickAction::Maximize => {
                if let Some(ref window) = self.window {
                    let is_max = window.is_maximized();
                    window.set_maximized(!is_max);
                }
            }
            ui_chrome::ClickAction::StartDrag => {
                if let Some(ref window) = self.window {
                    window.drag_window().ok();
                }
            }
            ui_chrome::ClickAction::SliderDrag => {
                self.config.save();
                self.update_grid_size();
            }
            ui_chrome::ClickAction::OpacitySliderDrag => {
                self.config.save();
            }
            ui_chrome::ClickAction::WindowModeChanged => {
                // Update theme to match new mode
                use crate::config::WindowMode;
                use crate::theme::Theme;
                use crate::terminal::Color8;
                self.theme = match self.config.window.mode {
                    WindowMode::FoxLight => Theme::fox_light(),
                    _ => Theme::fox_dark(),
                };
                // Update terminal default colors for all panes
                for tab in &mut self.tabs {
                    for pane in &mut tab.panes {
                        pane.terminal.set_default_colors(
                            self.theme.terminal_fg,
                            Color8::TRANSPARENT,
                            self.theme.terminal_bold,
                        );
                    }
                }
                self.config.save();
                self.update_grid_size();
            }
            ui_chrome::ClickAction::SplitHorizontal => {
                self.split_pane(SplitDir::Horizontal);
            }
            ui_chrome::ClickAction::SplitVertical => {
                self.split_pane(SplitDir::Vertical);
            }
            ui_chrome::ClickAction::ToggleSidebar => {
                self.sidebar.toggle();
                if self.sidebar.visible && !self.tabs.is_empty() {
                    let tab = &self.tabs[self.active_tab];
                    let pane = &tab.panes[tab.active_pane];
                    let cwd = pane
                        .terminal
                        .osc7_cwd
                        .clone()
                        .or_else(|| pane.pty.cwd())
                        .unwrap_or_else(|| {
                            std::env::var("HOME").unwrap_or_else(|_| "/".into())
                        });
                    self.sidebar.set_root(std::path::Path::new(&cwd));
                }
                self.update_grid_size();
            }
            ui_chrome::ClickAction::ClosePane => {
                if self.close_pane() {
                    event_loop.exit();
                    return EventResult::Exit;
                }
            }
            ui_chrome::ClickAction::FocusPrevPane => {
                if !self.tabs.is_empty() {
                    let tab = &mut self.tabs[self.active_tab];
                    if tab.panes.len() > 1 {
                        if tab.active_pane == 0 {
                            tab.active_pane = tab.panes.len() - 1;
                        } else {
                            tab.active_pane -= 1;
                        }
                    }
                }
            }
            ui_chrome::ClickAction::FocusNextPane => {
                if !self.tabs.is_empty() {
                    let tab = &mut self.tabs[self.active_tab];
                    if tab.panes.len() > 1 {
                        tab.active_pane = (tab.active_pane + 1) % tab.panes.len();
                    }
                }
            }
            ui_chrome::ClickAction::Copy => {
                if !self.tabs.is_empty() {
                    let tab = &self.tabs[self.active_tab];
                    let terminal = &tab.panes[tab.active_pane].terminal;
                    input::do_copy(terminal, &self.clipboard);
                }
            }
            ui_chrome::ClickAction::Paste => {
                if !self.tabs.is_empty() {
                    let tab = &self.tabs[self.active_tab];
                    let pane = &tab.panes[tab.active_pane];
                    input::do_paste(&self.clipboard, &pane.pty);
                }
            }
            ui_chrome::ClickAction::SelectAll => {
                if !self.tabs.is_empty() {
                    let tab = &mut self.tabs[self.active_tab];
                    let terminal = &mut tab.panes[tab.active_pane].terminal;
                    terminal.set_selection_anchor(0, 0);
                    let last_row = terminal.rows.saturating_sub(1);
                    let last_col = terminal.cols.saturating_sub(1);
                    terminal.set_selection_end(last_row, last_col);
                }
            }
            ui_chrome::ClickAction::None => {
                return self.handle_click_passthrough(screen_h);
            }
        }
        EventResult::Continue
    }

    fn handle_click_passthrough(&mut self, screen_h: u32) -> EventResult {
        let chrome_h = self.chrome_height();

        // Check sidebar mode toggle first
        if let Some(new_mode) = sidebar::handle_mode_click(&mut self.sidebar, self.cursor_pos, chrome_h) {
            self.handle_sidebar_mode_change(new_mode);
            self.request_redraw();
            return EventResult::Handled;
        }

        // Git sidebar click handling
        if self.sidebar.visible && self.sidebar.mode == sidebar::SidebarMode::Git {
            let git_top = chrome_h + sidebar::TOGGLE_H;
            if git_sidebar::contains(self.cursor_pos, self.sidebar.width, git_top) {
                let action = git_sidebar::handle_click(
                    &mut self.git_sidebar,
                    self.cursor_pos,
                    self.sidebar.width,
                    git_top,
                );
                self.dispatch_git_action(action);
                self.request_redraw();
                return EventResult::Handled;
            }
        }

        // Check file sidebar click
        if sidebar::contains(&self.sidebar, self.cursor_pos, chrome_h) {
            let ctrl = self.modifiers.contains(ModifiersState::CONTROL);
            let result = sidebar::handle_click(
                &mut self.sidebar,
                self.cursor_pos,
                chrome_h,
                screen_h,
                ctrl,
            );
            match result {
                sidebar::ClickResult::CopyPath(path_str) => {
                    if let Some(ref cb) = self.clipboard {
                        cb.set_text(&path_str);
                    }
                }
                _ => {}
            }
            self.request_redraw();
            return EventResult::Handled;
        }

        // Check scrollbar click
        if let Some((cx, cy)) = self.cursor_pos {
            if let Some(hit) = self.scrollbar_hit_test(cx, cy) {
                self.scrollbar_dragging = true;
                self.scroll_to_scrollbar(cy, &hit);
                self.request_redraw();
                return EventResult::Handled;
            }
        }

        // Click wasn't on chrome — check for hyperlink click or start text selection
        if !self.chrome.has_overlay() {
            if let Some((x, y)) = self.cursor_pos {
                if let Some((pane_idx, row, col)) = self.pixel_to_pane_cell(x, y) {
                    if !self.tabs.is_empty() {
                        // Ctrl+Click on hyperlink opens the URL
                        let ctrl = self.modifiers.contains(ModifiersState::CONTROL);
                        if ctrl {
                            let tab = &self.tabs[self.active_tab];
                            if let Some(url) = tab.panes[pane_idx].terminal.hyperlink_at(row, col) {
                                let url = url.to_string();
                                std::thread::spawn(move || {
                                    let _ = std::process::Command::new("xdg-open")
                                        .arg(&url)
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .status();
                                });
                                self.request_redraw();
                                return EventResult::Handled;
                            }
                        }

                        let tab = &mut self.tabs[self.active_tab];
                        tab.active_pane = pane_idx;
                        let terminal = &mut tab.panes[pane_idx].terminal;
                        terminal.set_selection_anchor(row, col);
                        terminal.set_selection_end(row, col);
                        self.selecting = true;
                    }
                }
            }
        }
        EventResult::Continue
    }

    pub(crate) fn handle_left_release(&mut self) {
        self.left_pressed = false;
        self.scrollbar_dragging = false;
        self.input.on_left_released();
        tab_bar::handle_drag_end(&mut self.tab_bar);
        if self.selecting && !self.tabs.is_empty() {
            let tab = &mut self.tabs[self.active_tab];
            let pane = &mut tab.panes[tab.active_pane];
            if pane.terminal.selection_anchor == pane.terminal.selection_end {
                pane.terminal.clear_selection();
            }
        }
        self.selecting = false;
        self.request_redraw();
    }

    pub(crate) fn handle_right_press(&mut self) {
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        let chrome_h = self.chrome_height();
        // Sidebar right-click
        if sidebar::handle_right_click(
            &mut self.sidebar,
            self.cursor_pos,
            chrome_h,
        ) {
            self.chrome.close_all_menus();
            self.tab_bar.context_menu = None;
            self.request_redraw();
            return;
        }

        let tab_displays: Vec<tab_bar::TabDisplay> = self
            .tabs
            .iter()
            .map(|t| {
                let title = t.custom_name.as_deref().unwrap_or_else(|| {
                    t.panes.get(t.active_pane).map_or("Shell", |p| p.title.as_str())
                });
                tab_bar::TabDisplay { title, pinned: t.pinned }
            })
            .collect();
        let menus = ui_chrome::build_menus(
            self.effective_font_size(),
            self.config.window.opacity,
            self.sidebar.visible,
            &self.config.window.mode,
        );
        let tabs_rect = ui_chrome::tabs_bounds(
            &menus,
            screen_w as f32,
            1.0,
            &self.config.window.mode,
        );
        if tab_bar::handle_right_click(
            &mut self.tab_bar,
            self.cursor_pos,
            &tab_displays,
            tabs_rect,
        ) {
            self.chrome.close_all_menus();
            self.sidebar.context_menu = None;
            self.request_redraw();
        } else if let Some((x, y)) = self.cursor_pos {
            self.tab_bar.context_menu = None;
            self.chrome.menu_bar.close();

            // Build context menu items
            let has_selection = if !self.tabs.is_empty() {
                let tab = &self.tabs[self.active_tab];
                tab.panes[tab.active_pane]
                    .terminal
                    .selection_range()
                    .is_some()
            } else {
                false
            };
            let items = ui_chrome::build_context_menu(has_selection);
            self.chrome.context_menu.open(x, y, items);
            self.chrome.context_menu.clamp_to_screen(screen_w as f32, screen_h as f32);
            self.request_redraw();
        }
    }

    pub(crate) fn handle_keyboard(
        &mut self,
        event: &winit::event::KeyEvent,
        event_loop: &ActiveEventLoop,
    ) -> EventResult {
        // Git sidebar commit input captures ALL keyboard input
        if self.git_sidebar.is_capturing_input() {
            if event.state == ElementState::Pressed {
                let key_str = match &event.logical_key {
                    winit::keyboard::Key::Named(n) => match n {
                        winit::keyboard::NamedKey::Enter => {
                            // Enter commits (if there's a message)
                            if !self.git_sidebar.commit_msg.trim().is_empty() {
                                let action = git_sidebar::GitAction::Commit;
                                self.dispatch_git_action(action);
                            }
                            self.request_redraw();
                            return EventResult::Handled;
                        }
                        winit::keyboard::NamedKey::Escape => Some("Escape"),
                        winit::keyboard::NamedKey::Backspace => Some("Backspace"),
                        winit::keyboard::NamedKey::Delete => Some("Delete"),
                        winit::keyboard::NamedKey::ArrowLeft => Some("Left"),
                        winit::keyboard::NamedKey::ArrowRight => Some("Right"),
                        winit::keyboard::NamedKey::Home => Some("Home"),
                        winit::keyboard::NamedKey::End => Some("End"),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(key) = key_str {
                    git_sidebar::handle_key(&mut self.git_sidebar, key);
                } else if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) = &event.logical_key {
                    git_sidebar::handle_char(&mut self.git_sidebar, ' ');
                } else if let winit::keyboard::Key::Character(s) = &event.logical_key {
                    for ch in s.chars() {
                        git_sidebar::handle_char(&mut self.git_sidebar, ch);
                    }
                }
            }
            self.request_redraw();
            return EventResult::Handled;
        }

        // Sidebar inline edit captures ALL keyboard input
        if self.sidebar.is_editing() {
            if event.state == ElementState::Pressed {
                let key_str = match &event.logical_key {
                    winit::keyboard::Key::Named(n) => match n {
                        winit::keyboard::NamedKey::Enter => Some("Enter"),
                        winit::keyboard::NamedKey::Escape => Some("Escape"),
                        winit::keyboard::NamedKey::Backspace => Some("Backspace"),
                        winit::keyboard::NamedKey::Delete => Some("Delete"),
                        winit::keyboard::NamedKey::ArrowLeft => Some("Left"),
                        winit::keyboard::NamedKey::ArrowRight => Some("Right"),
                        winit::keyboard::NamedKey::Home => Some("Home"),
                        winit::keyboard::NamedKey::End => Some("End"),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(key) = key_str {
                    sidebar::handle_edit_key(&mut self.sidebar, key);
                } else if let winit::keyboard::Key::Character(s) = &event.logical_key {
                    for ch in s.chars() {
                        sidebar::handle_edit_char(&mut self.sidebar, ch);
                    }
                }
            }
            self.update_grid_size();
            self.request_redraw();
            return EventResult::Handled;
        }

        // Tab rename mode captures ALL keyboard input (press and release)
        if tab_bar::is_capturing_input(&self.tab_bar) {
            if event.state == ElementState::Pressed {
                let key_str = match &event.logical_key {
                    winit::keyboard::Key::Named(n) => match n {
                        winit::keyboard::NamedKey::Enter => Some("Enter"),
                        winit::keyboard::NamedKey::Escape => Some("Escape"),
                        winit::keyboard::NamedKey::Backspace => Some("Backspace"),
                        winit::keyboard::NamedKey::Delete => Some("Delete"),
                        winit::keyboard::NamedKey::ArrowLeft => Some("Left"),
                        winit::keyboard::NamedKey::ArrowRight => Some("Right"),
                        winit::keyboard::NamedKey::Home => Some("Home"),
                        winit::keyboard::NamedKey::End => Some("End"),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(key) = key_str {
                    if let Some(action) = tab_bar::handle_rename_key(&mut self.tab_bar, key) {
                        self.handle_tab_bar_action(action, event_loop);
                    }
                } else if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) = &event.logical_key {
                    tab_bar::handle_rename_char(&mut self.tab_bar, ' ');
                } else if let winit::keyboard::Key::Character(s) = &event.logical_key {
                    for ch in s.chars() {
                        tab_bar::handle_rename_char(&mut self.tab_bar, ch);
                    }
                }
            }
            // Consume all events (press + release) while renaming
            self.request_redraw();
            return EventResult::Handled;
        }

        if event.state == ElementState::Pressed {
            // Close menus on Escape
            if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) =
                &event.logical_key
            {
                if self.chrome.has_overlay() || self.tab_bar.has_overlay() {
                    self.chrome.close_all_menus();
                    self.tab_bar.context_menu = None;
                    self.request_redraw();
                    return EventResult::Handled;
                }
            }

            let ctrl = self.modifiers.contains(ModifiersState::CONTROL);
            let shift = self.modifiers.contains(ModifiersState::SHIFT);

            // Tab and pane management shortcuts
            if ctrl && shift {
                if let Some(result) = self.handle_ctrl_shift_key(&event.logical_key, event_loop) {
                    return result;
                }
            }

            // Ctrl+Tab / Ctrl+Shift+Tab for tab switching
            if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) =
                &event.logical_key
            {
                if ctrl && self.tabs.len() > 1 {
                    if shift {
                        if self.active_tab == 0 {
                            self.active_tab = self.tabs.len() - 1;
                        } else {
                            self.active_tab -= 1;
                        }
                    } else {
                        self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    }
                    self.cursor_visible = true;
                    self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;
                    self.request_redraw();
                    return EventResult::Handled;
                }
            }
        }

        if !self.tabs.is_empty() {
            let font_size = self.effective_font_size();
            let tab = &mut self.tabs[self.active_tab];
            let pane = &mut tab.panes[tab.active_pane];
            let old_offset = pane.terminal.scroll_offset;
            input::handle_key(
                &event.logical_key,
                event.state,
                self.modifiers,
                &mut pane.terminal,
                &pane.pty,
                &self.clipboard,
            );
            if pane.terminal.scroll_offset != old_offset {
                let cell_h = render::measure_cell(font_size).1;
                let new_px = pane.terminal.scroll_offset as f32 * cell_h;
                self.scroll_target_px = new_px;
                if pane.terminal.scroll_offset == 0 {
                    self.scroll_current_px = 0.0;
                    self.scroll_animating = false;
                } else {
                    self.scroll_animating = true;
                }
            }
        }
        self.request_redraw();
        EventResult::Continue
    }

    fn handle_ctrl_shift_key(
        &mut self,
        key: &winit::keyboard::Key,
        event_loop: &ActiveEventLoop,
    ) -> Option<EventResult> {
        match key {
            winit::keyboard::Key::Character(s) if s.eq_ignore_ascii_case("g") => {
                // Toggle git sidebar
                if !self.sidebar.visible {
                    self.sidebar.visible = true;
                    self.sidebar.mode = sidebar::SidebarMode::Git;
                    self.handle_sidebar_mode_change(sidebar::SidebarMode::Git);
                } else if self.sidebar.mode == sidebar::SidebarMode::Git {
                    self.sidebar.visible = false;
                } else {
                    self.sidebar.mode = sidebar::SidebarMode::Git;
                    self.handle_sidebar_mode_change(sidebar::SidebarMode::Git);
                }
                self.update_grid_size();
                self.request_redraw();
                Some(EventResult::Handled)
            }
            winit::keyboard::Key::Character(s) if s.eq_ignore_ascii_case("t") => {
                self.spawn_tab();
                self.request_redraw();
                Some(EventResult::Handled)
            }
            winit::keyboard::Key::Character(s) if s.eq_ignore_ascii_case("w") => {
                if self.close_pane() {
                    event_loop.exit();
                    return Some(EventResult::Exit);
                }
                self.request_redraw();
                Some(EventResult::Handled)
            }
            winit::keyboard::Key::Character(s) if s.eq_ignore_ascii_case("d") => {
                self.split_pane(SplitDir::Horizontal);
                self.request_redraw();
                Some(EventResult::Handled)
            }
            winit::keyboard::Key::Character(s) if s.eq_ignore_ascii_case("e") => {
                self.split_pane(SplitDir::Vertical);
                self.request_redraw();
                Some(EventResult::Handled)
            }
            winit::keyboard::Key::Character(s)
                if s.as_str() == "[" || s.as_str() == "{" =>
            {
                if !self.tabs.is_empty() {
                    let tab = &mut self.tabs[self.active_tab];
                    if tab.panes.len() > 1 {
                        if tab.active_pane == 0 {
                            tab.active_pane = tab.panes.len() - 1;
                        } else {
                            tab.active_pane -= 1;
                        }
                    }
                }
                self.request_redraw();
                Some(EventResult::Handled)
            }
            winit::keyboard::Key::Character(s)
                if s.as_str() == "]" || s.as_str() == "}" =>
            {
                if !self.tabs.is_empty() {
                    let tab = &mut self.tabs[self.active_tab];
                    if tab.panes.len() > 1 {
                        tab.active_pane = (tab.active_pane + 1) % tab.panes.len();
                    }
                }
                self.request_redraw();
                Some(EventResult::Handled)
            }
            _ => None,
        }
    }

    pub(crate) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let chrome_h = self.chrome_height();

        // Git sidebar scroll
        if self.sidebar.visible && self.sidebar.mode == sidebar::SidebarMode::Git {
            let git_top = chrome_h + sidebar::TOGGLE_H;
            if git_sidebar::contains(self.cursor_pos, self.sidebar.width, git_top) {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 20.0,
                };
                self.git_sidebar.scroll(dy);
                self.request_redraw();
                return;
            }
        }

        // File sidebar scroll
        if sidebar::contains(&self.sidebar, self.cursor_pos, chrome_h) {
            let dy = match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 20.0,
            };
            self.sidebar.scroll(dy);
            self.request_redraw();
            return;
        }

        if self.tabs.is_empty() {
            return;
        }

        let cell_h = render::measure_cell(self.effective_font_size()).1;
        let delta_px = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * cell_h * 10.0,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 4.0,
        };

        let tab = &self.tabs[self.active_tab];
        let terminal = &tab.panes[tab.active_pane].terminal;
        let max_px = terminal.scrollback.len() as f32 * cell_h;

        self.scroll_target_px = (self.scroll_target_px + delta_px).clamp(0.0, max_px);
        self.scroll_animating = true;
        self.request_redraw();
    }

    pub(crate) fn handle_slider_drags(&mut self) {
        if self.scrollbar_dragging {
            if let Some((_, cy)) = self.cursor_pos {
                if let Some(hit) = self.scrollbar_hit_test(0.0, cy) {
                    self.scroll_to_scrollbar(cy, &hit);
                    self.request_redraw();
                }
            }
        }
    }

    /// Build scrollbar state for the active pane. Returns None if no scrollbar is visible.
    fn scrollbar_hit_test(&self, cx: f32, cy: f32) -> Option<ScrollbarHit> {
        if self.tabs.is_empty() {
            return None;
        }
        let cell_h = render::measure_cell(self.effective_font_size()).1;
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        let tab = &self.tabs[self.active_tab];
        let pane = &tab.panes[tab.active_pane];
        let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, self.sidebar_offset(), self.chrome_height());
        if tab.active_pane >= rects.len() {
            return None;
        }
        let (gx, gy, gw, gh) =
            Self::pane_grid_bounds(pane, rects[tab.active_pane], self.effective_font_size());
        let viewport = lntrn_render::Rect::new(gx, gy, gw, gh);
        let total_lines = pane.terminal.scrollback.len() + pane.terminal.rows;
        let content_height = total_lines as f32 * cell_h;
        let max_scroll = (content_height - gh).max(0.0);
        let inverted_offset = max_scroll - self.scroll_current_px.min(max_scroll);
        let scrollbar =
            lntrn_ui::gpu::scroll::Scrollbar::new(&viewport, content_height, inverted_offset);

        // For drag updates we skip the hit test (cx=0.0 sentinel)
        if cx == 0.0 || scrollbar.hover_zone().contains(cx, cy) {
            Some(ScrollbarHit {
                content_height,
                max_scroll,
            })
        } else {
            None
        }
    }

    fn scroll_to_scrollbar(&mut self, cy: f32, hit: &ScrollbarHit) {
        // Rebuild scrollbar for offset_for_thumb_y (lightweight, no alloc)
        if self.tabs.is_empty() {
            return;
        }
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        let tab = &self.tabs[self.active_tab];
        let pane = &tab.panes[tab.active_pane];
        let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, self.sidebar_offset(), self.chrome_height());
        if tab.active_pane >= rects.len() {
            return;
        }
        let (gx, gy, gw, gh) =
            Self::pane_grid_bounds(pane, rects[tab.active_pane], self.effective_font_size());
        let viewport = lntrn_render::Rect::new(gx, gy, gw, gh);
        let inverted_offset = hit.max_scroll - self.scroll_current_px.min(hit.max_scroll);
        let scrollbar = lntrn_ui::gpu::scroll::Scrollbar::new(
            &viewport,
            hit.content_height,
            inverted_offset,
        );
        let raw = scrollbar.offset_for_thumb_y(cy, hit.content_height, gh);
        let new_offset = hit.max_scroll - raw;
        self.scroll_target_px = new_offset;
        self.scroll_current_px = new_offset;
    }
}

struct ScrollbarHit {
    content_height: f32,
    max_scroll: f32,
}

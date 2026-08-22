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

        // Sidebar resize drag — owns the pointer while active.
        if self.sidebar.resizing {
            self.sidebar.resize_to(x);
            self.update_grid_size();
            if let Some(ref window) = self.window {
                window.set_cursor(CursorIcon::ColResize);
            }
            self.request_redraw();
            return EventResult::Handled;
        }

        // Tab drag reorder
        if self.tab_bar.dragging.is_some() {
            let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
            let drag_displays: Vec<tab_bar::TabDisplay> = self
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
                self.sidebar.visible,
                self.config.general.cursor_style,
                self.config.general.open_chrome_hidden,
                &crate::config::WindowMode::current(),
            );
            let bounds = ui_chrome::tabs_bounds(
                &menus,
                screen_w as f32,
                self.scale,
                &crate::config::WindowMode::current(),
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
                tab.panes[tab.active_pane]
                    .terminal
                    .set_selection_end(row, col);
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
                tab.panes[tab.active_pane]
                    .terminal
                    .hyperlink_at(row, col)
                    .is_some()
            } else {
                false
            }
        } else {
            false
        };

        let chrome_h = self.chrome_height();
        let on_resize_handle = self.sidebar.resize_handle_hit(self.cursor_pos, chrome_h);
        if self.selecting {
            if let Some(ref window) = self.window {
                window.set_cursor(CursorIcon::Text);
            }
        } else if on_resize_handle {
            if let Some(ref window) = self.window {
                window.set_cursor(CursorIcon::ColResize);
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

        // Rice mode: skip all chrome/tab/menubar click routing — go straight
        // to the terminal-selection passthrough. Exception: the right-click
        // context menu still works here. Clicks inside it land via its
        // interaction zones during the overlay draw; clicks outside close it
        // instead of starting a selection underneath.
        if self.chrome_hidden {
            self.input.on_left_pressed();
            if self.chrome.context_menu.is_open() {
                if let Some((x, y)) = self.cursor_pos {
                    if !self.chrome.context_menu.contains(x, y) {
                        self.chrome.context_menu.close();
                        self.sidebar.close_menu();
                    }
                }
                self.request_redraw();
                return EventResult::Handled;
            }
            self.request_redraw();
            return self.handle_click_passthrough(screen_h);
        }

        // Open right-click context menu takes priority over everything else
        // (same rule as rice mode above): clicks inside land via its
        // interaction zones during the overlay draw; clicks outside dismiss
        // it without activating whatever sits underneath (sidebar rows,
        // tabs, terminal selection).
        if self.chrome.context_menu.is_open() {
            self.input.on_left_pressed();
            if let Some((x, y)) = self.cursor_pos {
                if !self.chrome.context_menu.contains(x, y) {
                    self.chrome.context_menu.close();
                    self.sidebar.close_menu();
                }
            }
            self.request_redraw();
            return EventResult::Handled;
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
                self.sidebar.visible,
                self.config.general.cursor_style,
                self.config.general.open_chrome_hidden,
                &crate::config::WindowMode::current(),
            );
            let tabs_rect = ui_chrome::tabs_bounds(
                &menus,
                screen_w as f32,
                self.scale,
                &crate::config::WindowMode::current(),
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

        // Intercept "Files" label click BEFORE on_left_pressed so the
        // InteractionContext never sees it — prevents the menu bar from
        // opening a dropdown that captures all input.
        if let Some((x, y)) = self.cursor_pos {
            if y <= self.chrome_height() {
                if ui_chrome::is_files_label_hit(
                    x,
                    &crate::config::WindowMode::current(),
                    self.scale,
                ) {
                    self.chrome.close_all_menus();
                    match self.dispatch_chrome_action(
                        ui_chrome::ClickAction::ToggleSidebar,
                        event_loop,
                        screen_h,
                    ) {
                        EventResult::Exit => return EventResult::Exit,
                        _ => {}
                    }
                    self.request_redraw();
                    return EventResult::Handled;
                }
            }
        }

        self.input.on_left_pressed();
        let menus = ui_chrome::build_menus(
            self.effective_font_size(),
            self.sidebar.visible,
            self.config.general.cursor_style,
            self.config.general.open_chrome_hidden,
            &crate::config::WindowMode::current(),
        );
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width()) as f32;
        let action = ui_chrome::handle_click(
            &mut self.chrome,
            &mut self.input,
            &menus,
            self.scale,
            &crate::config::WindowMode::current(),
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
            ui_chrome::ClickAction::WindowModeChanged => {
                // Theme now comes from System Settings (`[appearance].theme`);
                // re-resolve and propagate to every open pane's terminal.
                use crate::terminal::Color8;
                use crate::theme::Theme;
                self.theme = Theme::current();
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
                        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()));
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
                    input::do_paste(&self.clipboard, &pane.terminal, &pane.pty);
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
            ui_chrome::ClickAction::NewTab => {
                self.spawn_tab();
            }
            ui_chrome::ClickAction::CloseTab => {
                if self.close_tab(self.active_tab) {
                    event_loop.exit();
                    return EventResult::Exit;
                }
            }
            ui_chrome::ClickAction::PrevTab => {
                if self.tabs.len() > 1 {
                    self.active_tab = if self.active_tab == 0 {
                        self.tabs.len() - 1
                    } else {
                        self.active_tab - 1
                    };
                    self.cursor_visible = true;
                    self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;
                }
            }
            ui_chrome::ClickAction::NextTab => {
                if self.tabs.len() > 1 {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    self.cursor_visible = true;
                    self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;
                }
            }
            ui_chrome::ClickAction::SelectTab(i) => {
                if i < self.tabs.len() && i != self.active_tab {
                    self.active_tab = i;
                    self.cursor_visible = true;
                    self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;
                }
            }
            ui_chrome::ClickAction::RunLntrn => {
                if !self.tabs.is_empty() {
                    let tab = &self.tabs[self.active_tab];
                    // 0x0D = Enter, same byte the keyboard path sends.
                    tab.panes[tab.active_pane].pty.write(b"lntrn\r");
                }
            }
            ui_chrome::ClickAction::ClearScrollback => {
                if !self.tabs.is_empty() {
                    let tab = &mut self.tabs[self.active_tab];
                    tab.panes[tab.active_pane].terminal.clear_scrollback();
                }
            }
            ui_chrome::ClickAction::SidebarNewFile => {
                self.sidebar.menu_new_file();
            }
            ui_chrome::ClickAction::SidebarNewFolder => {
                self.sidebar.menu_new_folder();
            }
            ui_chrome::ClickAction::SidebarRename => {
                self.sidebar.menu_rename();
            }
            ui_chrome::ClickAction::SidebarDelete => {
                self.sidebar.menu_delete();
            }
            ui_chrome::ClickAction::SidebarOpenCode => {
                self.sidebar.menu_open_code();
            }
            ui_chrome::ClickAction::None => {
                return self.handle_click_passthrough(screen_h);
            }
        }
        EventResult::Continue
    }

    fn handle_click_passthrough(&mut self, screen_h: u32) -> EventResult {
        let chrome_h = self.chrome_height();

        // Resize handle on the sidebar's right edge takes priority over content
        // hits so a drag starting at the edge never lands on a list row. A
        // double-click on the handle resets to auto-fit width.
        if self.sidebar.resize_handle_hit(self.cursor_pos, chrome_h) {
            let now = Instant::now();
            let double = now
                .duration_since(self.last_resize_handle_click)
                .as_millis()
                < 400;
            self.last_resize_handle_click = now;
            if double {
                self.sidebar.reset_width();
                self.config.sidebar.width = None;
                self.config.save();
                self.update_grid_size();
            } else {
                self.sidebar.begin_resize();
            }
            self.request_redraw();
            return EventResult::Handled;
        }

        // Check sidebar mode toggle first
        if let Some(new_mode) =
            sidebar::handle_mode_click(&mut self.sidebar, self.cursor_pos, chrome_h)
        {
            self.handle_sidebar_mode_change(new_mode);
            self.request_redraw();
            return EventResult::Handled;
        }

        // Git sidebar click handling
        if self.sidebar.visible && self.sidebar.mode == sidebar::SidebarMode::Git {
            let git_top = chrome_h + sidebar::TOGGLE_H * self.sidebar.scale;
            if git_sidebar::contains(
                self.cursor_pos,
                self.sidebar.width,
                git_top,
                self.git_sidebar.scale,
            ) {
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
            let result =
                sidebar::handle_click(&mut self.sidebar, self.cursor_pos, chrome_h, screen_h, ctrl);
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
        if self.sidebar.end_resize() {
            // Persist the dragged width so it survives restarts.
            self.config.sidebar.width = self.sidebar.manual_width;
            self.config.save();
        }
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
        // Sidebar right-click — stays routable in rice mode (the handler
        // no-ops when the sidebar is closed).
        if let Some(target) =
            sidebar::handle_right_click(&mut self.sidebar, self.cursor_pos, chrome_h)
        {
            self.open_sidebar_context_menu(&target, screen_w, screen_h);
            return;
        }

        // Rice mode: the tab bar isn't drawn, so its hit region belongs to
        // the terminal grid — go straight to the terminal menu.
        if self.chrome_hidden {
            self.open_terminal_context_menu(screen_w, screen_h);
            return;
        }

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
            self.sidebar.visible,
            self.config.general.cursor_style,
            self.config.general.open_chrome_hidden,
            &crate::config::WindowMode::current(),
        );
        let tabs_rect = ui_chrome::tabs_bounds(
            &menus,
            screen_w as f32,
            self.scale,
            &crate::config::WindowMode::current(),
        );
        if tab_bar::handle_right_click(&mut self.tab_bar, self.cursor_pos, &tab_displays, tabs_rect)
        {
            self.chrome.close_all_menus();
            self.sidebar.close_menu();
            self.request_redraw();
        } else {
            self.open_terminal_context_menu(screen_w, screen_h);
        }
    }

    /// Open the sidebar's context menu — the same chrome ContextMenu the
    /// terminal uses, with file-tree items for the right-clicked entry.
    fn open_sidebar_context_menu(
        &mut self,
        target: &sidebar::RightClickTarget,
        screen_w: u32,
        screen_h: u32,
    ) {
        let Some((x, y)) = self.cursor_pos else {
            return;
        };
        self.tab_bar.context_menu = None;
        self.chrome.menu_bar.close();

        let items =
            ui_chrome::build_sidebar_context_menu(&target.name, target.is_root, target.is_dir);
        self.chrome.refresh_theme();
        self.chrome.context_menu.set_scale(self.scale);
        self.chrome.context_menu.open(x, y, items);
        self.chrome
            .context_menu
            .clamp_to_screen(screen_w as f32, screen_h as f32);
        self.request_redraw();
    }

    /// Open the terminal's right-click context menu at the cursor. Shared by
    /// the normal path and rice mode (where it doubles as the title bar).
    fn open_terminal_context_menu(&mut self, screen_w: u32, screen_h: u32) {
        let Some((x, y)) = self.cursor_pos else {
            return;
        };
        self.tab_bar.context_menu = None;
        self.sidebar.close_menu();
        self.chrome.menu_bar.close();

        let items = self.context_menu_items();
        self.chrome.refresh_theme();
        self.chrome.context_menu.set_scale(self.scale);
        self.chrome.context_menu.open(x, y, items);
        self.chrome
            .context_menu
            .clamp_to_screen(screen_w as f32, screen_h as f32);
        self.request_redraw();
    }

    /// Context-menu items for the current terminal state.
    fn context_menu_items(&self) -> Vec<lntrn_ui::gpu::MenuItem> {
        let has_selection = self.tabs.get(self.active_tab).map_or(false, |t| {
            t.panes[t.active_pane].terminal.selection_range().is_some()
        });
        let pane_count = self.tabs.get(self.active_tab).map_or(0, |t| t.panes.len());
        ui_chrome::build_context_menu(
            has_selection,
            self.tabs.len(),
            self.active_tab,
            pane_count,
            self.sidebar.visible,
            self.effective_font_size(),
        )
    }

    /// Rebuild the open context menu's items in place so live state (active
    /// tab dot, chevrons, pane count) stays current after actions that keep
    /// the menu open.
    pub(crate) fn refresh_context_menu_items(&mut self) {
        if !self.chrome.context_menu.is_open() {
            return;
        }
        let items = self.context_menu_items();
        self.chrome.context_menu.replace_items(items);
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
                } else if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) =
                    &event.logical_key
                {
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
                } else if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) =
                    &event.logical_key
                {
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

            // Super+F11: toggle "rice mode" — hides the title/tab bar for
            // screenshots / fastfetch glamour. The sidebar keeps following
            // its own visible flag. The compositor lets Super+F11 fall
            // through; plain F11 still toggles compositor fullscreen.
            if self.modifiers.contains(ModifiersState::SUPER) {
                if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::F11) =
                    &event.logical_key
                {
                    self.chrome_hidden = !self.chrome_hidden;
                    if self.chrome_hidden {
                        self.chrome.close_all_menus();
                        self.tab_bar.context_menu = None;
                    }
                    self.update_grid_size();
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
            if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) = &event.logical_key
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
            winit::keyboard::Key::Character(s) if s.as_str() == "[" || s.as_str() == "{" => {
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
            winit::keyboard::Key::Character(s) if s.as_str() == "]" || s.as_str() == "}" => {
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
            let git_top = chrome_h + sidebar::TOGGLE_H * self.sidebar.scale;
            if git_sidebar::contains(
                self.cursor_pos,
                self.sidebar.width,
                git_top,
                self.git_sidebar.scale,
            ) {
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

        if self.forward_wheel_to_tui(delta) {
            return;
        }

        let cell_h = render::measure_cell(self.effective_font_size()).1;
        // PixelDelta: one wheel detent arrives as 15 logical px × scale
        // (compositor-synthesized), so ×8 ≈ 6 lines per detent; trackpads
        // scroll at 8× finger speed. LineDelta (non-Lantern fallback):
        // detents arrive pre-quantized, so scale by cell height directly.
        let delta_px = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * cell_h * self.scroll_speed,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * self.scroll_speed,
        };

        let tab = &self.tabs[self.active_tab];
        let terminal = &tab.panes[tab.active_pane].terminal;
        let max_px = terminal.active_scrollback().len() as f32 * cell_h;

        self.scroll_target_px = (self.scroll_target_px + delta_px).clamp(0.0, max_px);
        self.scroll_animating = true;
        self.request_redraw();
    }

    /// Wheel handling for TUIs that own their viewport. Apps that enabled
    /// mouse reporting (Claude Code, htop, vim `mouse=a`) get each wheel
    /// tick forwarded as a mouse-button report at the hovered cell;
    /// alt-screen apps without mouse reporting (`less`, `man`) get arrow
    /// keys instead (alternate scroll). Returns false for everything else
    /// so the caller scrolls our scrollback — which is the only thing the
    /// wheel could do before, and on the alt screen meant scrolling an
    /// empty buffer, i.e. a dead wheel.
    ///
    /// One tick = one wheel detent: the compositor synthesizes 15 logical
    /// px of continuous scroll per detent (see lntrn-compositor
    /// `handle_pointer_axis`), which winit hands us ×scale as PixelDelta.
    /// Trackpads integrate their finger deltas on the same scale.
    fn forward_wheel_to_tui(&mut self, delta: MouseScrollDelta) -> bool {
        let tab = &self.tabs[self.active_tab];
        let pane = &tab.panes[tab.active_pane];
        let terminal = &pane.terminal;
        let mouse_on = terminal.mouse_mode != crate::terminal::MouseMode::Off;
        if !mouse_on && !terminal.is_alt_screen() {
            return false;
        }

        self.wheel_tick_accum += match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / (15.0 * self.scale),
        };
        let ticks = self.wheel_tick_accum as i32;
        self.wheel_tick_accum -= ticks as f32;
        if ticks == 0 {
            // Consumed: the sub-detent remainder stays accumulated.
            return true;
        }
        let up = ticks > 0;

        let bytes = if mouse_on {
            let (col, row) = self.hovered_cell(tab, pane);
            let report = crate::terminal::mouse::wheel_report(terminal.mouse_sgr, up, col, row);
            report.repeat(ticks.unsigned_abs() as usize)
        } else {
            crate::terminal::mouse::alternate_scroll(
                up,
                terminal.application_cursor,
                ticks.unsigned_abs() as usize,
            )
        };
        pane.pty.write(&bytes);
        true
    }

    /// 1-based cell coordinates under the pointer, clamped into the active
    /// pane's grid — mouse reports have no notion of "outside the grid".
    fn hovered_cell(&self, tab: &crate::app::Tab, pane: &crate::app::Pane) -> (usize, usize) {
        let font_size = self.effective_font_size();
        let (cell_w, cell_h) = render::measure_cell(font_size);
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        let rects = Self::pane_rects_for_tab(
            tab,
            screen_w,
            screen_h,
            self.sidebar_offset(),
            self.chrome_height(),
        );
        if tab.active_pane >= rects.len() {
            return (1, 1);
        }
        let (gx, gy, _, _) = Self::pane_grid_bounds(pane, rects[tab.active_pane], font_size);
        let (cx, cy) = self.cursor_pos.unwrap_or((gx, gy));
        let col = ((cx - gx) / cell_w).floor().max(0.0) as usize;
        let row = ((cy - gy) / cell_h).floor().max(0.0) as usize;
        (
            col.min(pane.terminal.cols.saturating_sub(1)) + 1,
            row.min(pane.terminal.rows.saturating_sub(1)) + 1,
        )
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
        let rects = Self::pane_rects_for_tab(
            tab,
            screen_w,
            screen_h,
            self.sidebar_offset(),
            self.chrome_height(),
        );
        if tab.active_pane >= rects.len() {
            return None;
        }
        let (gx, gy, gw, gh) =
            Self::pane_grid_bounds(pane, rects[tab.active_pane], self.effective_font_size());
        let viewport = lntrn_render::Rect::new(gx, gy, gw, gh);
        let total_lines = pane.terminal.active_scrollback().len() + pane.terminal.rows;
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
        let rects = Self::pane_rects_for_tab(
            tab,
            screen_w,
            screen_h,
            self.sidebar_offset(),
            self.chrome_height(),
        );
        if tab.active_pane >= rects.len() {
            return;
        }
        let (gx, gy, gw, gh) =
            Self::pane_grid_bounds(pane, rects[tab.active_pane], self.effective_font_size());
        let viewport = lntrn_render::Rect::new(gx, gy, gw, gh);
        let inverted_offset = hit.max_scroll - self.scroll_current_px.min(hit.max_scroll);
        let scrollbar =
            lntrn_ui::gpu::scroll::Scrollbar::new(&viewport, hit.content_height, inverted_offset);
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

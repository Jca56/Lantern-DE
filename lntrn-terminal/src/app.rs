use std::sync::Arc;
use std::time::{Duration, Instant};

use lntrn_render::{GpuContext, Painter, TextRenderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{Icon, Window, WindowAttributes, WindowId};

use lntrn_ui::gpu::InteractionContext;

use crate::clipboard;
use crate::config::LanternConfig;
use crate::events::EventResult;
use crate::pty::Pty;
use crate::render;
use crate::sidebar;
use crate::tab_bar;
use crate::terminal::TerminalState;
use crate::theme::Theme;
use crate::ui_chrome;
use crate::UserEvent;

pub(crate) const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);
const RESIZE_BORDER: f32 = 10.0;

pub struct Pane {
    pub terminal: TerminalState,
    pub pty: Pty,
    pub title: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SplitDir {
    Horizontal, // Side by side (left | right)
    Vertical,   // Stacked (top / bottom)
}

pub struct Tab {
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    pub split: Option<SplitDir>,
    pub pinned: bool,
    pub custom_name: Option<String>,
}

pub(crate) const SPLIT_DIVIDER: f32 = 2.0;

pub struct App {
    pub config: LanternConfig,
    pub theme: Theme,
    pub(crate) proxy: EventLoopProxy<UserEvent>,

    // Initialized on resumed
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) gpu: Option<GpuContext>,
    pub(crate) painter: Option<Painter>,
    pub(crate) overlay_painter: Option<Painter>,
    pub(crate) text: Option<TextRenderer>,
    pub(crate) overlay_text: Option<TextRenderer>,

    // Tabs
    pub tabs: Vec<Tab>,
    pub active_tab: usize,

    // Input state
    pub modifiers: ModifiersState,
    pub cursor_pos: Option<(f32, f32)>,
    pub left_pressed: bool,

    // UI chrome state
    pub chrome: ui_chrome::ChromeState,
    pub tab_bar: tab_bar::TabBarState,
    pub input: InteractionContext,

    // Cursor blink
    pub cursor_visible: bool,
    pub(crate) cursor_blink_deadline: Instant,

    // Clipboard
    pub clipboard: Option<clipboard::WaylandClipboard>,

    // Smooth scrolling
    pub(crate) scroll_target_px: f32,
    pub(crate) scroll_current_px: f32,
    pub(crate) scroll_animating: bool,
    pub(crate) last_frame_time: Instant,

    // Selection drag
    pub(crate) selecting: bool,

    // Scrollbar drag
    pub(crate) scrollbar_dragging: bool,

    // Pending menu action from overlay rendering
    pub(crate) pending_menu_event: Option<ui_chrome::ClickAction>,

    // Sidebar file browser
    pub sidebar: sidebar::SidebarState,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let config = LanternConfig::load();
        let theme = Theme::from_config(&config);

        Self {
            config,
            theme,
            proxy,
            window: None,
            gpu: None,
            painter: None,
            overlay_painter: None,
            text: None,
            overlay_text: None,
            tabs: Vec::new(),
            active_tab: 0,
            modifiers: ModifiersState::empty(),
            cursor_pos: None,
            left_pressed: false,
            chrome: ui_chrome::ChromeState::new(),
            tab_bar: tab_bar::TabBarState::new(),
            input: InteractionContext::new(),
            cursor_visible: true,
            cursor_blink_deadline: Instant::now() + CURSOR_BLINK_INTERVAL,
            clipboard: clipboard::WaylandClipboard::new(),
            scroll_target_px: 0.0,
            scroll_current_px: 0.0,
            scroll_animating: false,
            last_frame_time: Instant::now(),
            selecting: false,
            scrollbar_dragging: false,
            pending_menu_event: None,
            sidebar: sidebar::SidebarState::new(),
        }
    }

    fn init_gpu(&mut self) {
        let window = self.window.as_ref().unwrap();
        let size = window.inner_size();
        let w = size.width.max(1);
        let h = size.height.max(1);

        let gpu =
            GpuContext::from_window(window.as_ref(), w, h).expect("Failed to create GPU context");
        eprintln!(
            "[lntrn-terminal] surface format: {:?}, size: {}x{}",
            gpu.format, w, h
        );
        let painter = Painter::new(&gpu);
        let overlay_painter = Painter::new(&gpu);
        let text = TextRenderer::new_monospace(&gpu);
        let overlay_text = TextRenderer::new_monospace(&gpu);

        self.gpu = Some(gpu);
        self.painter = Some(painter);
        self.overlay_painter = Some(overlay_painter);
        self.text = Some(text);
        self.overlay_text = Some(overlay_text);
    }

    /// Font size scaled to current window width.
    /// At the configured window width (or larger), returns the full config font size.
    /// As the window shrinks, the font scales down proportionally (min 10px).
    pub(crate) fn effective_font_size(&self) -> f32 {
        let base = self.config.font.size;
        let ref_w = self.config.window.width; // logical reference width
        let cur_w = self.gpu.as_ref().map_or(ref_w, |g| g.width() as f32);
        if cur_w >= ref_w {
            return base;
        }
        let scaled = base * cur_w / ref_w;
        scaled.clamp(10.0, base)
    }

    pub(crate) fn sidebar_offset(&self) -> f32 {
        if self.sidebar.visible {
            self.sidebar.width
        } else {
            0.0
        }
    }

    pub(crate) fn pane_rects_for_tab(
        tab: &Tab,
        screen_w: u32,
        screen_h: u32,
        sidebar_offset: f32,
    ) -> Vec<(f32, f32, f32, f32)> {
        let chrome_h = render::chrome_height();
        let avail_w = screen_w as f32 - sidebar_offset;
        let avail_h = screen_h as f32 - chrome_h;
        let x0 = sidebar_offset;
        let n = tab.panes.len();

        if n <= 1 || tab.split.is_none() {
            return vec![(x0, chrome_h, avail_w, avail_h)];
        }

        match tab.split.unwrap() {
            SplitDir::Horizontal => {
                let dividers = (n - 1) as f32 * SPLIT_DIVIDER;
                let pane_w = ((avail_w - dividers) / n as f32).floor();
                (0..n)
                    .map(|i| {
                        let x = x0 + i as f32 * (pane_w + SPLIT_DIVIDER);
                        let w = if i == n - 1 {
                            screen_w as f32 - x
                        } else {
                            pane_w
                        };
                        (x, chrome_h, w, avail_h)
                    })
                    .collect()
            }
            SplitDir::Vertical => {
                let dividers = (n - 1) as f32 * SPLIT_DIVIDER;
                let pane_h = ((avail_h - dividers) / n as f32).floor();
                (0..n)
                    .map(|i| {
                        let y = chrome_h + i as f32 * (pane_h + SPLIT_DIVIDER);
                        let h = if i == n - 1 {
                            screen_h as f32 - y
                        } else {
                            pane_h
                        };
                        (x0, y, avail_w, h)
                    })
                    .collect()
            }
        }
    }

    pub(crate) fn pane_grid_bounds(
        pane: &Pane,
        rect: (f32, f32, f32, f32),
        font_size: f32,
    ) -> (f32, f32, f32, f32) {
        let (px, py, pw, ph) = rect;
        let (cell_w, cell_h) = render::measure_cell(font_size);
        let grid_w = (pane.terminal.cols as f32 * cell_w).min(pw);
        let grid_h = (pane.terminal.rows as f32 * cell_h).min(ph);
        let gx = px + ((pw - grid_w).max(0.0) * 0.5).floor();
        (gx, py, grid_w, grid_h)
    }

    pub(crate) fn drain_pty(&mut self) {
        const MAX_BYTES_PER_FRAME: usize = 64 * 1024;

        let mut had_output = false;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if let Some((data, has_more)) = pane.pty.read(MAX_BYTES_PER_FRAME) {
                    pane.terminal.process(&data);
                    had_output = true;

                    if has_more {
                        self.proxy.send_event(UserEvent::PtyOutput).ok();
                    }

                    if let Some(title) = pane.terminal.title.take() {
                        pane.title = title;
                    }
                }

                for response in pane.terminal.pending_responses.drain(..) {
                    pane.pty.write(&response);
                }

                if pane.terminal.bell {
                    pane.terminal.bell = false;
                    fire_bell_notification();
                }

                for (title, body) in pane.terminal.pending_notifications.drain(..) {
                    fire_desktop_notification(title, body);
                }
            }
        }

        if had_output {
            self.cursor_visible = true;
            self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }
    }

    pub(crate) fn update_grid_size(&mut self) {
        let gpu = match self.gpu.as_ref() {
            Some(g) => g,
            None => return,
        };

        let screen_w = gpu.width();
        let screen_h = gpu.height();
        let font_size = self.effective_font_size();
        let (cell_w, cell_h) = render::measure_cell(font_size);
        let sb_offset = self.sidebar_offset();

        for tab in &mut self.tabs {
            let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, sb_offset);
            for (i, pane) in tab.panes.iter_mut().enumerate() {
                if i >= rects.len() {
                    break;
                }
                let (_, _, pw, ph) = rects[i];
                let new_cols = (pw / cell_w).floor().max(1.0) as usize;
                let new_rows = (ph / cell_h).floor().max(1.0) as usize;
                if new_cols != pane.terminal.cols || new_rows != pane.terminal.rows {
                    pane.terminal.resize(new_cols, new_rows);
                    pane.pty.resize(new_cols as u16, new_rows as u16);
                }
            }
        }
    }

    pub(crate) fn pixel_to_pane_cell(&self, x: f32, y: f32) -> Option<(usize, usize, usize)> {
        let screen_w = self.gpu.as_ref().map_or(800, |g| g.width());
        let screen_h = self.gpu.as_ref().map_or(600, |g| g.height());
        let tab = &self.tabs[self.active_tab];
        let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, self.sidebar_offset());
        let font_size = self.effective_font_size();
        let (cell_w, cell_h) = render::measure_cell(font_size);

        for (i, &rect) in rects.iter().enumerate() {
            if i >= tab.panes.len() {
                return None;
            }
            let pane = &tab.panes[i];
            let (gx, gy, gw, gh) = Self::pane_grid_bounds(pane, rect, font_size);
            if x >= gx && x < gx + gw && y >= gy && y < gy + gh {
                let row = ((y - gy) / cell_h) as usize;
                let col = ((x - gx) / cell_w) as usize;
                if row >= pane.terminal.rows || col >= pane.terminal.cols {
                    return None;
                }
                return Some((i, row, col));
            }
        }
        None
    }

    pub(crate) fn resize_direction(
        &self,
        screen_w: u32,
        screen_h: u32,
    ) -> Option<winit::window::ResizeDirection> {
        use winit::window::ResizeDirection;
        let (x, y) = self.cursor_pos?;
        let w = screen_w as f32;
        let h = screen_h as f32;

        let left = x < RESIZE_BORDER;
        let right = x > w - RESIZE_BORDER;
        let top = y < RESIZE_BORDER;
        let bottom = y > h - RESIZE_BORDER;

        match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        }
    }

    pub(crate) fn sync_scroll_to_terminal(&mut self) -> f32 {
        if self.tabs.is_empty() {
            return 0.0;
        }
        let cell_h = render::measure_cell(self.effective_font_size()).1;
        let tab = &mut self.tabs[self.active_tab];
        let terminal = &mut tab.panes[tab.active_pane].terminal;
        let max_px = terminal.scrollback.len() as f32 * cell_h;

        self.scroll_current_px = self.scroll_current_px.clamp(0.0, max_px);
        self.scroll_target_px = self.scroll_target_px.clamp(0.0, max_px);

        let line_offset = (self.scroll_current_px / cell_h) as usize;
        let sub_pixel = self.scroll_current_px - (line_offset as f32 * cell_h);

        terminal.scroll_offset = line_offset.min(terminal.scrollback.len());
        sub_pixel
    }

    pub(crate) fn request_redraw(&self) {
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let mut attrs = WindowAttributes::default()
            .with_name("lntrn-terminal", "lntrn-terminal")
            .with_title("Lantern Terminal")
            .with_inner_size(LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0))
            .with_decorations(false)
            .with_transparent(true);

        if let Some(icon_path) = lntrn_theme::lantern_home().map(|h| h.join("icons/lntrn-terminal.png")) {
            if let Ok(img) = image::open(&icon_path) {
                let rgba = img.into_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                if let Ok(icon) = Icon::from_rgba(rgba.into_raw(), w, h) {
                    attrs = attrs.with_window_icon(Some(icon));
                }
            }
        }

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("Failed to create window"),
        );
        self.window = Some(window);

        self.init_gpu();
        self.restore_pinned_tabs();
        self.spawn_tab();
        self.update_grid_size();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let is_cursor_move = matches!(event, WindowEvent::CursorMoved { .. });

        match event {
            WindowEvent::CloseRequested => {
                for tab in &mut self.tabs {
                    for pane in &mut tab.panes {
                        pane.pty.cleanup();
                    }
                }
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(ref mut gpu) = self.gpu {
                    gpu.resize(size.width.max(1), size.height.max(1));
                }
                self.update_grid_size();
                self.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                self.render_frame();
                // Process any menu events that occurred during rendering
                if let Some(action) = self.pending_menu_event.take() {
                    match self.dispatch_chrome_action(action, event_loop, self.gpu.as_ref().map_or(600, |g| g.height())) {
                        EventResult::Exit => {
                            event_loop.exit();
                            return;
                        }
                        _ => {}
                    }
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as f32;
                let y = position.y as f32;
                if matches!(self.handle_cursor_moved(x, y), EventResult::Handled) {
                    return;
                }
            }

            WindowEvent::CursorLeft { .. } => {
                self.cursor_pos = None;
                self.input.on_cursor_left();
                self.request_redraw();
            }

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    match self.handle_left_press(event_loop) {
                        EventResult::Exit | EventResult::Handled => return,
                        EventResult::Continue => {}
                    }
                } else {
                    self.handle_left_release();
                }
            }

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                if state == ElementState::Pressed {
                    self.handle_right_press();
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                match self.handle_keyboard(&event, event_loop) {
                    EventResult::Exit | EventResult::Handled => return,
                    EventResult::Continue => {}
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(delta);
            }

            _ => {}
        }

        if is_cursor_move {
            self.handle_slider_drags();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyOutput => {
                self.drain_pty();

                // Remove dead panes
                for tab in &mut self.tabs {
                    let mut i = 0;
                    while i < tab.panes.len() {
                        if !tab.panes[i].pty.alive {
                            let mut pane = tab.panes.remove(i);
                            pane.pty.cleanup();
                            if tab.active_pane >= tab.panes.len() && !tab.panes.is_empty() {
                                tab.active_pane = tab.panes.len() - 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    if tab.panes.len() <= 1 {
                        tab.split = None;
                    }
                }
                self.tabs.retain(|t| !t.panes.is_empty());
                if self.tabs.is_empty() {
                    event_loop.exit();
                    return;
                }
                if self.active_tab >= self.tabs.len() {
                    self.active_tab = self.tabs.len() - 1;
                }

                self.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.cursor_blink_deadline {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_deadline = now + CURSOR_BLINK_INTERVAL;
            self.request_redraw();
        }

        // Animate smooth scrolling
        if self.scroll_animating {
            let dt = now
                .duration_since(self.last_frame_time)
                .as_secs_f32()
                .min(0.05);
            let speed = 30.0_f32;
            let t = 1.0 - (-speed * dt).exp();
            let diff = self.scroll_target_px - self.scroll_current_px;
            self.scroll_current_px += diff * t;

            if diff.abs() < 0.5 {
                self.scroll_current_px = self.scroll_target_px;
                self.scroll_animating = false;
            }
            self.request_redraw();
        }
        self.last_frame_time = now;

        if self.scroll_animating {
            let next = now + Duration::from_millis(8);
            let deadline = next.min(self.cursor_blink_deadline);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.cursor_blink_deadline));
        }
    }
}

/// Fire a desktop notification when the terminal receives BEL (0x07).
fn fire_bell_notification() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_BELL: AtomicU64 = AtomicU64::new(0);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let prev = LAST_BELL.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < 2000 {
        return;
    }
    LAST_BELL.store(now, Ordering::Relaxed);

    std::thread::spawn(|| {
        let _ = std::process::Command::new("notify-send")
            .args(["Terminal", "Bell"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

/// Fire a desktop notification via notify-send (OSC 99 / Kitty protocol).
fn fire_desktop_notification(title: String, body: String) {
    std::thread::spawn(move || {
        let summary = if title.is_empty() { "Terminal" } else { &title };
        let mut args = vec![summary.to_string()];
        if !body.is_empty() {
            args.push(body);
        }
        let _ = std::process::Command::new("notify-send")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

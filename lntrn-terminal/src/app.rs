use std::sync::Arc;
use std::time::{Duration, Instant};

use lntrn_render::{GpuContext, GpuTexture, Painter, TextRenderer, TexturePass};
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
use crate::git;
use crate::git_sidebar;
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

pub(crate) const SPLIT_DIVIDER: f32 = 3.0;

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
    pub(crate) texture_pass: Option<TexturePass>,
    pub(crate) image_textures: Vec<(u32, u64, GpuTexture)>, // (image_id, version, gpu_texture)

    /// Current output/display scale factor (from winit's fractional scale).
    /// 1.0 = native; 1.4 = "everything 40% bigger". All UI geometry is laid
    /// out in physical pixels = logical-design-px × scale. Updated on
    /// `ScaleFactorChanged` and propagated into the stateful sub-components.
    pub(crate) scale: f32,

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
    /// "Rice mode" — toggled by Super+F11. When true, the title/tab bar is
    /// not drawn and click-handlers ignore that region so the terminal grid
    /// fills from y=0. The sidebar and right-click context menu stay usable
    /// (the sidebar follows its own `visible` flag).
    pub chrome_hidden: bool,

    // Cursor blink
    pub cursor_visible: bool,
    pub(crate) cursor_blink_deadline: Instant,

    // Clipboard
    pub clipboard: Option<clipboard::WaylandClipboard>,

    // Smooth scrolling
    pub(crate) scroll_target_px: f32,
    pub(crate) scroll_current_px: f32,
    pub(crate) scroll_animating: bool,
    /// Wheel scroll speed multiplier — `[terminal] scroll_speed` in
    /// lantern.toml. Per-machine so wheel (PC) and trackpad (laptop) can
    /// be tuned independently. Refreshed by the 500ms config poll.
    pub(crate) scroll_speed: f32,
    /// Fractional wheel detents carried over between events while forwarding
    /// scroll to a mouse-mode TUI (see `forward_wheel_to_tui`).
    pub(crate) wheel_tick_accum: f32,
    pub(crate) last_frame_time: Instant,
    /// Last time we polled `lantern.toml` for theme/accent changes. Cheap
    /// throttle — the read isn't cached so we only check a few times per
    /// second.
    pub(crate) last_theme_poll: Instant,

    // Selection drag
    pub(crate) selecting: bool,

    // Scrollbar drag
    pub(crate) scrollbar_dragging: bool,

    // Pending menu action from overlay rendering
    pub(crate) pending_menu_event: Option<ui_chrome::ClickAction>,

    // Sidebar file browser
    pub sidebar: sidebar::SidebarState,
    /// Time of the last resize-handle press, for double-click-to-reset.
    pub(crate) last_resize_handle_click: Instant,

    // Git sidebar
    pub git_sidebar: git_sidebar::GitSidebarState,
    pub(crate) git_cmd_tx: Option<std::sync::mpsc::Sender<git::worker::GitCmd>>,
    pub(crate) git_event_rx: Option<std::sync::mpsc::Receiver<git::worker::GitEvent>>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let config = LanternConfig::load();
        let theme = Theme::from_config(&config);
        let open_chrome_hidden = config.general.open_chrome_hidden;

        // Restore a previously dragged sidebar width, if any.
        let mut sidebar = sidebar::SidebarState::new();
        if let Some(w) = config.sidebar.width {
            sidebar.apply_saved_width(w);
        }

        // Spawn git worker thread
        let git_proxy = proxy.clone();
        let (git_cmd_tx, git_event_rx) =
            git::worker::spawn(move || { git_proxy.send_event(UserEvent::GitUpdate).ok(); });

        Self {
            config,
            theme,
            proxy,
            window: None,
            gpu: None,
            scale: 1.0,
            painter: None,
            overlay_painter: None,
            text: None,
            overlay_text: None,
            texture_pass: None,
            image_textures: Vec::new(),
            tabs: Vec::new(),
            active_tab: 0,
            modifiers: ModifiersState::empty(),
            cursor_pos: None,
            left_pressed: false,
            chrome: ui_chrome::ChromeState::new(),
            tab_bar: tab_bar::TabBarState::new(),
            input: InteractionContext::new(),
            chrome_hidden: open_chrome_hidden,
            cursor_visible: true,
            cursor_blink_deadline: Instant::now() + CURSOR_BLINK_INTERVAL,
            clipboard: clipboard::WaylandClipboard::new(),
            scroll_target_px: 0.0,
            scroll_speed: lntrn_theme::read_config_f32("terminal", "scroll_speed", 8.0),
            scroll_current_px: 0.0,
            wheel_tick_accum: 0.0,
            scroll_animating: false,
            last_frame_time: Instant::now(),
            last_theme_poll: Instant::now(),
            selecting: false,
            scrollbar_dragging: false,
            pending_menu_event: None,
            sidebar,
            last_resize_handle_click: Instant::now() - std::time::Duration::from_secs(10),
            git_sidebar: git_sidebar::GitSidebarState::new(),
            git_cmd_tx: Some(git_cmd_tx),
            git_event_rx: Some(git_event_rx),
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
        let texture_pass = TexturePass::new(&gpu);

        self.gpu = Some(gpu);
        self.painter = Some(painter);
        self.overlay_painter = Some(overlay_painter);
        self.text = Some(text);
        self.overlay_text = Some(overlay_text);
        self.texture_pass = Some(texture_pass);
    }

    /// Chrome height — tabs now live inside the title bar, so this is just
    /// the title bar height. Returns 0 in rice mode so the terminal grid
    /// fills from y=0.
    pub(crate) fn chrome_height(&self) -> f32 {
        if self.chrome_hidden {
            return 0.0;
        }
        ui_chrome::title_bar_height(&crate::config::WindowMode::current()) * self.scale
    }

    /// Effective font size in PHYSICAL pixels for rendering.
    ///
    /// Two factors combine: (1) a width-responsive shrink based on the
    /// *logical* window width relative to a reference, so narrow windows get
    /// smaller text the same way at every scale; (2) the display scale factor,
    /// so a 1.4× output scale yields 1.4× bigger glyphs on screen. The grid is
    /// drawn into a physical-pixel buffer, so callers want physical px.
    pub(crate) fn effective_font_size(&self) -> f32 {
        const FONT_SCALE_REF_WIDTH: f32 = 1060.0; // logical px
        let base = self.config.font.size; // logical px
        let scale = self.scale;
        let logical_w = self
            .gpu
            .as_ref()
            .map_or(FONT_SCALE_REF_WIDTH, |g| g.width() as f32 / scale);
        let logical_font = if logical_w >= FONT_SCALE_REF_WIDTH {
            base
        } else {
            (base * logical_w / FONT_SCALE_REF_WIDTH).clamp(10.0, base)
        };
        logical_font * scale
    }

    /// Update the active display scale and fan it out to the stateful
    /// sub-components (which lay out their own geometry). Recomputes the grid
    /// so the cell count tracks the new cell size. Cheap; safe to call on
    /// every `ScaleFactorChanged`.
    pub(crate) fn apply_scale(&mut self, scale: f32) {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        if (scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = scale;
        self.tab_bar.scale = scale;
        self.sidebar.set_scale(scale);
        self.git_sidebar.set_scale(scale);
        self.update_grid_size();
        self.request_redraw();
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
        chrome_h: f32,
    ) -> Vec<(f32, f32, f32, f32)> {
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
        let mut any_syncing = false;
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

                if pane.terminal.is_syncing() {
                    any_syncing = true;
                }
            }
        }

        if had_output {
            self.cursor_visible = true;
            self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;

            // Suppress redraw while any pane is mid synchronized-update
            // batch (mode 2026). about_to_wait will fire a redraw once the
            // sync flag clears or the fallback deadline passes.
            if !any_syncing {
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
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
        let chrome_h = self.chrome_height();

        for tab in &mut self.tabs {
            let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, sb_offset, chrome_h);
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
        let rects = Self::pane_rects_for_tab(tab, screen_w, screen_h, self.sidebar_offset(), self.chrome_height());
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
        let max_px = terminal.active_scrollback().len() as f32 * cell_h;

        self.scroll_current_px = self.scroll_current_px.clamp(0.0, max_px);
        self.scroll_target_px = self.scroll_target_px.clamp(0.0, max_px);

        let line_offset = (self.scroll_current_px / cell_h) as usize;
        let raw_sub = self.scroll_current_px - (line_offset as f32 * cell_h);
        // Snap a near-zero residual to 0. Otherwise, when a smooth-scroll
        // animation has just settled, the origin still gets shifted by a
        // fraction of a pixel and `extra_rows=1` is requested — making the
        // renderer pull one row beyond the grid edge while drawing from a
        // shifted origin, which can visually composite with the previous
        // frame on slow displays.
        let sub_pixel = if raw_sub.abs() < 0.5 { 0.0 } else { raw_sub };

        terminal.scroll_offset = line_offset.min(terminal.active_scrollback().len());
        sub_pixel
    }

    pub(crate) fn request_redraw(&self) {
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}

/// Initial window size: 16:9 at `[windows] default_size_pct` of the monitor
/// width (the same knob the compositor uses for default window sizing, set
/// in lntrn-system-settings). Falls back to 1500x1000 when winit hasn't
/// learned about any outputs yet.
fn initial_window_size(event_loop: &ActiveEventLoop) -> LogicalSize<f64> {
    let pct = (lntrn_theme::read_config_f32("windows", "default_size_pct", 60.0)
        / 100.0)
        .clamp(0.2, 1.0) as f64;

    let Some(monitor) = event_loop
        .primary_monitor()
        .or_else(|| event_loop.available_monitors().next())
    else {
        return LogicalSize::new(1500.0, 1000.0);
    };

    let logical = monitor.size().to_logical::<f64>(monitor.scale_factor());
    let mut w = logical.width * pct;
    let mut h = w * 9.0 / 16.0;
    // Portrait/short monitors: keep 16:9 but fit within the same pct of height.
    let max_h = logical.height * pct;
    if h > max_h {
        h = max_h;
        w = h * 16.0 / 9.0;
    }
    LogicalSize::new(w.max(480.0), h.max(320.0))
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let mut attrs = WindowAttributes::default()
            .with_name("lntrn-terminal", "lntrn-terminal")
            .with_title("Lantern Terminal")
            .with_inner_size(initial_window_size(event_loop))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0))
            .with_decorations(false)
            .with_transparent(true);

        if let Some(data) = lntrn_icons::get("lntrn-terminal.png") {
            if let Ok(img) = image::load_from_memory(data) {
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

        // Spawn the Wayland DnD receiver. We pull the raw wl_display
        // ptr from winit's window handle and share it with our own
        // wl_data_device on a side queue (winit 0.30 has no Wayland
        // DnD support of its own — only X11).
        {
            use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
            if let Ok(dh) = window.display_handle() {
                if let RawDisplayHandle::Wayland(wh) = dh.as_raw() {
                    crate::dnd::spawn(wh.display.as_ptr(), self.proxy.clone());
                }
            }
        }

        // Seed the scale from the compositor's preferred fractional scale
        // before any layout happens so the first frame is already correct.
        let initial_scale = window.scale_factor() as f32;
        self.window = Some(window);
        self.scale = if initial_scale.is_finite() && initial_scale > 0.0 {
            initial_scale
        } else {
            1.0
        };
        self.tab_bar.scale = self.scale;
        self.sidebar.set_scale(self.scale);
        self.git_sidebar.set_scale(self.scale);

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

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The compositor changed the output scale (e.g. 1.0 → 1.4).
                // winit keeps the logical window size constant and hands us a
                // larger physical buffer via the following Resized event; here
                // we just adopt the new factor so fonts/chrome grow with it.
                self.apply_scale(scale_factor as f32);
            }

            WindowEvent::RedrawRequested => {
                // If any pane is mid synchronized-update batch (mode 2026)
                // we MUST NOT paint — the grid is in a half-written state
                // and rendering it produces visible tearing (floating
                // letters, duplicated lines). The redraw will be re-armed
                // when drain_pty observes sync clear, or when the fallback
                // deadline expires in about_to_wait.
                let syncing = self.tabs.iter().any(|tab| {
                    tab.panes.iter().any(|pane| pane.terminal.is_syncing())
                });
                if syncing {
                    return;
                }
                self.render_frame();
                // Process any menu events that occurred during rendering
                if let Some(action) = self.pending_menu_event.take() {
                    // Tab-switching actions keep the menu open — its items
                    // (active tab dot, chevrons) must be rebuilt afterwards.
                    let tab_nav = matches!(
                        &action,
                        ui_chrome::ClickAction::PrevTab
                            | ui_chrome::ClickAction::NextTab
                            | ui_chrome::ClickAction::SelectTab(_)
                    );
                    match self.dispatch_chrome_action(action, event_loop, self.gpu.as_ref().map_or(600, |g| g.height())) {
                        EventResult::Exit => {
                            event_loop.exit();
                            return;
                        }
                        _ => {}
                    }
                    if tab_nav {
                        self.refresh_context_menu_items();
                    }
                    // The menu auto-closed mid-frame (its geometry was
                    // already queued) — paint once more so the close and
                    // the action's result actually show.
                    self.request_redraw();
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
            UserEvent::FilesDropped(paths) => {
                if self.tabs.is_empty() {
                    return;
                }
                let tab = &self.tabs[self.active_tab];
                let pane = &tab.panes[tab.active_pane];
                // Paste the multi-path string so the shell (or Claude Code)
                // treats it as a single literal insertion instead of
                // running it.
                let joined = paths
                    .iter()
                    .map(|p| crate::dnd::shell_quote(p))
                    .collect::<Vec<_>>()
                    .join(" ");
                crate::input::write_paste(&joined, &pane.terminal, &pane.pty);
                self.request_redraw();
            }
            UserEvent::GitUpdate => {
                self.poll_git_events();
                self.request_redraw();
            }
            UserEvent::PtyOutput => {
                self.drain_pty();

                if self.sidebar.visible && self.sidebar.mode == sidebar::SidebarMode::Git {
                    self.open_git_repo();
                }

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

        // Live-reload theme from [appearance].theme. Throttled to ~2 Hz —
        // active_variant() is uncached so we don't want to spam stat() at
        // animation-frame rate.
        if now.duration_since(self.last_theme_poll).as_millis() >= 500 {
            self.last_theme_poll = now;
            self.scroll_speed =
                lntrn_theme::read_config_f32("terminal", "scroll_speed", 8.0);
            let new_theme = crate::theme::Theme::current();
            if new_theme.bg != self.theme.bg
                || new_theme.terminal_fg != self.theme.terminal_fg
                || new_theme.terminal_bold != self.theme.terminal_bold
            {
                self.theme = new_theme;
                let fg = self.theme.terminal_fg;
                let bold = self.theme.terminal_bold;
                for tab in &mut self.tabs {
                    for pane in &mut tab.panes {
                        pane.terminal.set_default_colors(
                            fg,
                            crate::terminal::Color8::TRANSPARENT,
                            bold,
                        );
                    }
                }
                self.request_redraw();
            }
        }

        // Synchronized output (mode 2026) deadline check. When a pane has
        // sync_update set but the deadline has expired, force-clear it and
        // redraw — recovers from a missing/lost CSI?2026l. Track the
        // earliest pending deadline so we can wake up exactly when it fires.
        let mut earliest_sync: Option<Instant> = None;
        let mut sync_expired = false;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if let Some(deadline) = pane.terminal.sync_deadline {
                    if now >= deadline {
                        pane.terminal.sync_update = false;
                        pane.terminal.sync_deadline = None;
                        sync_expired = true;
                    } else {
                        earliest_sync = Some(
                            earliest_sync.map_or(deadline, |e| e.min(deadline)),
                        );
                    }
                }
            }
        }
        if sync_expired {
            self.request_redraw();
        }

        // Clear git status messages after timeout
        if self.git_sidebar.check_message_timeout() {
            self.request_redraw();
        }

        // Animate smooth scrolling. Exponential ease-out: ~12/s reaches
        // 95% of the target in ~250ms — a visible glide. Higher values
        // finish within a few frames and read as rigid per-detent hops.
        if self.scroll_animating {
            let dt = now
                .duration_since(self.last_frame_time)
                .as_secs_f32()
                .min(0.05);
            let speed = 12.0_f32;
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

        let mut deadline = self.cursor_blink_deadline;
        if self.scroll_animating {
            deadline = deadline.min(now + Duration::from_millis(8));
        }
        if let Some(s) = earliest_sync {
            deadline = deadline.min(s);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
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

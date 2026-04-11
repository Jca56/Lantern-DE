mod actions;
mod auto_pair;
mod bracket_match;
mod clipboard;
mod editor;
mod find_bar;
mod format;
mod keys;
mod markdown;
mod mouse;
mod render;
mod scrollbar;
mod sidebar;
mod status_bar;
mod syntax;
mod tab_strip;
mod tabs;
mod theme;
mod title_bar;
mod wrap;

use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{CursorIcon, ResizeDirection, Window, WindowAttributes, WindowId};

use lntrn_render::{GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, MenuBar, MenuEvent, ScrollArea};

use clipboard::WaylandClipboard;
use editor::Editor;
use find_bar::FindBar;
use keys::KeyAction;
use sidebar::Sidebar;
use theme::Theme;

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub(crate) const ZONE_CLOSE: u32 = 1;
pub(crate) const ZONE_MAXIMIZE: u32 = 2;
pub(crate) const ZONE_MINIMIZE: u32 = 3;
pub(crate) const ZONE_EDITOR: u32 = 10;
pub(crate) const ZONE_EDITOR_SCROLL_THUMB: u32 = 4000;
pub(crate) const ZONE_EDITOR_SCROLL_TRACK: u32 = 4001;
pub(crate) const ZONE_SIDEBAR_SCROLL_THUMB: u32 = 4002;
pub(crate) const ZONE_SIDEBAR_SCROLL_TRACK: u32 = 4003;

// ── Menu item IDs ───────────────────────────────────────────────────────────

pub(crate) const MENU_NEW: u32 = 100;
pub(crate) const MENU_OPEN: u32 = 101;
pub(crate) const MENU_SAVE: u32 = 102;
pub(crate) const MENU_THEME_PAPER: u32 = 200;
pub(crate) const MENU_THEME_NIGHT: u32 = 201;
pub(crate) const MENU_THEME_DARK: u32 = 202;

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let file_paths: Vec<String> = std::env::args().skip(1).collect();
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut handler = TextHandler::new(file_paths);
    event_loop.run_app(&mut handler).expect("Event loop failed");
}

// ── GPU resources ───────────────────────────────────────────────────────────

struct Gpu {
    ctx: GpuContext,
    painter: Painter,
    text: TextRenderer,
}

// ── Handler ─────────────────────────────────────────────────────────────────

/// Cursor blink interval.
const BLINK_INTERVAL: Duration = Duration::from_millis(530);

pub(crate) struct TextHandler {
    pub(crate) window: Option<Window>,
    pub(crate) gpu: Option<Gpu>,
    pub(crate) tabs: Vec<Editor>,
    pub(crate) active_tab: usize,
    pub(crate) next_tab_id: u64,
    pub(crate) find_bar: FindBar,
    pub(crate) sidebar: Sidebar,
    pub(crate) input: InteractionContext,
    pub(crate) menu_bar: MenuBar,
    pub(crate) clipboard: Option<WaylandClipboard>,
    pub(crate) theme: Theme,
    pub(crate) palette: FoxPalette,
    pub(crate) scale: f32,
    pub(crate) needs_redraw: bool,
    pub(crate) modifiers: ModifiersState,
    pub(crate) cursor_visible: bool,
    pub(crate) cursor_blink_deadline: Instant,
    pub(crate) dragging: bool,
    /// Wall-clock of the last animation tick — used for dt-based easing.
    pub(crate) last_anim_tick: Instant,
}

impl TextHandler {
    fn new(file_paths: Vec<String>) -> Self {
        let mut next_id: u64 = 0;
        let mut tabs: Vec<Editor> = file_paths
            .into_iter()
            .map(|path| {
                let mut e = Editor::new();
                e.tab_id = next_id;
                next_id += 1;
                let _ = e.load_file(std::path::PathBuf::from(path));
                e
            })
            .collect();
        if tabs.is_empty() {
            let mut e = Editor::new();
            e.tab_id = next_id;
            next_id += 1;
            tabs.push(e);
        }
        let theme = theme::load_active();
        let palette = theme.palette();
        Self {
            window: None,
            gpu: None,
            tabs,
            active_tab: 0,
            next_tab_id: next_id,
            find_bar: FindBar::new(),
            sidebar: Sidebar::new(),
            input: InteractionContext::new(),
            menu_bar: MenuBar::new(&palette),
            clipboard: WaylandClipboard::new(),
            theme,
            palette,
            scale: 1.0,
            needs_redraw: true,
            modifiers: ModifiersState::empty(),
            cursor_visible: true,
            cursor_blink_deadline: Instant::now() + BLINK_INTERVAL,
            dragging: false,
            last_anim_tick: Instant::now(),
        }
    }

    /// Borrow the active editor.
    pub(crate) fn editor(&self) -> &Editor {
        &self.tabs[self.active_tab]
    }

    /// Borrow the active editor mutably.
    pub(crate) fn editor_mut(&mut self) -> &mut Editor {
        &mut self.tabs[self.active_tab]
    }

    fn edge_resize_direction(&self) -> Option<ResizeDirection> {
        let (cx, cy) = self.input.cursor()?;
        // Don't intercept resize when the cursor is over a scrollbar thumb
        // or track — the user is trying to drag the scrollbar, not the
        // window edge.
        if let Some(zone_id) = self.input.zone_at(cx, cy) {
            if zone_id == ZONE_EDITOR_SCROLL_THUMB
                || zone_id == ZONE_EDITOR_SCROLL_TRACK
                || zone_id == ZONE_SIDEBAR_SCROLL_THUMB
                || zone_id == ZONE_SIDEBAR_SCROLL_TRACK
            {
                return None;
            }
        }
        let gpu = self.gpu.as_ref()?;
        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let border = 10.0 * self.scale;
        let left = cx < border;
        let right = cx > wf - border;
        let top = cy < border;
        let bottom = cy > hf - border;
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

    fn is_on_title_bar(&self) -> bool {
        self.input
            .cursor()
            .map_or(false, |(_, cy)| cy < title_bar::TITLE_BAR_H * self.scale)
    }

    fn window_size(&self) -> (f32, f32) {
        self.gpu
            .as_ref()
            .map_or((800.0, 600.0), |g| (g.ctx.width() as f32, g.ctx.height() as f32))
    }

    /// Crate-visible alias so sibling modules (mouse.rs) can read window
    /// dimensions without us exposing the gpu field.
    pub(crate) fn window_size_pub(&self) -> (f32, f32) {
        self.window_size()
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.gpu = None;
        self.window = None;
        event_loop.exit();
    }

    /// Set cursor from a click at physical (cx, cy), using real text measurement.
    fn click_to_cursor(&mut self, cx: f32, cy: f32) {
        let s = self.scale;
        let (wf, hf) = self.window_size();
        let font_size = editor::FONT_SIZE * s;
        let sidebar_w = if self.sidebar.visible {
            sidebar::SIDEBAR_W * s
        } else {
            0.0
        };
        let er = render::editor_rect(wf, hf, s, self.find_bar.height(s), sidebar_w);
        let active = self.active_tab;
        let editor = &mut self.tabs[active];

        let (doc_line, row_start, row_end) = editor.wrap_row_at_y(cy, er, s);
        editor.cursor_line = doc_line;

        if let Some(gpu) = &mut self.gpu {
            let base = render::measure_to_offset(
                &mut gpu.text, editor, doc_line, row_start, font_size,
            );
            let col = editor.col_at_x(cx, doc_line, row_start, row_end, er, s, |byte_off| {
                render::measure_to_offset(&mut gpu.text, editor, doc_line, byte_off, font_size) - base
            });
            editor.cursor_col = col;
        }
    }

    fn reset_blink(&mut self) {
        self.cursor_visible = true;
        self.cursor_blink_deadline = Instant::now() + BLINK_INTERVAL;
    }

}

// ── Application handler ──────────────────────────────────────────────────────

impl ApplicationHandler for TextHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_name("lntrn-notepad", "lntrn-notepad")
            .with_title("lntrn-notepad")
            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 700.0))
            .with_decorations(false)
            .with_transparent(true);

        let window = event_loop
            .create_window(attrs)
            .expect("Failed to create window");
        self.scale = window.scale_factor() as f32;

        let size = window.inner_size();
        let gpu_ctx = GpuContext::from_window(&window, size.width, size.height)
            .expect("Failed to create GPU context");

        self.gpu = Some(Gpu {
            painter: Painter::new(&gpu_ctx),
            text: TextRenderer::new(&gpu_ctx),
            ctx: gpu_ctx,
        });
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => self.shutdown(event_loop),

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.needs_redraw = true;
            }

            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.ctx.resize(size.width, size.height);
                }
                self.needs_redraw = true;
            }

            WindowEvent::CursorMoved { position, .. } => {
                let (cx, cy) = (position.x as f32, position.y as f32);
                self.input.on_cursor_moved(cx, cy);

                if mouse::update_scrollbar_drag(self, cx, cy) {
                    // scrollbar drag consumes the move
                } else if self.dragging {
                    self.click_to_cursor(cx, cy);
                    self.reset_blink();
                } else if let Some(dir) = self.edge_resize_direction() {
                    if let Some(w) = &self.window {
                        w.set_cursor(CursorIcon::from(dir));
                    }
                } else if let Some(w) = &self.window {
                    w.set_cursor(CursorIcon::Default);
                }
                self.needs_redraw = true;
            }

            WindowEvent::CursorLeft { .. } => {
                self.input.on_cursor_left();
                self.needs_redraw = true;
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if let mouse::MouseAction::Consumed =
                    mouse::handle_mouse_input(self, event_loop, button, state)
                {
                    self.needs_redraw = true;
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0 * self.scale,
                    MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                let s = self.scale;
                let (wf, hf) = self.window_size();

                // Sidebar gets the wheel when the cursor is over it.
                if self.sidebar.visible {
                    if let Some((cx, _)) = self.input.cursor() {
                        if cx < sidebar::SIDEBAR_W * s {
                            self.sidebar.handle_scroll(scroll, s);
                            self.sidebar.scrollbar.ping();
                            self.needs_redraw = true;
                            return;
                        }
                    }
                }

                let find_h = self.find_bar.height(s);
                let sidebar_w = if self.sidebar.visible {
                    sidebar::SIDEBAR_W * s
                } else {
                    0.0
                };
                let editor_rect = render::editor_rect(wf, hf, s, find_h, sidebar_w);
                let editor = self.editor_mut();
                let total_h = editor.content_height(s);
                // Apply scroll to the TARGET; the animation tick eases the
                // visible offset toward it for a smooth feel.
                ScrollArea::apply_scroll(
                    &mut editor.scroll_target,
                    scroll,
                    total_h,
                    editor_rect.h,
                );
                editor.scrollbar.ping();
                self.needs_redraw = true;
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let mods = self.modifiers;
                if let KeyAction::Consumed = keys::handle_key(self, &event.logical_key, mods) {
                    self.reset_blink();
                    self.needs_redraw = true;
                }
            }

            WindowEvent::RedrawRequested => {
                if !self.needs_redraw {
                    return;
                }
                let cursor_vis = self.cursor_visible;
                let tab_labels = self.tab_labels();
                let active_tab = self.active_tab;
                let scale = self.scale;
                let palette = self.palette;
                let theme = self.theme;
                // Sync sidebar root with the active tab's directory.
                if let Some(parent) = self.tabs[self.active_tab]
                    .file_path
                    .as_ref()
                    .and_then(|p| p.parent())
                {
                    self.sidebar.set_root(parent.to_path_buf());
                }

                // Sync any preview tabs from their source tabs.
                markdown::sync_all_previews(&mut self.tabs);
                // Split borrow: gpu, the active editor (via tabs), find_bar,
                // sidebar, and menu/toolbar state are all separate fields.
                let active = self.active_tab;
                let editor = &mut self.tabs[active];
                let find_bar = &self.find_bar;
                let sidebar = &mut self.sidebar;
                let event = if let Some(gpu) = self.gpu.as_mut() {
                    render::render_frame(
                        gpu,
                        editor,
                        &tab_labels,
                        active_tab,
                        find_bar,
                        sidebar,
                        &mut self.input,
                        &mut self.menu_bar,
                        &palette,
                        theme,
                        scale,
                        cursor_vis,
                    )
                } else {
                    None
                };
                if let Some(evt) = event {
                    match evt {
                        MenuEvent::Action(MENU_NEW) => {
                            self.new_tab();
                            self.menu_bar.close();
                        }
                        MenuEvent::Action(MENU_OPEN) => {
                            self.menu_bar.close();
                            actions::open_file_dialog(self);
                        }
                        MenuEvent::Action(MENU_SAVE) => {
                            self.menu_bar.close();
                            actions::save_file_dialog(self);
                        }
                        MenuEvent::Action(MENU_THEME_PAPER) => {
                            self.menu_bar.close();
                            self.set_theme(Theme::Paper);
                        }
                        MenuEvent::Action(MENU_THEME_NIGHT) => {
                            self.menu_bar.close();
                            self.set_theme(Theme::NightSky);
                        }
                        MenuEvent::Action(MENU_THEME_DARK) => {
                            self.menu_bar.close();
                            self.set_theme(Theme::Dark);
                        }
                        _ => {}
                    }
                    self.needs_redraw = true;
                }
                self.needs_redraw = false;
            }

            _ => {}
        }

        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        // ── Smooth scroll animation tick ──────────────────────────────
        let dt = now.duration_since(self.last_anim_tick).as_secs_f32();
        self.last_anim_tick = now;
        let mut animating = false;
        // Tick every tab so background tabs settle while not visible too.
        for tab in &mut self.tabs {
            let diff = tab.scroll_target - tab.scroll_offset;
            if diff.abs() > 0.5 {
                // Exponential decay: alpha = 1 - e^(-rate * dt). rate ~18
                // gives a snappy ~80ms settle from a typical wheel notch.
                let rate = 18.0;
                let alpha = (1.0 - (-rate * dt).exp()).clamp(0.0, 1.0);
                tab.scroll_offset += diff * alpha;
                animating = true;
            } else {
                tab.scroll_offset = tab.scroll_target;
            }
        }
        if animating {
            self.needs_redraw = true;
        }

        // ── Cursor blink ──────────────────────────────────────────────
        if now >= self.cursor_blink_deadline {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_deadline = now + BLINK_INTERVAL;
            self.needs_redraw = true;
        }

        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }

        // Schedule the next wake-up. While animating we want ~60fps; the
        // blink deadline takes over once everything has settled.
        let next = if animating {
            now + Duration::from_millis(16)
        } else {
            self.cursor_blink_deadline
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }
}

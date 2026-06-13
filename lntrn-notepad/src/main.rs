mod actions;
mod clipboard;
mod context_menu;
mod editor;
mod find_bar;
mod fonts;
mod format;
mod keys;
mod metrics;
mod mouse;
mod page;
mod persist;
mod render;
mod ribbon;
mod scrollbar;
mod status_bar;
mod tab_strip;
mod tabs;
mod theme;
mod title_bar;
mod tokens;
mod toolbar;
mod window_size;
mod wrap;

use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::monitor::MonitorHandle;
use winit::window::{CursorIcon, ResizeDirection, Window, WindowAttributes, WindowId};

use lntrn_render::{GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{ContextMenu, FoxPalette, InteractionContext, MenuBar, MenuEvent, ScrollArea};

use clipboard::WaylandClipboard;
use editor::Editor;
use find_bar::FindBar;
use keys::KeyAction;
use theme::Theme;
use toolbar::FormatToolbar;

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub(crate) const ZONE_CLOSE: u32 = 1;
pub(crate) const ZONE_MAXIMIZE: u32 = 2;
pub(crate) const ZONE_MINIMIZE: u32 = 3;
pub(crate) const ZONE_EDITOR: u32 = 10;
pub(crate) const ZONE_PAGE_HANDLE_L: u32 = 11;
pub(crate) const ZONE_PAGE_HANDLE_R: u32 = 12;
pub(crate) const ZONE_EDITOR_SCROLL_THUMB: u32 = 4000;
pub(crate) const ZONE_EDITOR_SCROLL_TRACK: u32 = 4001;

// ── Menu item IDs ───────────────────────────────────────────────────────────

pub(crate) const MENU_NEW: u32 = 100;
pub(crate) const MENU_OPEN: u32 = 101;
pub(crate) const MENU_SAVE: u32 = 102;
pub(crate) const MENU_SAVE_DOCX: u32 = 103;
pub(crate) const MENU_SAVE_AS: u32 = 104;
pub(crate) const MENU_THEME_PAPER: u32 = 200;
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
    pub(crate) input: InteractionContext,
    pub(crate) menu_bar: MenuBar,
    pub(crate) context_menu: ContextMenu,
    pub(crate) fmt_toolbar: FormatToolbar,
    pub(crate) clipboard: Option<WaylandClipboard>,
    pub(crate) theme: Theme,
    pub(crate) palette: FoxPalette,
    pub(crate) scale: f32,
    pub(crate) needs_redraw: bool,
    pub(crate) modifiers: ModifiersState,
    pub(crate) cursor_visible: bool,
    pub(crate) cursor_blink_deadline: Instant,
    pub(crate) dragging: bool,
    /// Writable-area width as a fraction of the available editor width.
    /// Controlled by dragging the page margins; persisted to disk.
    pub(crate) page_width_frac: f32,
    /// True while a page-margin handle is being dragged.
    pub(crate) page_drag: bool,
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
        let cfg = theme::load();
        let theme = cfg.theme;
        let palette = theme.palette();
        Self {
            window: None,
            gpu: None,
            tabs,
            active_tab: 0,
            next_tab_id: next_id,
            find_bar: FindBar::new(),
            input: InteractionContext::new(),
            menu_bar: MenuBar::new(&palette),
            context_menu: ContextMenu::new(context_menu::context_menu_style(&palette)),
            fmt_toolbar: FormatToolbar::new(),
            clipboard: WaylandClipboard::new(),
            theme,
            palette,
            scale: 1.0,
            needs_redraw: true,
            modifiers: ModifiersState::empty(),
            cursor_visible: true,
            cursor_blink_deadline: Instant::now() + BLINK_INTERVAL,
            dragging: false,
            page_width_frac: cfg.page_width,
            page_drag: false,
            last_anim_tick: Instant::now(),
        }
    }

    /// Persist current view settings (theme + page width) to disk.
    pub(crate) fn save_config(&self) {
        theme::save(&theme::NotepadConfig {
            theme: self.theme,
            page_width: self.page_width_frac,
        });
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
            if zone_id == ZONE_EDITOR_SCROLL_THUMB || zone_id == ZONE_EDITOR_SCROLL_TRACK {
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
        let pad = editor::PAD * s;
        let er = render::editor_rect(wf, hf, s, self.find_bar.height(s));
        let active = self.active_tab;
        let editor = &mut self.tabs[active];

        let (doc_line, row_start, row_end) = editor.wrap_row_at_y(cy, er, s);
        editor.cursor_line = doc_line;

        if let Some(gpu) = &mut self.gpu {
            // Compute content_x matching render.rs page layout
            let (page_x, page_w) = page::geometry(er, self.page_width_frac, s);
            let content_x = page_x + pad;
            let content_max_w = (page_w - pad * 2.0).max(10.0);

            // Alignment + indent + bullet offset for this row. Must match
            // render::row_x_offset so clicks land on the right glyph.
            let para = editor.formats.get(doc_line).para;
            let wraps = &editor.wrap_rows[doc_line];
            let row_idx = wraps.iter().position(|&st| st == row_start).unwrap_or(0);
            let bullet_off = if para.bullet { editor::BULLET_INDENT * s } else { 0.0 };
            let avail = (content_max_w - bullet_off).max(10.0);
            let row_w = render::measure_range(
                &mut gpu.text, editor, doc_line, row_start, row_end, font_size,
            );
            let align_off = render::alignment_offset(para.alignment, avail, row_w);
            let indent_off = if row_idx == 0 { para.first_indent * s } else { 0.0 };
            let effective_x = content_x + bullet_off + align_off + indent_off;

            let base = render::measure_to_offset(
                &mut gpu.text, editor, doc_line, row_start, font_size,
            );
            let col = editor.col_at_x(cx, doc_line, row_start, row_end, effective_x, |byte_off| {
                render::measure_to_offset(&mut gpu.text, editor, doc_line, byte_off, font_size) - base
            });
            editor.cursor_col = col;
        }
    }

    fn reset_blink(&mut self) {
        self.cursor_visible = true;
        self.cursor_blink_deadline = Instant::now() + BLINK_INTERVAL;
    }

    /// Open the right-click context menu at physical (x, y). Restyles from the
    /// live theme/accent and bakes in the current scale so it renders crisp at
    /// any output scale, then closes the title-bar dropdown so only one menu is
    /// ever up.
    pub(crate) fn open_context_menu(&mut self, x: f32, y: f32) {
        self.menu_bar.close();
        let style = context_menu::context_menu_style(&self.palette).with_scale(self.scale);
        self.context_menu.set_style(style);
        let has_sel = self.editor().has_selection();
        self.context_menu.open(x, y, context_menu::build_items(has_sel));
        self.needs_redraw = true;
    }

}

// ── Application handler ──────────────────────────────────────────────────────

/// The largest connected output by pixel area. winit gives us no current /
/// primary monitor on Lantern and lists outputs smallest-first, so this is how
/// we find the display the notepad should size itself against — the desktop's
/// 4K primary, or the laptop's single panel.
fn largest_monitor(event_loop: &ActiveEventLoop) -> Option<MonitorHandle> {
    event_loop
        .available_monitors()
        .max_by_key(|m| m.size().width as u64 * m.size().height as u64)
}

impl ApplicationHandler for TextHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // A portrait window sized as a share of the active output — taller than
        // wide, scale-agnostic on both the laptop and the 4K desktop. On this
        // compositor winit reports neither a current nor a primary monitor and
        // lists outputs smallest-first, so we size to the LARGEST output (the
        // roomy desktop primary / the laptop's only panel) instead of guessing.
        // The target is in PHYSICAL pixels (monitor physical size is accurate);
        // we convert to the logical request via the window's REAL scale below.
        let monitor = largest_monitor(event_loop);
        let (target_w, target_h) = window_size::portrait_physical(monitor.as_ref());
        let attrs = WindowAttributes::default()
            .with_name("lntrn-notepad", "lntrn-notepad")
            .with_title("lntrn-notepad")
            // First guess for the attribute (scale unknown pre-creation).
            .with_inner_size(winit::dpi::LogicalSize::new(target_w, target_h))
            .with_decorations(false)
            .with_transparent(true);

        let window = event_loop
            .create_window(attrs)
            .expect("Failed to create window");
        self.scale = window.scale_factor() as f32;
        // Lantern suggests `[windows] default_size_pct` in the initial configure
        // and winit adopts it, overriding the attribute above — re-assert our
        // deliberate portrait size. winit turns a LogicalSize into a buffer of
        // `logical × real_scale`, so divide the physical target by the window's
        // true fractional scale (the MonitorHandle's is rounded to 2.0 on the
        // 4K and would land the window ~30% too small). One request, no loop.
        let real_scale = (window.scale_factor()).max(1.0);
        let _ = window.request_inner_size(winit::dpi::LogicalSize::new(
            target_w / real_scale,
            target_h / real_scale,
        ));

        let size = window.inner_size();
        let gpu_ctx = GpuContext::from_window(&window, size.width, size.height)
            .expect("Failed to create GPU context");

        let mut text = TextRenderer::new(&gpu_ctx);
        // Load the bundled Google Fonts so families resolve by name.
        fonts::load_bundled(&mut text);
        self.gpu = Some(Gpu {
            painter: Painter::new(&gpu_ctx),
            text,
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

                let on_handle = matches!(
                    self.input.zone_at(cx, cy),
                    Some(ZONE_PAGE_HANDLE_L | ZONE_PAGE_HANDLE_R)
                );

                if mouse::update_scrollbar_drag(self, cx, cy) {
                    // scrollbar drag consumes the move
                } else if self.page_drag {
                    mouse::set_page_width_from_cursor(self, cx);
                    if let Some(w) = &self.window {
                        w.set_cursor(CursorIcon::EwResize);
                    }
                } else if self.dragging {
                    self.click_to_cursor(cx, cy);
                    self.reset_blink();
                } else if on_handle {
                    if let Some(w) = &self.window {
                        w.set_cursor(CursorIcon::EwResize);
                    }
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
                // A scroll dismisses the right-click menu rather than leaving it
                // floating over content that slides out from under it.
                if self.context_menu.is_open() {
                    self.context_menu.close();
                    self.needs_redraw = true;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0 * self.scale,
                    MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                let s = self.scale;
                let (wf, hf) = self.window_size();
                let find_h = self.find_bar.height(s);
                let editor_rect = render::editor_rect(wf, hf, s, find_h);
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
                let page_width_frac = self.page_width_frac;
                // Split borrow: gpu, the active editor (via tabs), find_bar,
                // and menu/toolbar state are all separate fields.
                let active = self.active_tab;
                let editor = &mut self.tabs[active];
                let find_bar = &self.find_bar;
                let (event, ctx_event) = if let Some(gpu) = self.gpu.as_mut() {
                    render::render_frame(
                        gpu,
                        editor,
                        &tab_labels,
                        active_tab,
                        find_bar,
                        &mut self.input,
                        &mut self.menu_bar,
                        &mut self.context_menu,
                        &mut self.fmt_toolbar,
                        &palette,
                        theme,
                        scale,
                        page_width_frac,
                        cursor_vis,
                    )
                } else {
                    (None, None)
                };
                if let Some(evt) = ctx_event {
                    if context_menu::handle_event(self, &evt) {
                        self.context_menu.close();
                    }
                    self.needs_redraw = true;
                }
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
                        MenuEvent::Action(MENU_SAVE_AS) => {
                            self.menu_bar.close();
                            actions::save_as_dialog(self);
                        }
                        MenuEvent::Action(MENU_SAVE_DOCX) => {
                            self.menu_bar.close();
                            actions::export_docx_dialog(self);
                        }
                        MenuEvent::Action(MENU_THEME_PAPER) => {
                            self.menu_bar.close();
                            self.set_theme(Theme::Paper);
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

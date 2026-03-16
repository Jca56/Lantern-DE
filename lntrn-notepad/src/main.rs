mod clipboard;
mod editor;
mod render;

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowAttributes, WindowId};

use lntrn_render::{GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, MenuBar, MenuEvent, ScrollArea};

use clipboard::WaylandClipboard;
use editor::Editor;

// ── Hit zone IDs ────────────────────────────────────────────────────────────

const ZONE_CLOSE: u32 = 1;
const ZONE_MAXIMIZE: u32 = 2;
const ZONE_MINIMIZE: u32 = 3;
const ZONE_EDITOR: u32 = 10;

// ── Menu item IDs ───────────────────────────────────────────────────────────

const MENU_NEW: u32 = 100;
const MENU_OPEN: u32 = 101;
const MENU_SAVE: u32 = 102;

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let file_path = std::env::args().nth(1);
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut handler = TextHandler::new(file_path);
    event_loop.run_app(&mut handler).expect("Event loop failed");
}

// ── GPU resources ───────────────────────────────────────────────────────────

struct Gpu {
    ctx: GpuContext,
    painter: Painter,
    text: TextRenderer,
}

// ── Handler ─────────────────────────────────────────────────────────────────

/// Cursor blink: 530ms on, 530ms off.
const BLINK_INTERVAL_MS: u128 = 530;

struct TextHandler {
    window: Option<Window>,
    gpu: Option<Gpu>,
    editor: Editor,
    input: InteractionContext,
    menu_bar: MenuBar,
    clipboard: Option<WaylandClipboard>,
    palette: FoxPalette,
    scale: f32,
    needs_redraw: bool,
    modifiers: ModifiersState,
    cursor_blink_time: Instant,
    dragging: bool,
}

impl TextHandler {
    fn new(file_path: Option<String>) -> Self {
        let mut editor = Editor::new();
        if let Some(path) = file_path {
            let _ = editor.load_file(std::path::PathBuf::from(path));
        }
        let palette = FoxPalette::dark();
        Self {
            window: None,
            gpu: None,
            editor,
            input: InteractionContext::new(),
            menu_bar: MenuBar::new(&palette),
            clipboard: WaylandClipboard::new(),
            palette,
            scale: 1.0,
            needs_redraw: true,
            modifiers: ModifiersState::empty(),
            cursor_blink_time: Instant::now(),
            dragging: false,
        }
    }

    fn edge_resize_direction(&self) -> Option<ResizeDirection> {
        let (cx, cy) = self.input.cursor()?;
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
            .map_or(false, |(_, cy)| cy < render::TITLE_BAR_H * self.scale)
    }

    fn window_size(&self) -> (f32, f32) {
        self.gpu
            .as_ref()
            .map_or((800.0, 600.0), |g| (g.ctx.width() as f32, g.ctx.height() as f32))
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.gpu = None;
        self.window = None;
        event_loop.exit();
    }

    fn reset_blink(&mut self) {
        self.cursor_blink_time = Instant::now();
    }

    fn cursor_visible(&self) -> bool {
        let elapsed = self.cursor_blink_time.elapsed().as_millis();
        (elapsed / BLINK_INTERVAL_MS) % 2 == 0
    }

    fn open_file_dialog(&mut self) {
        let output = std::process::Command::new("zenity")
            .args(["--file-selection", "--title=Open File"])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    let _ = self.editor.load_file(std::path::PathBuf::from(path));
                }
            }
        }
    }

    fn save_file_dialog(&mut self) {
        if self.editor.file_path.is_some() {
            let _ = self.editor.save_file();
            return;
        }
        let output = std::process::Command::new("zenity")
            .args(["--file-selection", "--save", "--title=Save File", "--confirm-overwrite"])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    self.editor.file_path = Some(std::path::PathBuf::from(path));
                    let _ = self.editor.save_file();
                }
            }
        }
    }

    fn do_copy(&mut self) {
        if let Some(text) = self.editor.selected_text() {
            if let Some(cb) = &self.clipboard {
                cb.set_text(&text);
            }
        }
    }

    fn do_cut(&mut self) {
        if let Some(text) = self.editor.selected_text() {
            if let Some(cb) = &self.clipboard {
                cb.set_text(&text);
            }
            self.editor.delete_selection();
        }
    }

    fn do_paste(&mut self) {
        if let Some(cb) = &self.clipboard {
            if let Some(text) = cb.get_text() {
                self.editor.insert_str(&text);
            }
        }
    }
}

// ── Application handler ──────────────────────────────────────────────────────

impl ApplicationHandler for TextHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("lntrn-text")
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

                if self.dragging {
                    let s = self.scale;
                    let (wf, hf) = self.window_size();
                    self.editor.click_to_position(cx, cy, wf, hf, s);
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
                match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => {
                        if let Some(dir) = self.edge_resize_direction() {
                            if let Some(w) = &self.window {
                                let _ = w.drag_resize_window(dir);
                            }
                            return;
                        }

                        // Let menu bar handle clicks first
                        let menus = render::file_menu_items();
                        if self.menu_bar.on_click(&mut self.input, &menus, self.scale) {
                            self.needs_redraw = true;
                            return;
                        }

                        if let Some(zone_id) = self.input.on_left_pressed() {
                            match zone_id {
                                ZONE_CLOSE => {
                                    self.shutdown(event_loop);
                                    return;
                                }
                                ZONE_MINIMIZE => {
                                    if let Some(w) = &self.window {
                                        w.set_minimized(true);
                                    }
                                }
                                ZONE_MAXIMIZE => {
                                    if let Some(w) = &self.window {
                                        w.set_maximized(!w.is_maximized());
                                    }
                                }
                                ZONE_EDITOR => {
                                    self.editor.clear_selection();
                                    if let Some((cx, cy)) = self.input.cursor() {
                                        let s = self.scale;
                                        let (wf, hf) = self.window_size();
                                        self.editor.click_to_position(
                                            cx, cy, wf, hf, s,
                                        );
                                        self.editor.begin_selection();
                                        self.dragging = true;
                                    }
                                }
                                _ => {}
                            }
                        } else if self.is_on_title_bar() {
                            if let Some(w) = &self.window {
                                let _ = w.drag_window();
                            }
                            return;
                        }
                        self.needs_redraw = true;
                    }
                    (MouseButton::Left, ElementState::Released) => {
                        self.input.on_left_released();
                        if self.dragging {
                            self.dragging = false;
                            // If anchor == cursor, it was just a click — clear selection
                            if !self.editor.has_selection() {
                                self.editor.clear_selection();
                            }
                        }
                        self.needs_redraw = true;
                    }
                    _ => {}
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0 * self.scale,
                    MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                let s = self.scale;
                let (wf, hf) = self.window_size();
                let editor_rect = render::editor_rect(wf, hf, s);
                let total_h = self.editor.content_height(s);
                ScrollArea::apply_scroll(
                    &mut self.editor.scroll_offset,
                    scroll,
                    total_h,
                    editor_rect.h,
                );
                self.needs_redraw = true;
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }

                // Close menu on Escape
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    if self.menu_bar.is_open() {
                        self.menu_bar.close();
                        self.needs_redraw = true;
                        return;
                    }
                }

                let ctrl = self.modifiers.contains(ModifiersState::CONTROL);
                let shift = self.modifiers.contains(ModifiersState::SHIFT);

                match &event.logical_key {
                    Key::Character(s) if ctrl && shift => {
                        match s.as_str() {
                            "Z" | "z" => self.editor.redo(),
                            _ => {}
                        }
                    }
                    Key::Character(s) if ctrl => {
                        match s.as_str() {
                            "s" => self.save_file_dialog(),
                            "o" => self.open_file_dialog(),
                            "n" => self.editor = Editor::new(),
                            "a" => self.editor.select_all(),
                            "c" => self.do_copy(),
                            "x" => self.do_cut(),
                            "v" => self.do_paste(),
                            "z" => self.editor.undo(),
                            _ => {}
                        }
                    }
                    Key::Named(NamedKey::Enter) => self.editor.insert_char('\n'),
                    Key::Named(NamedKey::Backspace) => self.editor.backspace(),
                    Key::Named(NamedKey::Delete) => self.editor.delete(),
                    Key::Named(NamedKey::ArrowLeft) => self.editor.move_left(shift),
                    Key::Named(NamedKey::ArrowRight) => self.editor.move_right(shift),
                    Key::Named(NamedKey::ArrowUp) => self.editor.move_up(shift),
                    Key::Named(NamedKey::ArrowDown) => self.editor.move_down(shift),
                    Key::Named(NamedKey::Home) => self.editor.home(shift),
                    Key::Named(NamedKey::End) => self.editor.end(shift),
                    Key::Named(NamedKey::Space) => self.editor.insert_char(' '),
                    Key::Named(NamedKey::Tab) => self.editor.insert_str("    "),
                    Key::Character(s) if !ctrl => {
                        for ch in s.chars() {
                            self.editor.insert_char(ch);
                        }
                    }
                    _ => {}
                }
                self.reset_blink();
                self.needs_redraw = true;
            }

            WindowEvent::RedrawRequested => {
                let cursor_vis = self.cursor_visible();
                if let Some(gpu) = &mut self.gpu {
                    let event = render::render_frame(
                        gpu,
                        &mut self.editor,
                        &mut self.input,
                        &mut self.menu_bar,
                        &self.palette,
                        self.scale,
                        cursor_vis,
                    );
                    if let Some(evt) = event {
                        match evt {
                            MenuEvent::Action(MENU_NEW) => {
                                self.editor = Editor::new();
                                self.menu_bar.close();
                            }
                            MenuEvent::Action(MENU_OPEN) => {
                                self.menu_bar.close();
                                self.open_file_dialog();
                            }
                            MenuEvent::Action(MENU_SAVE) => {
                                self.menu_bar.close();
                                self.save_file_dialog();
                            }
                            _ => {}
                        }
                        self.needs_redraw = true;
                    }
                }
                self.needs_redraw = false;
            }

            _ => {}
        }

        // Always request redraws for cursor blink
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

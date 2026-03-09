use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use lntrn_render::{Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer, TexturePass, TextureDraw};
use lntrn_ui::gpu::FoxPalette;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::{CursorIcon, Fullscreen, Window, WindowAttributes, WindowId},
};

mod capture;
mod clipboard;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let delay = args
        .iter()
        .position(|a| a == "--delay" || a == "-d")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let output_path = args
        .iter()
        .position(|a| a == "--output" || a == "-o")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    if delay > 0 {
        eprintln!("Waiting {} seconds...", delay);
        std::thread::sleep(std::time::Duration::from_secs(delay));
    }

    eprintln!("Capturing screen...");
    let cap = capture::capture_screen()?;
    eprintln!("Captured {}x{}", cap.width, cap.height);

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = ScreenshotApp {
        state: None,
        capture: Some(cap),
        output_path,
        clipboard_data: None,
    };
    event_loop.run_app(&mut app)?;

    // After the window closes, serve clipboard on the main thread (blocking).
    // The process stays alive until another client takes the clipboard.
    if let Some(png_data) = app.clipboard_data.take() {
        eprintln!("Serving clipboard...");
        if let Err(e) = clipboard::serve_clipboard(png_data) {
            eprintln!("Clipboard error: {e}");
        }
    }

    Ok(())
}

struct ScreenshotApp {
    state: Option<AppState>,
    capture: Option<capture::ScreenCapture>,
    output_path: Option<PathBuf>,
    clipboard_data: Option<Arc<Vec<u8>>>,
}

struct AppState {
    window: Window,
    window_id: WindowId,
    gpu: GpuContext,
    painter: Painter,
    text: TextRenderer,
    tex_pass: TexturePass,
    screenshot_tex: lntrn_render::GpuTexture,
    palette: FoxPalette,

    // Original capture data for export
    capture_data: Vec<u8>,
    capture_width: u32,
    capture_height: u32,

    // Interaction
    selection: Option<Selection>,
    drag_mode: DragMode,
    cursor_pos: (f32, f32),
    modifiers: ModifiersState,
    output_path: Option<PathBuf>,
}

#[derive(Clone)]
struct Selection {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

const HANDLE_SIZE: f32 = 16.0;
const HANDLE_HIT: f32 = 22.0; // hit area slightly larger than visual

/// Which part of the selection is being dragged.
#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    None,
    /// Drawing a new selection from scratch.
    New { start_x: f32, start_y: f32 },
    /// Dragging a corner/edge handle to resize.
    Handle {
        edge: HandleEdge,
        /// Original selection at drag start.
        orig: (f32, f32, f32, f32),
    },
    /// Dragging the selection body to move it.
    Move { offset_x: f32, offset_y: f32 },
}

#[derive(Clone, Copy, PartialEq)]
enum HandleEdge {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Selection {
    fn normalized(&self) -> (f32, f32, f32, f32) {
        let (x, w) = if self.w < 0.0 {
            (self.x + self.w, -self.w)
        } else {
            (self.x, self.w)
        };
        let (y, h) = if self.h < 0.0 {
            (self.y + self.h, -self.h)
        } else {
            (self.y, self.h)
        };
        (x, y, w, h)
    }

    fn from_normalized(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Check which handle (if any) the cursor is over.
    fn hit_handle(&self, cx: f32, cy: f32) -> Option<HandleEdge> {
        let (x, y, w, h) = self.normalized();
        let half = HANDLE_HIT;

        let on_left = (cx - x).abs() < half;
        let on_right = (cx - (x + w)).abs() < half;
        let on_top = (cy - y).abs() < half;
        let on_bottom = (cy - (y + h)).abs() < half;
        let in_x = cx >= x - half && cx <= x + w + half;
        let in_y = cy >= y - half && cy <= y + h + half;

        if on_top && on_left { return Some(HandleEdge::TopLeft); }
        if on_top && on_right { return Some(HandleEdge::TopRight); }
        if on_bottom && on_left { return Some(HandleEdge::BottomLeft); }
        if on_bottom && on_right { return Some(HandleEdge::BottomRight); }
        if on_top && in_x { return Some(HandleEdge::Top); }
        if on_bottom && in_x { return Some(HandleEdge::Bottom); }
        if on_left && in_y { return Some(HandleEdge::Left); }
        if on_right && in_y { return Some(HandleEdge::Right); }
        None
    }

    /// Check if cursor is inside the selection body (not on a handle).
    fn contains(&self, cx: f32, cy: f32) -> bool {
        let (x, y, w, h) = self.normalized();
        cx >= x && cx <= x + w && cy >= y && cy <= y + h
    }
}

impl AppState {
    fn render(&mut self) -> Result<(), SurfaceError> {
        let sw = self.gpu.width() as f32;
        let sh = self.gpu.height() as f32;
        let dim = Color::from_rgba8(0, 0, 0, 140);
        let accent = self.palette.accent;
        let handle_color = Color::WHITE;

        self.painter.clear();

        if let Some(ref sel) = self.selection {
            let (sx, sy, sel_w, sel_h) = sel.normalized();

            // Four dim strips around selection
            self.painter.rect_filled(Rect::new(0.0, 0.0, sw, sy), 0.0, dim);
            self.painter.rect_filled(Rect::new(0.0, sy + sel_h, sw, sh - sy - sel_h), 0.0, dim);
            self.painter.rect_filled(Rect::new(0.0, sy, sx, sel_h), 0.0, dim);
            self.painter.rect_filled(Rect::new(sx + sel_w, sy, sw - sx - sel_w, sel_h), 0.0, dim);

            // Selection border
            self.painter.rect_stroke(Rect::new(sx, sy, sel_w, sel_h), 0.0, 2.0, accent);

            // Drag handles: 4 corners + 4 edge midpoints
            let hs = HANDLE_SIZE;
            let half = hs / 2.0;
            let handles = [
                (sx - half, sy - half),                             // TL
                (sx + sel_w / 2.0 - half, sy - half),              // T
                (sx + sel_w - half, sy - half),                     // TR
                (sx + sel_w - half, sy + sel_h / 2.0 - half),      // R
                (sx + sel_w - half, sy + sel_h - half),             // BR
                (sx + sel_w / 2.0 - half, sy + sel_h - half),      // B
                (sx - half, sy + sel_h - half),                     // BL
                (sx - half, sy + sel_h / 2.0 - half),              // L
            ];
            for (hx, hy) in handles {
                self.painter.rect_filled(Rect::new(hx, hy, hs, hs), 2.0, handle_color);
                self.painter.rect_stroke(Rect::new(hx, hy, hs, hs), 2.0, 1.0, accent);
            }

            // Size label
            let label = format!("{}x{}", sel_w as u32, sel_h as u32);
            let label_y = if sy > 28.0 { sy - 24.0 } else { sy + sel_h + 6.0 };
            self.painter.rect_filled(
                Rect::new(sx, label_y - 2.0, 90.0, 20.0),
                4.0,
                Color::from_rgba8(0, 0, 0, 180),
            );
            self.text.queue(
                &label, 14.0, sx + 4.0, label_y, self.palette.text, 200.0,
                sw as u32, sh as u32,
            );
        } else {
            // No selection: dim the entire screen
            self.painter.rect_filled(Rect::new(0.0, 0.0, sw, sh), 0.0, dim);
        }

        // Bottom hint bar
        let hint = if self.selection.is_some() {
            "Enter = save + copy  \u{00b7}  Ctrl+C = copy  \u{00b7}  Ctrl+S = save  \u{00b7}  Esc = cancel"
        } else {
            "Drag to select  \u{00b7}  Enter = full screen  \u{00b7}  Esc = cancel"
        };
        let hint_w = 860.0;
        self.painter.rect_filled(
            Rect::new(sw / 2.0 - hint_w / 2.0 - 16.0, sh - 56.0, hint_w + 32.0, 44.0),
            10.0,
            Color::from_rgba8(0, 0, 0, 200),
        );
        self.text.queue(
            hint, 22.0,
            sw / 2.0 - hint_w / 2.0, sh - 48.0,
            self.palette.text, hint_w,
            sw as u32, sh as u32,
        );

        let mut frame = self.gpu.begin_frame("screenshot")?;
        let view = frame.view().clone();

        // 1. Clear to black
        {
            let encoder = frame.encoder_mut();
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }

        // 2. Screenshot texture
        let tex_draw = TextureDraw::new(&self.screenshot_tex, 0.0, 0.0, sw, sh);
        self.tex_pass.render_pass(&self.gpu, frame.encoder_mut(), &view, &[tex_draw], None);

        // 3. Overlay shapes
        self.painter.render_pass_overlay(&self.gpu, frame.encoder_mut(), &view);

        // 4. Text
        self.text.render_queued(&self.gpu, frame.encoder_mut(), &view);

        frame.submit(&self.gpu.queue);
        Ok(())
    }

    fn on_cursor_moved(&mut self, cx: f32, cy: f32) {
        self.cursor_pos = (cx, cy);

        match self.drag_mode {
            DragMode::New { start_x, start_y } => {
                self.selection = Some(Selection {
                    x: start_x,
                    y: start_y,
                    w: cx - start_x,
                    h: cy - start_y,
                });
                self.window.request_redraw();
            }
            DragMode::Handle { edge, ref orig } => {
                let (ox, oy, ow, oh) = *orig;
                let (nx, ny, nw, nh) = match edge {
                    HandleEdge::TopLeft => (cx, cy, ox + ow - cx, oy + oh - cy),
                    HandleEdge::Top => (ox, cy, ow, oy + oh - cy),
                    HandleEdge::TopRight => (ox, cy, cx - ox, oy + oh - cy),
                    HandleEdge::Right => (ox, oy, cx - ox, oh),
                    HandleEdge::BottomRight => (ox, oy, cx - ox, cy - oy),
                    HandleEdge::Bottom => (ox, oy, ow, cy - oy),
                    HandleEdge::BottomLeft => (cx, oy, ox + ow - cx, cy - oy),
                    HandleEdge::Left => (cx, oy, ox + ow - cx, oh),
                };
                self.selection = Some(Selection::from_normalized(nx, ny, nw, nh));
                self.window.request_redraw();
            }
            DragMode::Move { offset_x, offset_y } => {
                if let Some(ref sel) = self.selection {
                    let (_, _, w, h) = sel.normalized();
                    self.selection = Some(Selection::from_normalized(
                        cx - offset_x, cy - offset_y, w, h,
                    ));
                    self.window.request_redraw();
                }
            }
            DragMode::None => {
                // Update cursor icon based on what we're hovering
                let icon = if let Some(ref sel) = self.selection {
                    if let Some(edge) = sel.hit_handle(cx, cy) {
                        match edge {
                            HandleEdge::TopLeft | HandleEdge::BottomRight => CursorIcon::NwseResize,
                            HandleEdge::TopRight | HandleEdge::BottomLeft => CursorIcon::NeswResize,
                            HandleEdge::Top | HandleEdge::Bottom => CursorIcon::NsResize,
                            HandleEdge::Left | HandleEdge::Right => CursorIcon::EwResize,
                        }
                    } else if sel.contains(cx, cy) {
                        CursorIcon::Move
                    } else {
                        CursorIcon::Crosshair
                    }
                } else {
                    CursorIcon::Crosshair
                };
                self.window.set_cursor(icon);
            }
        }
    }

    fn on_left_pressed(&mut self, cx: f32, cy: f32) {
        // Check if clicking on a handle of existing selection
        if let Some(ref sel) = self.selection {
            if let Some(edge) = sel.hit_handle(cx, cy) {
                let orig = sel.normalized();
                self.drag_mode = DragMode::Handle { edge, orig };
                return;
            }
            if sel.contains(cx, cy) {
                let (sx, sy, _, _) = sel.normalized();
                self.drag_mode = DragMode::Move {
                    offset_x: cx - sx,
                    offset_y: cy - sy,
                };
                return;
            }
        }
        // Start new selection
        self.drag_mode = DragMode::New { start_x: cx, start_y: cy };
        self.selection = None;
    }

    fn on_left_released(&mut self) {
        // Normalize selection on release so handles work correctly
        if let Some(ref sel) = self.selection {
            let (x, y, w, h) = sel.normalized();
            if w > 2.0 && h > 2.0 {
                self.selection = Some(Selection::from_normalized(x, y, w, h));
            } else {
                self.selection = None; // too small, discard
            }
        }
        self.drag_mode = DragMode::None;
    }

    fn export(&self, copy: bool, save: bool) -> Option<Arc<Vec<u8>>> {
        let sel = self.selection.as_ref();
        let (crop_x, crop_y, crop_w, crop_h) = if let Some(sel) = sel {
            let (sx, sy, sw, sh) = sel.normalized();
            let screen_w = self.gpu.width() as f32;
            let screen_h = self.gpu.height() as f32;
            let scale_x = self.capture_width as f32 / screen_w;
            let scale_y = self.capture_height as f32 / screen_h;
            (
                (sx * scale_x) as u32,
                (sy * scale_y) as u32,
                (sw * scale_x).max(1.0) as u32,
                (sh * scale_y).max(1.0) as u32,
            )
        } else {
            (0, 0, self.capture_width, self.capture_height)
        };

        let img = match image::RgbaImage::from_raw(
            self.capture_width,
            self.capture_height,
            self.capture_data.clone(),
        ) {
            Some(img) => img,
            None => {
                eprintln!("Failed to create image from capture data");
                return None;
            }
        };
        let cropped =
            image::imageops::crop_imm(&img, crop_x, crop_y, crop_w, crop_h).to_image();

        if save {
            let path = self
                .output_path
                .clone()
                .unwrap_or_else(default_output_path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match cropped.save(&path) {
                Ok(()) => eprintln!("Saved to {}", path.display()),
                Err(e) => eprintln!("Failed to save: {e}"),
            }
        }

        if copy {
            use image::ImageEncoder;
            let mut png_data = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
            if let Err(e) = encoder.write_image(
                cropped.as_raw(),
                crop_w,
                crop_h,
                image::ExtendedColorType::Rgba8,
            ) {
                eprintln!("Failed to encode PNG: {e}");
                return None;
            }
            eprintln!("Clipboard: {} bytes PNG", png_data.len());
            return Some(Arc::new(png_data));
        }

        None
    }
}

fn default_output_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join("Pictures").join("Screenshots");
    let ts = timestamp();
    dir.join(format!("screenshot_{ts}.png"))
}

fn timestamp() -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let tm = libc::localtime(&t);
        if tm.is_null() {
            return format!("{t}");
        }
        let tm = &*tm;
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

// ── Winit ApplicationHandler ──────────────────────────────────────────────────

impl ScreenshotApp {
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.state = None;
        event_loop.exit();
    }
}

impl ApplicationHandler for ScreenshotApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Lantern Screenshot")
            .with_fullscreen(Some(Fullscreen::Borderless(None)))
            .with_decorations(false);

        let window = event_loop
            .create_window(attrs)
            .expect("failed to create window");

        let size = window.inner_size();
        let gpu = GpuContext::from_window(&window, size.width.max(1), size.height.max(1))
            .expect("failed to create GPU context");
        let painter = Painter::new(&gpu);
        let text = TextRenderer::new(&gpu);
        let tex_pass = TexturePass::new(&gpu);

        let cap = self.capture.take().expect("capture data missing");
        let screenshot_tex = tex_pass.upload(&gpu, &cap.data, cap.width, cap.height);
        let palette = FoxPalette::dark();

        let window_id = window.id();
        window.request_redraw();

        self.state = Some(AppState {
            window,
            window_id,
            gpu,
            painter,
            text,
            tex_pass,
            screenshot_tex,
            palette,
            capture_data: cap.data,
            capture_width: cap.width,
            capture_height: cap.height,
            selection: None,
            drag_mode: DragMode::None,
            cursor_pos: (0.0, 0.0),
            modifiers: ModifiersState::empty(),
            output_path: self.output_path.clone(),
        });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.window_id != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => self.shutdown(event_loop),

            WindowEvent::Resized(size) => {
                state.gpu.resize(size.width.max(1), size.height.max(1));
                state.window.request_redraw();
            }

            WindowEvent::ScaleFactorChanged { .. } => {
                let size = state.window.inner_size();
                state.gpu.resize(size.width.max(1), size.height.max(1));
                state.window.request_redraw();
            }

            WindowEvent::ModifiersChanged(mods) => {
                state.modifiers = mods.state();
            }

            WindowEvent::CursorMoved { position, .. } => {
                state.on_cursor_moved(position.x as f32, position.y as f32);
            }

            WindowEvent::MouseInput {
                state: button_state,
                button: MouseButton::Left,
                ..
            } => {
                match button_state {
                    ElementState::Pressed => {
                        state.on_left_pressed(state.cursor_pos.0, state.cursor_pos.1);
                    }
                    ElementState::Released => {
                        state.on_left_released();
                    }
                }
                state.window.request_redraw();
            }

            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => {
                        self.shutdown(event_loop);
                        return;
                    }
                    PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => {
                        self.clipboard_data = state.export(true, true);
                        self.shutdown(event_loop);
                        return;
                    }
                    PhysicalKey::Code(KeyCode::KeyC)
                        if state.modifiers.control_key() =>
                    {
                        self.clipboard_data = state.export(true, false);
                        self.shutdown(event_loop);
                        return;
                    }
                    PhysicalKey::Code(KeyCode::KeyS)
                        if state.modifiers.control_key() =>
                    {
                        state.export(false, true);
                        self.shutdown(event_loop);
                        return;
                    }
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => match state.render() {
                Ok(()) => {}
                Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                    let size = state.window.inner_size();
                    state.gpu.resize(size.width.max(1), size.height.max(1));
                    state.window.request_redraw();
                }
                Err(SurfaceError::OutOfMemory) => event_loop.exit(),
                Err(SurfaceError::Timeout | SurfaceError::Other) => {
                    state.window.request_redraw();
                }
            },
            _ => {}
        }
    }
}

//! Lantern screenshot tool.
//!
//! Flow:
//!   1. Parse args (`--delay`, `--output`).
//!   2. Open a fullscreen overlay layer surface (wayland.rs) to learn
//!      which output we're on, and capture that output via
//!      `zwlr_screencopy_v1` (capture.rs).
//!   3. Take exclusive keyboard focus. The overlay sits above every other
//!      surface including the Command Center and fullscreen windows, and
//!      the exclusive grab means Ctrl+C / Enter always reach us.
//!   4. Run the selection UI on top of the captured frame.
//!   5. On commit: encode PNG, destroy the layer surface (releasing
//!      input grab), then serve the PNG on the Wayland clipboard via
//!      `zwlr_data_control_v1` (clipboard.rs).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use lntrn_render::{GpuContext, Painter, SurfaceError, TextRenderer, TexturePass};

mod capture;
mod clipboard;
mod export;
mod render;
mod selection;
mod toolbar;
mod wayland;
mod window_query;

use selection::{DragMode, HandleEdge, Selection, HANDLE_HIT};
use toolbar::{ToolbarAction, ToolbarLayout};
use wayland::{FrameInput, LayerWindow};
use window_query::WindowRect;

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
        std::thread::sleep(Duration::from_secs(delay));
    }

    // Open the layer surface *before* capturing so we can ask the
    // compositor which output it placed us on (via wl_surface::enter),
    // then capture that same output. Without this, on multi-monitor
    // setups the screenshot would show whichever output the registry
    // enumerated first — typically the wrong monitor.
    //
    // The layer surface has no buffer attached at this point, so it
    // does not appear on screen and does not pollute the capture.
    eprintln!("Opening selection overlay...");
    let mut window = LayerWindow::new()?;

    let target_output = window.entered_output_name();
    eprintln!("Capturing output {:?}...", target_output);
    let cap = capture::capture_screen(target_output.as_deref())?;
    eprintln!("Captured {}x{}", cap.width, cap.height);
    window.grab_keyboard();

    let phys_w = window.state.phys_width().max(1);
    let phys_h = window.state.phys_height().max(1);

    let mut gpu = GpuContext::from_window(&window.handle, phys_w, phys_h)
        .map_err(|e| anyhow::anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let tex_pass = TexturePass::new(&gpu);
    let screenshot_tex = tex_pass.upload(&gpu, &cap.data, cap.width, cap.height);

    let mut ui = SelectionUi {
        selection: None,
        drag_mode: DragMode::None,
        cursor: (0.0, 0.0),
        capture_data: cap.data,
        capture_width: cap.width,
        capture_height: cap.height,
        output_path,
        mode: UiMode::Normal,
        target_output,
        windows: Vec::new(),
        windows_queried: false,
        hover_window: None,
    };

    let mut commit: Option<CommitAction> = None;

    while window.state.running && commit.is_none() {
        window.dispatch()?;

        let scale = window.state.fractional_scale() as f32;
        let cur_phys_w = window.state.phys_width().max(1);
        let cur_phys_h = window.state.phys_height().max(1);
        if cur_phys_w != gpu.width() || cur_phys_h != gpu.height() {
            gpu.resize(cur_phys_w, cur_phys_h);
            if let Some(vp) = window.viewport.as_ref() {
                vp.set_destination(window.state.width as i32, window.state.height as i32);
            }
        }

        let input = window.state.take_frame_input();
        commit = ui.handle_input(&input, cur_phys_w as f32, cur_phys_h as f32, scale);
        if commit.is_some() {
            break;
        }

        if window.state.frame_done {
            window.request_frame();
            match ui.render(
                &mut gpu,
                &mut painter,
                &mut text,
                &tex_pass,
                &screenshot_tex,
                scale,
            ) {
                Ok(()) => {
                    window.surface.commit();
                    let _ = window.conn.flush();
                }
                Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                    gpu.resize(cur_phys_w, cur_phys_h);
                }
                Err(SurfaceError::OutOfMemory) => break,
                Err(SurfaceError::Timeout | SurfaceError::Other) => {}
            }
        }
    }

    let png_data = match commit {
        Some(CommitAction::SaveAndCopy) => ui.export(true, true),
        Some(CommitAction::CopyOnly) => ui.export(true, false),
        Some(CommitAction::SaveOnly) => {
            ui.export(false, true);
            None
        }
        Some(CommitAction::Cancel) | None => None,
    };

    // Drop GPU resources before destroying the surface so wgpu's wayland
    // handle isn't holding a dangling pointer when the surface goes away.
    drop(screenshot_tex);
    drop(tex_pass);
    drop(text);
    drop(painter);
    drop(gpu);
    window.destroy();

    if let Some(png_data) = png_data {
        eprintln!("Serving clipboard...");
        if let Err(e) = clipboard::serve_clipboard(png_data) {
            eprintln!("Clipboard error: {e}");
        }
    }

    Ok(())
}

/// What the user asked us to do once they committed the selection.
#[derive(Clone, Copy, PartialEq)]
enum CommitAction {
    SaveAndCopy,
    CopyOnly,
    SaveOnly,
    Cancel,
}

/// Which interaction the overlay is in. Region selection is the default;
/// `PickWindow` is the transient "click a window to grab it" state entered
/// from the toolbar.
#[derive(Clone, Copy, PartialEq)]
enum UiMode {
    Normal,
    PickWindow,
}

struct SelectionUi {
    selection: Option<Selection>,
    drag_mode: DragMode,
    cursor: (f32, f32),
    capture_data: Vec<u8>,
    capture_width: u32,
    capture_height: u32,
    output_path: Option<PathBuf>,

    mode: UiMode,
    /// Output the overlay/capture is on, used to query its windows.
    target_output: Option<String>,
    /// Window rectangles for the captured output (physical px, output-local),
    /// queried lazily the first time window-pick is entered. Bottom→top order.
    windows: Vec<WindowRect>,
    windows_queried: bool,
    /// Index into `windows` of the window currently under the cursor.
    hover_window: Option<usize>,
}

impl SelectionUi {
    fn handle_input(
        &mut self,
        input: &FrameInput,
        screen_w: f32,
        screen_h: f32,
        scale: f32,
    ) -> Option<CommitAction> {
        if input.esc {
            // While picking a window, Esc just backs out to normal mode
            // instead of cancelling the whole screenshot.
            if self.mode == UiMode::PickWindow {
                self.mode = UiMode::Normal;
                return None;
            }
            return Some(CommitAction::Cancel);
        }
        if input.enter {
            return Some(CommitAction::SaveAndCopy);
        }
        if input.ctrl_c {
            return Some(CommitAction::CopyOnly);
        }
        if input.ctrl_s {
            return Some(CommitAction::SaveOnly);
        }

        let toolbar = ToolbarLayout::compute(screen_w, screen_h, scale);

        if input.cursor_moved {
            self.cursor = (input.cursor_x, input.cursor_y);
            if self.mode == UiMode::PickWindow {
                self.update_hover_window(input.cursor_x, input.cursor_y);
            } else {
                self.on_cursor_moved(input.cursor_x, input.cursor_y);
            }
        }

        if input.left_pressed {
            let (cx, cy) = self.cursor;
            // The toolbar sits on top of everything and absorbs presses.
            if toolbar.panel_contains(cx, cy) {
                if let Some(action) = toolbar.button_at(cx, cy) {
                    self.on_toolbar_action(action);
                }
                return None;
            }
            match self.mode {
                UiMode::PickWindow => self.on_pick_window_click(cx, cy),
                UiMode::Normal => self.on_left_pressed(cx, cy),
            }
        }

        if input.left_released && self.mode == UiMode::Normal {
            self.on_left_released();
        }
        None
    }

    fn on_toolbar_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::FullScreen => {
                self.mode = UiMode::Normal;
                self.selection = Some(Selection::from_normalized(
                    0.0,
                    0.0,
                    self.capture_width as f32,
                    self.capture_height as f32,
                ));
                self.drag_mode = DragMode::None;
            }
            ToolbarAction::Window => {
                if !self.windows_queried {
                    self.windows = window_query::query_windows(self.target_output.as_deref());
                    self.windows_queried = true;
                }
                self.mode = UiMode::PickWindow;
                self.hover_window = None;
                self.update_hover_window(self.cursor.0, self.cursor.1);
            }
        }
    }

    /// Topmost window under the cursor = last match in bottom→top order.
    fn update_hover_window(&mut self, cx: f32, cy: f32) {
        self.hover_window = self.windows.iter().rposition(|w| w.contains(cx, cy));
    }

    fn on_pick_window_click(&mut self, cx: f32, cy: f32) {
        self.update_hover_window(cx, cy);
        if let Some(idx) = self.hover_window {
            let w = &self.windows[idx];
            // Clamp to the captured area so an off-screen window edge can't
            // produce a selection outside the image.
            let x = w.x.max(0.0);
            let y = w.y.max(0.0);
            let right = (w.x + w.w).min(self.capture_width as f32);
            let bottom = (w.y + w.h).min(self.capture_height as f32);
            self.selection = Some(Selection::from_normalized(x, y, right - x, bottom - y));
        }
        // Whether or not we hit a window, leave pick mode — a stray click in
        // empty space drops back to normal rather than trapping the user.
        self.mode = UiMode::Normal;
    }

    fn on_cursor_moved(&mut self, cx: f32, cy: f32) {
        match self.drag_mode {
            DragMode::New { start_x, start_y } => {
                self.selection = Some(Selection {
                    x: start_x,
                    y: start_y,
                    w: cx - start_x,
                    h: cy - start_y,
                });
            }
            DragMode::Handle { edge, orig } => {
                let (ox, oy, ow, oh) = orig;
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
            }
            DragMode::Move { offset_x, offset_y } => {
                if let Some(ref sel) = self.selection {
                    let (_, _, w, h) = sel.normalized();
                    self.selection = Some(Selection::from_normalized(
                        cx - offset_x,
                        cy - offset_y,
                        w,
                        h,
                    ));
                }
            }
            DragMode::None => {}
        }
    }

    fn on_left_pressed(&mut self, cx: f32, cy: f32) {
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
        self.drag_mode = DragMode::New {
            start_x: cx,
            start_y: cy,
        };
        self.selection = None;
    }

    fn on_left_released(&mut self) {
        if let Some(ref sel) = self.selection {
            let (x, y, w, h) = sel.normalized();
            if w > 2.0 && h > 2.0 {
                self.selection = Some(Selection::from_normalized(x, y, w, h));
            } else {
                self.selection = None;
            }
        }
        self.drag_mode = DragMode::None;
    }
}

// Keep the unused-import-warning quiet for HANDLE_HIT (re-exported for
// API completeness; the hit-test uses it internally).
#[allow(dead_code)]
const _: f32 = HANDLE_HIT;

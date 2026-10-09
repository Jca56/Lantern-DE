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
//!   4. Run the selection UI on top of the captured frame: set the
//!      region, then draw on it (annotate.rs).
//!   5. On commit: lay what was drawn over the captured pixels, encode
//!      PNG, destroy the layer surface (releasing
//!      input grab), then serve the PNG on the Wayland clipboard via
//!      `zwlr_data_control_v1` (clipboard.rs).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use lntrn_render::{GpuContext, Painter, SurfaceError, TextRenderer, TexturePass};

mod annotate;
mod capture;
mod clipboard;
mod export;
mod region;
mod render;
mod selection;
mod settings;
mod toolbar;
mod wayland;
mod window_query;

use annotate::{Mark, Tool};
use selection::{DragMode, Selection, HANDLE_HIT};
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
    let plain_tex = tex_pass.upload(&gpu, &cap.data, cap.width, cap.height);
    let cursor_tex = tex_pass.upload(&gpu, &cap.data_with_cursor, cap.width, cap.height);

    let mut ui = SelectionUi {
        selection: None,
        drag_mode: DragMode::None,
        cursor: (0.0, 0.0),
        capture_data: cap.data,
        capture_with_cursor: cap.data_with_cursor,
        hide_mouse: settings::hide_mouse(),
        capture_width: cap.width,
        capture_height: cap.height,
        output_path,
        mode: UiMode::Normal,
        target_output,
        windows: Vec::new(),
        windows_queried: false,
        hover_window: None,
        marks: Vec::new(),
        drawing: None,
        typing: None,
        tool: None,
        color: 0,
        size: 1,
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
            let screenshot_tex = if ui.hide_mouse {
                &plain_tex
            } else {
                &cursor_tex
            };
            match ui.render(
                &mut gpu,
                &mut painter,
                &mut text,
                &tex_pass,
                screenshot_tex,
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

    // What was drawn on the screenshot, as pixels to lay over it. Done
    // while the GPU is still up: the marks are drawn by it once more.
    let ink = match commit {
        Some(CommitAction::Cancel) | None => None,
        Some(_) => annotate::ink::bake(&gpu, &mut painter, &mut text, &ui.marks),
    };
    let ink = ink.as_ref();
    let png_data = match commit {
        Some(CommitAction::SaveAndCopy) => ui.export(true, true, ink),
        Some(CommitAction::CopyOnly) => ui.export(true, false, ink),
        Some(CommitAction::SaveOnly) => {
            ui.export(false, true, ink);
            None
        }
        Some(CommitAction::Cancel) | None => None,
    };

    // Drop GPU resources before destroying the surface so wgpu's wayland
    // handle isn't holding a dangling pointer when the surface goes away.
    drop(plain_tex);
    drop(cursor_tex);
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
    /// The captured frame without the mouse cursor, and the same frame
    /// with it; `hide_mouse` picks which one is shown and saved.
    capture_data: Vec<u8>,
    capture_with_cursor: Vec<u8>,
    hide_mouse: bool,
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

    /// What has been drawn on the screenshot, oldest first.
    marks: Vec<Mark>,
    /// The mark being dragged out.
    drawing: Option<Mark>,
    /// The text being typed with the text tool, not yet a mark.
    typing: Option<Mark>,
    /// The drawing tool in hand. With none, drags set, move and size the
    /// region.
    tool: Option<Tool>,
    /// The colour and thickness the next mark gets (see `annotate`).
    color: usize,
    size: usize,
}

impl SelectionUi {
    fn handle_input(
        &mut self,
        input: &FrameInput,
        screen_w: f32,
        screen_h: f32,
        scale: f32,
    ) -> Option<CommitAction> {
        // While text is being typed the keyboard is the text tool's:
        // Enter finishes the text and Esc throws it away.
        if self.typing.is_some() {
            self.type_text(&input.typed);
            if input.esc {
                self.typing = None;
                return None;
            }
            if input.enter {
                self.commit_typing();
                return None;
            }
        }
        if input.esc {
            // While picking a window, Esc just backs out to normal mode
            // instead of cancelling the whole screenshot.
            if self.mode == UiMode::PickWindow {
                self.mode = UiMode::Normal;
                return None;
            }
            // With a tool in hand, Esc puts it down.
            if self.tool.is_some() {
                self.tool = None;
                self.drawing = None;
                return None;
            }
            // With a region drawn, Esc drops it first, which brings the
            // toolbar back; the next Esc cancels.
            if self.selection.is_some() {
                self.selection = None;
                self.drag_mode = DragMode::None;
                return None;
            }
            return Some(CommitAction::Cancel);
        }
        if input.ctrl_z {
            self.undo();
        }
        if input.enter || input.ctrl_c || input.ctrl_s {
            // Text still being typed goes into the picture as it is.
            self.commit_typing();
            self.finish_mark();
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

        if input.cursor_moved {
            self.cursor = (input.cursor_x, input.cursor_y);
            if self.drawing.is_some() {
                self.drag_mark(self.cursor);
            } else if self.mode == UiMode::PickWindow {
                self.update_hover_window(input.cursor_x, input.cursor_y);
            } else {
                self.on_cursor_moved(input.cursor_x, input.cursor_y);
            }
        }

        if input.left_pressed {
            let (cx, cy) = self.cursor;
            // The toolbar sits on top of everything and absorbs presses.
            if self.toolbar_visible() {
                let toolbar = ToolbarLayout::compute(screen_w, screen_h, scale);
                if toolbar.panel_contains(cx, cy) {
                    if let Some(action) = toolbar.button_at(cx, cy) {
                        self.on_toolbar_action(action);
                    }
                    return None;
                }
            }
            // So does the bar by the region.
            if let Some(bar) = self.bar_layout(screen_w, screen_h, scale) {
                if bar.panel_contains(cx, cy) {
                    if let Some(action) = bar.action_at(cx, cy) {
                        self.on_bar_action(action);
                    }
                    return None;
                }
            }
            match (self.mode, self.tool) {
                (UiMode::PickWindow, _) => self.on_pick_window_click(cx, cy),
                (UiMode::Normal, Some(tool)) => self.start_mark(tool, (cx, cy), scale),
                (UiMode::Normal, None) => self.on_left_pressed(cx, cy),
            }
        }

        if input.left_released && self.mode == UiMode::Normal {
            if self.drawing.is_some() {
                self.finish_mark();
            } else {
                self.on_left_released();
            }
        }
        None
    }

    /// The toolbar shows only while no region is drawn, so it is never in
    /// the way of setting one.
    fn toolbar_visible(&self) -> bool {
        self.selection.is_none()
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
            ToolbarAction::HideMouse => {
                self.hide_mouse = !self.hide_mouse;
                if let Err(e) = settings::set_hide_mouse(self.hide_mouse) {
                    eprintln!("Failed to remember hide mouse: {e}");
                }
            }
        }
    }
}

// Keep the unused-import-warning quiet for HANDLE_HIT (re-exported for
// API completeness; the hit-test uses it internally).
#[allow(dead_code)]
const _: f32 = HANDLE_HIT;

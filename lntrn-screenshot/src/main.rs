//! Lantern screenshot tool.
//!
//! Flow:
//!   1. Parse args (`--delay`, `--output`).
//!   2. Capture the output via `zwlr_screencopy_v1` (capture.rs).
//!   3. Open a fullscreen overlay layer surface with exclusive keyboard
//!      focus (wayland.rs). The overlay sits above every other layer
//!      surface including the Command Center, and the exclusive grab
//!      means Ctrl+C / Enter always reach us.
//!   4. Run the selection UI on top of the captured frame.
//!   5. On commit: encode PNG, destroy the layer surface (releasing
//!      input grab), then serve the PNG on the Wayland clipboard via
//!      `zwlr_data_control_v1` (clipboard.rs).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use lntrn_render::{
    Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer, TextureDraw, TexturePass,
};

mod capture;
mod clipboard;
mod selection;
mod wayland;

use selection::{DragMode, HandleEdge, Selection, HANDLE_HIT, HANDLE_SIZE};
use wayland::{FrameInput, LayerWindow};

// Lantern look. Inlined to avoid pulling lntrn-ui / lntrn-theme into a
// tool this small (per project convention: apps style directly with
// Painter rather than depending on the shared theme crates).
// Built via `from_rgba8` (sRGB → linear), so these can't be `const`.
fn text_tan() -> Color { Color::from_rgba8(0xe8, 0xdc, 0xc8, 0xff) }
fn accent_orange() -> Color { Color::from_rgba8(0xff, 0x9b, 0x42, 0xff) }

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
        commit = ui.handle_input(&input);
        if commit.is_some() {
            break;
        }

        if window.state.frame_done {
            window.request_frame();
            match ui.render(&mut gpu, &mut painter, &mut text, &tex_pass, &screenshot_tex, scale) {
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

struct SelectionUi {
    selection: Option<Selection>,
    drag_mode: DragMode,
    cursor: (f32, f32),
    capture_data: Vec<u8>,
    capture_width: u32,
    capture_height: u32,
    output_path: Option<PathBuf>,
}

impl SelectionUi {
    fn handle_input(&mut self, input: &FrameInput) -> Option<CommitAction> {
        if input.esc {
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

        if input.cursor_moved {
            self.cursor = (input.cursor_x, input.cursor_y);
            self.on_cursor_moved(input.cursor_x, input.cursor_y);
        }
        if input.left_pressed {
            self.on_left_pressed(self.cursor.0, self.cursor.1);
        }
        if input.left_released {
            self.on_left_released();
        }
        None
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
        self.drag_mode = DragMode::New { start_x: cx, start_y: cy };
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

    fn render(
        &self,
        gpu: &mut GpuContext,
        painter: &mut Painter,
        text: &mut TextRenderer,
        tex_pass: &TexturePass,
        screenshot_tex: &lntrn_render::GpuTexture,
        scale: f32,
    ) -> Result<(), SurfaceError> {
        let sw = gpu.width() as f32;
        let sh = gpu.height() as f32;
        let dim = Color::from_rgba8(0, 0, 0, 140);

        painter.clear();

        if let Some(ref sel) = self.selection {
            let (sx, sy, sw_, sh_) = sel.normalized();

            painter.rect_filled(Rect::new(0.0, 0.0, sw, sy), 0.0, dim);
            painter.rect_filled(Rect::new(0.0, sy + sh_, sw, sh - sy - sh_), 0.0, dim);
            painter.rect_filled(Rect::new(0.0, sy, sx, sh_), 0.0, dim);
            painter.rect_filled(Rect::new(sx + sw_, sy, sw - sx - sw_, sh_), 0.0, dim);

            let stroke = (2.0 * scale).max(2.0);
            painter.rect_stroke(Rect::new(sx, sy, sw_, sh_), 0.0, stroke, accent_orange());

            // Drag handles — scaled with the output so they're easy to grab on HiDPI.
            let hs = HANDLE_SIZE * scale.max(1.0);
            let half = hs / 2.0;
            let handles = [
                (sx - half, sy - half),
                (sx + sw_ / 2.0 - half, sy - half),
                (sx + sw_ - half, sy - half),
                (sx + sw_ - half, sy + sh_ / 2.0 - half),
                (sx + sw_ - half, sy + sh_ - half),
                (sx + sw_ / 2.0 - half, sy + sh_ - half),
                (sx - half, sy + sh_ - half),
                (sx - half, sy + sh_ / 2.0 - half),
            ];
            for (hx, hy) in handles {
                painter.rect_filled(Rect::new(hx, hy, hs, hs), 2.0, Color::WHITE);
                painter.rect_stroke(Rect::new(hx, hy, hs, hs), 2.0, 1.0 * scale, accent_orange());
            }

            // Size readout above (or below) the selection.
            let label = format!("{} x {}", sw_ as u32, sh_ as u32);
            let label_font = 18.0 * scale.max(1.0);
            let label_pad = 6.0 * scale.max(1.0);
            let label_box_w = 180.0 * scale.max(1.0);
            let label_box_h = label_font + label_pad * 2.0;
            let label_y = if sy > label_box_h + 4.0 {
                sy - label_box_h - 4.0
            } else {
                sy + sh_ + 4.0
            };
            painter.rect_filled(
                Rect::new(sx, label_y, label_box_w, label_box_h),
                4.0 * scale.max(1.0),
                Color::from_rgba8(0, 0, 0, 200),
            );
            text.queue(
                &label,
                label_font,
                sx + label_pad,
                label_y + label_pad,
                text_tan(),
                label_box_w - label_pad * 2.0,
                sw as u32,
                sh as u32,
            );
        } else {
            painter.rect_filled(Rect::new(0.0, 0.0, sw, sh), 0.0, dim);
        }

        // Hint bar — sized up for readability.
        let hint = if self.selection.is_some() {
            "Enter = save + copy   \u{00b7}   Ctrl+C = copy   \u{00b7}   Ctrl+S = save   \u{00b7}   Esc = cancel"
        } else {
            "Drag to select a region   \u{00b7}   Enter = capture full screen   \u{00b7}   Esc = cancel"
        };
        let hint_font = 26.0 * scale.max(1.0);
        let hint_pad_x = 28.0 * scale.max(1.0);
        let hint_pad_y = 14.0 * scale.max(1.0);
        let hint_box_w = 1100.0 * scale.max(1.0);
        let hint_box_h = hint_font + hint_pad_y * 2.0;
        let hint_x = sw / 2.0 - hint_box_w / 2.0;
        let hint_y = sh - hint_box_h - 32.0 * scale.max(1.0);
        painter.rect_filled(
            Rect::new(hint_x, hint_y, hint_box_w, hint_box_h),
            12.0 * scale.max(1.0),
            Color::from_rgba8(0, 0, 0, 210),
        );
        text.queue(
            hint,
            hint_font,
            hint_x + hint_pad_x,
            hint_y + hint_pad_y,
            text_tan(),
            hint_box_w - hint_pad_x * 2.0,
            sw as u32,
            sh as u32,
        );

        let mut frame = gpu.begin_frame("screenshot")?;
        let view = frame.view().clone();

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

        let tex_draw = TextureDraw::new(screenshot_tex, 0.0, 0.0, sw, sh);
        tex_pass.render_pass(gpu, frame.encoder_mut(), &view, &[tex_draw], None);
        painter.render_pass_overlay(gpu, frame.encoder_mut(), &view);
        text.render_queued(gpu, frame.encoder_mut(), &view);

        frame.submit(&gpu.queue);
        Ok(())
    }

    fn export(&self, copy: bool, save: bool) -> Option<Arc<Vec<u8>>> {
        // Selection coords are in physical pixels (same space as the
        // captured image), so we crop directly without rescaling.
        let (crop_x, crop_y, crop_w, crop_h) = if let Some(ref sel) = self.selection {
            let (sx, sy, sw, sh) = sel.normalized();
            (
                sx.max(0.0) as u32,
                sy.max(0.0) as u32,
                (sw.max(1.0) as u32).min(self.capture_width),
                (sh.max(1.0) as u32).min(self.capture_height),
            )
        } else {
            (0, 0, self.capture_width, self.capture_height)
        };

        let img = image::RgbaImage::from_raw(
            self.capture_width,
            self.capture_height,
            self.capture_data.clone(),
        )?;
        let cropped = image::imageops::crop_imm(&img, crop_x, crop_y, crop_w, crop_h).to_image();

        if save {
            let path = self.output_path.clone().unwrap_or_else(default_output_path);
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

// Keep the unused-import-warning quiet for HANDLE_HIT (re-exported for
// API completeness; the hit-test uses it internally).
#[allow(dead_code)]
const _: f32 = HANDLE_HIT;

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

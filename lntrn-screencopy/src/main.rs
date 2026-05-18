//! Lantern screen recorder — region-based, cursor's-monitor-aware,
//! toggled by re-invocation.
//!
//! Run once: bring up a region selection UI on the cursor's monitor,
//! confirm with Enter, then start recording (with an on-screen
//! indicator and red dot). Run again while a recording is in
//! progress: the second invocation hands a stop signal to the first
//! via a unix socket and exits. Ctrl+C also stops cleanly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use anyhow::Result;
use lntrn_render::{Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer};

mod capture;
mod indicator;
mod ipc;
mod selection;
mod selection_window;

use capture::Region;
use selection::{DragMode, HandleEdge, Selection, HANDLE_HIT, HANDLE_SIZE};
use selection_window::{FrameInput, LayerWindow};

static STOP: AtomicBool = AtomicBool::new(false);
static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);

fn text_tan() -> Color { Color::from_rgba8(0xe8, 0xdc, 0xc8, 0xff) }
fn accent_orange() -> Color { Color::from_rgba8(0xff, 0x9b, 0x42, 0xff) }
fn accent_red() -> Color { Color::from_rgba8(0xff, 0x60, 0x60, 0xff) }

fn main() -> Result<()> {
    // First invocation while another recorder is running? Toggle it
    // off and exit. This lets a single hotkey both start and stop.
    if ipc::try_toggle_existing() {
        return Ok(());
    }

    let (output_path, framerate) = parse_args();
    if let Some(dir) = output_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    install_signal_handler();
    let _socket_guard = ipc::spawn_listener(&STOP);

    // ── Selection phase ─────────────────────────────────────────────────
    let Some((output_name, region)) = run_selection_ui()? else {
        eprintln!("Recording cancelled.");
        return Ok(());
    };
    if region.w < 16 || region.h < 16 {
        eprintln!("Selected region too small; aborting.");
        return Ok(());
    }
    eprintln!(
        "Selected {}x{} at ({},{}) on {output_name}",
        region.w, region.h, region.x, region.y
    );

    // ── Indicator + capture loop ────────────────────────────────────────
    let started_at = Instant::now();
    indicator::spawn(output_name.clone(), &STOP, started_at, &FRAME_COUNT);

    eprintln!("Recording to {} @ {framerate} fps", output_path.display());
    eprintln!("Click STOP, re-run `lntrn-screencopy`, or press Ctrl+C to finish.\n");
    match capture::record_region(&output_name, region, &output_path, framerate, &STOP) {
        Ok(frames) => {
            FRAME_COUNT.store(frames, Ordering::Relaxed);
            let secs = started_at.elapsed().as_secs_f64();
            eprintln!(
                "\nRecording finished: {frames} frames, {secs:.1}s, avg {:.1} fps",
                if secs > 0.0 { frames as f64 / secs } else { 0.0 }
            );
            if let Ok(meta) = std::fs::metadata(&output_path) {
                let mb = meta.len() as f64 / (1024.0 * 1024.0);
                eprintln!("Output: {} ({:.1} MB)", output_path.display(), mb);
            }
        }
        Err(e) => {
            eprintln!("Recording failed: {e}");
            STOP.store(true, Ordering::SeqCst);
            std::process::exit(1);
        }
    }

    // Tell the indicator thread to wind down if the capture loop
    // finished on its own (e.g. ffmpeg pipe closed unexpectedly).
    STOP.store(true, Ordering::SeqCst);
    Ok(())
}

// ── Selection UI ────────────────────────────────────────────────────────────

fn run_selection_ui() -> Result<Option<(String, Region)>> {
    let mut window = LayerWindow::new()?;
    let output_name = window
        .entered_output_name()
        .ok_or_else(|| anyhow::anyhow!("compositor never told us which output we're on"))?;

    let phys_w = window.state.phys_width().max(1);
    let phys_h = window.state.phys_height().max(1);
    let mut gpu = GpuContext::from_window(&window.handle, phys_w, phys_h)
        .map_err(|e| anyhow::anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);

    let mut ui = SelectionUi {
        selection: None,
        drag_mode: DragMode::None,
        cursor: (0.0, 0.0),
    };

    let mut commit: Option<CommitAction> = None;
    while window.state.running && commit.is_none() && !STOP.load(Ordering::SeqCst) {
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
        if commit.is_some() { break; }

        if window.state.frame_done {
            window.request_frame();
            match ui.render(&mut gpu, &mut painter, &mut text, scale) {
                Ok(()) => {
                    window.surface.commit();
                    let _ = window.conn.flush();
                }
                Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                    gpu.resize(cur_phys_w, cur_phys_h);
                }
                Err(_) => {}
            }
        }
    }

    let result = match commit {
        Some(CommitAction::Confirm) => ui.selection.map(|s| {
            let (x, y, w, h) = s.normalized();
            Region {
                x: x.max(0.0) as u32,
                y: y.max(0.0) as u32,
                w: w.max(1.0) as u32,
                h: h.max(1.0) as u32,
            }
        }),
        _ => None,
    };

    drop(text); drop(painter); drop(gpu);
    window.destroy();

    Ok(result.map(|r| (output_name, r)))
}

#[derive(Clone, Copy, PartialEq)]
enum CommitAction { Confirm, Cancel }

struct SelectionUi {
    selection: Option<Selection>,
    drag_mode: DragMode,
    cursor: (f32, f32),
}

impl SelectionUi {
    fn handle_input(&mut self, input: &FrameInput) -> Option<CommitAction> {
        if input.esc { return Some(CommitAction::Cancel); }
        if input.enter { return Some(CommitAction::Confirm); }

        if input.cursor_moved {
            self.cursor = (input.cursor_x, input.cursor_y);
            self.on_cursor_moved(input.cursor_x, input.cursor_y);
        }
        if input.left_pressed { self.on_left_pressed(self.cursor.0, self.cursor.1); }
        if input.left_released { self.on_left_released(); }
        None
    }

    fn on_cursor_moved(&mut self, cx: f32, cy: f32) {
        match self.drag_mode {
            DragMode::New { start_x, start_y } => {
                self.selection = Some(Selection {
                    x: start_x, y: start_y, w: cx - start_x, h: cy - start_y,
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
                        cx - offset_x, cy - offset_y, w, h,
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
                self.drag_mode = DragMode::Move { offset_x: cx - sx, offset_y: cy - sy };
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
        scale: f32,
    ) -> Result<(), SurfaceError> {
        let sw = gpu.width() as f32;
        let sh = gpu.height() as f32;
        let dim = Color::from_rgba8(0, 0, 0, 140);

        painter.clear();

        // The four-rect dim pattern leaves the selection interior at
        // alpha=0 so the live desktop shows through the "hole" — no
        // captured background image is needed.
        if let Some(ref sel) = self.selection {
            let (sx, sy, sw_, sh_) = sel.normalized();
            painter.rect_filled(Rect::new(0.0, 0.0, sw, sy), 0.0, dim);
            painter.rect_filled(Rect::new(0.0, sy + sh_, sw, sh - sy - sh_), 0.0, dim);
            painter.rect_filled(Rect::new(0.0, sy, sx, sh_), 0.0, dim);
            painter.rect_filled(Rect::new(sx + sw_, sy, sw - sx - sw_, sh_), 0.0, dim);

            let stroke = (2.0 * scale).max(2.0);
            painter.rect_stroke(Rect::new(sx, sy, sw_, sh_), 0.0, stroke, accent_red());

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
                painter.rect_stroke(Rect::new(hx, hy, hs, hs), 2.0, 1.0 * scale, accent_red());
            }

            let label = format!("{} x {}", sw_ as u32, sh_ as u32);
            let label_font = 18.0 * scale.max(1.0);
            let label_pad = 6.0 * scale.max(1.0);
            let label_box_w = 200.0 * scale.max(1.0);
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
                &label, label_font,
                sx + label_pad, label_y + label_pad,
                text_tan(),
                label_box_w - label_pad * 2.0,
                sw as u32, sh as u32,
            );
        } else {
            painter.rect_filled(Rect::new(0.0, 0.0, sw, sh), 0.0, dim);
        }

        // Hint bar.
        let hint = if self.selection.is_some() {
            "Enter = start recording   \u{00b7}   Esc = cancel"
        } else {
            "Drag to select a region to record   \u{00b7}   Esc = cancel"
        };
        let hint_font = 26.0 * scale.max(1.0);
        let hint_pad_x = 28.0 * scale.max(1.0);
        let hint_pad_y = 14.0 * scale.max(1.0);
        let hint_box_w = 900.0 * scale.max(1.0);
        let hint_box_h = hint_font + hint_pad_y * 2.0;
        let hint_x = sw / 2.0 - hint_box_w / 2.0;
        let hint_y = sh - hint_box_h - 32.0 * scale.max(1.0);
        painter.rect_filled(
            Rect::new(hint_x, hint_y, hint_box_w, hint_box_h),
            12.0 * scale.max(1.0),
            Color::from_rgba8(0, 0, 0, 210),
        );
        text.queue(
            hint, hint_font,
            hint_x + hint_pad_x, hint_y + hint_pad_y,
            accent_orange(),
            hint_box_w - hint_pad_x * 2.0,
            sw as u32, sh as u32,
        );

        let mut frame = gpu.begin_frame("selection")?;
        let view = frame.view().clone();
        {
            let encoder = frame.encoder_mut();
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("selection-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }
        painter.render_pass_overlay(gpu, frame.encoder_mut(), &view);
        text.render_queued(gpu, frame.encoder_mut(), &view);
        frame.submit(&gpu.queue);
        Ok(())
    }
}

// ── CLI / signals ──────────────────────────────────────────────────────────

fn parse_args() -> (PathBuf, u32) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut output: Option<PathBuf> = None;
    let mut framerate: u32 = 60;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" if i + 1 < args.len() => {
                output = Some(PathBuf::from(&args[i + 1])); i += 2;
            }
            "-r" | "--framerate" if i + 1 < args.len() => {
                framerate = args[i + 1].parse().unwrap_or_else(|_| {
                    eprintln!("--framerate requires a number"); std::process::exit(1);
                });
                i += 2;
            }
            "-h" | "--help" => {
                eprintln!("Usage: lntrn-screencopy [OPTIONS]");
                eprintln!("  -o, --output <PATH>    Output file (default: ~/Videos/recording_*.mp4)");
                eprintln!("  -r, --framerate <FPS>  Target framerate (default: 60)");
                eprintln!("  -h, --help             Show this help");
                eprintln!();
                eprintln!("Run a second time during recording to stop and finalize.");
                std::process::exit(0);
            }
            _ => { eprintln!("Unknown argument: {}", args[i]); std::process::exit(1); }
        }
    }
    (output.unwrap_or_else(default_output_path), framerate)
}

fn default_output_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join("Videos").join(format!("recording_{}.mp4", timestamp()))
}

fn timestamp() -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let tm = libc::localtime(&t);
        if tm.is_null() { return format!("{t}"); }
        let tm = &*tm;
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday,
            tm.tm_hour, tm.tm_min, tm.tm_sec,
        )
    }
}

fn install_signal_handler() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_signal as *const () as usize;
        sa.sa_flags = 0;
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

extern "C" fn handle_signal(_sig: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

// Keep HANDLE_HIT referenced — selection.rs uses it internally but the
// const is also re-exported for API completeness; without this the
// unused-import lint fires.
#[allow(dead_code)]
const _: f32 = HANDLE_HIT;

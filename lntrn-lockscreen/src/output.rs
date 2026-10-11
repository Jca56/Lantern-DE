//! One lock surface per monitor: its Wayland objects, its GPU resources, and
//! drawing it.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use lntrn_render::{
    GpuContext, GpuTexture, Painter, SurfaceError, TextRenderer, TextureDraw, TexturePass,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_output, wl_surface};
use wayland_client::Proxy;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::ExtSessionLockSurfaceV1;

use crate::config::Style;
use crate::render::{self, Ui};
use crate::wayland::App;

/// First and longest wait before retrying an output whose frame or GPU setup
/// failed. The wait doubles each time, so a monitor that stays broken costs
/// one attempt every few seconds instead of stalling every key press.
const RETRY_MIN: Duration = Duration::from_millis(100);
const RETRY_MAX: Duration = Duration::from_secs(5);

// ── raw-window-handle glue for wgpu ──────────────────────────────────────────

struct WaylandHandle {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}
impl HasDisplayHandle for WaylandHandle {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
impl HasWindowHandle for WaylandHandle {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(self.surface));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

// ── Per-output wayland objects + configure state ─────────────────────────────

pub(crate) struct OutputCtx {
    /// Registry name of this output's global, to match its removal.
    pub global: u32,
    pub surface: wl_surface::WlSurface,
    pub lock_surface: ExtSessionLockSurfaceV1,
    pub scale: i32,
    /// The buffer scale last set on the surface (0: none yet).
    applied_scale: i32,
    pub width: u32,
    pub height: u32,
    pub pending_serial: Option<u32>,
    pub configured: bool,
    pub dirty: bool,
    /// After a failure: don't try this output again before then.
    retry_at: Option<Instant>,
    retry_delay: Duration,
}

impl OutputCtx {
    pub fn new(
        global: u32,
        surface: wl_surface::WlSurface,
        lock_surface: ExtSessionLockSurfaceV1,
        scale: i32,
    ) -> Self {
        Self {
            global,
            surface,
            lock_surface,
            scale,
            applied_scale: 0,
            width: 0,
            height: 0,
            pending_serial: None,
            configured: false,
            dirty: false,
            retry_at: None,
            retry_delay: Duration::ZERO,
        }
    }

    /// Something failed: back off, a little longer each time in a row.
    fn schedule_retry(&mut self, now: Instant) -> Instant {
        self.retry_delay = (self.retry_delay * 2).clamp(RETRY_MIN, RETRY_MAX);
        let at = now + self.retry_delay;
        self.retry_at = Some(at);
        at
    }
}

// ── GPU resources for one output (lives in the run loop, not in dispatch) ────

pub(crate) struct OutputGpu {
    gpu: GpuContext,
    painter: Painter,
    text: TextRenderer,
    tex_pass: TexturePass,
    bg: GpuTexture,
}

/// GPU state per output, keyed by its wl_surface.
pub(crate) type Gpus = HashMap<ObjectId, OutputGpu>;

impl OutputGpu {
    fn new(
        display_ptr: *mut c_void,
        surface: &wl_surface::WlSurface,
        width: u32,
        height: u32,
        bg: &BgImage,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let handle = WaylandHandle {
            display: NonNull::new(display_ptr).ok_or("no wl_display")?,
            surface: NonNull::new(surface.id().as_ptr() as *mut c_void).ok_or("no wl_surface")?,
        };
        let gpu = GpuContext::from_window(&handle, width, height)?;
        let painter = Painter::new(&gpu);
        let text = TextRenderer::new(&gpu);
        let tex_pass = TexturePass::new(&gpu);
        let bg = upload_background(&tex_pass, &gpu, bg);
        Ok(Self {
            gpu,
            painter,
            text,
            tex_pass,
            bg,
        })
    }
}

/// Decoded background image (RGBA8), shared and uploaded per-device.
pub(crate) struct BgImage {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// The largest texture side a Lantern GPU context accepts. `GpuContext` asks
/// wgpu for its default limits, so this is the device's limit too.
pub(crate) fn max_texture_side() -> u32 {
    wgpu::Limits::default().max_texture_dimension_2d
}

/// Upload the background. `main` already shrinks an oversized image; this
/// checks again against the real device, because a texture past its limit
/// doesn't fail, it aborts the process, and a dead locker is a locked session
/// with nothing on screen to unlock it.
fn upload_background(tex_pass: &TexturePass, gpu: &GpuContext, bg: &BgImage) -> GpuTexture {
    let max = gpu.device.limits().max_texture_dimension_2d;
    let well_formed = bg.rgba.len() as u64 == bg.w as u64 * bg.h as u64 * 4;
    if bg.w == 0 || bg.h == 0 || bg.w > max || bg.h > max || !well_formed {
        eprintln!("[lockscreen] background {}x{} can't be a texture, using black", bg.w, bg.h);
        return tex_pass.upload(gpu, &[0, 0, 0, 255], 1, 1);
    }
    tex_pass.upload(gpu, &bg.rgba, bg.w, bg.h)
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Draw every output that needs it. Returns when the earliest failed output
/// is due another attempt, so the run loop wakes up for it.
pub(crate) fn render_outputs(
    app: &mut App,
    display_ptr: *mut c_void,
    bg: &BgImage,
    style: &Style,
    gpus: &mut Gpus,
) -> Option<Instant> {
    let ui = Ui {
        pw_len: app.entry.len(),
        error: app.entry.error(),
        checking: app.entry.checking(),
        caps_lock: app.keyboard.caps_active(),
    };
    let now = Instant::now();
    let mut next_retry: Option<Instant> = None;
    let mut retry_later = |at: Instant| next_retry = Some(next_retry.map_or(at, |t| t.min(at)));

    for out in app.outputs.iter_mut() {
        if !out.configured || out.width == 0 || out.height == 0 {
            continue;
        }
        let sid = out.surface.id();
        let max_side = match gpus.get(&sid) {
            Some(og) => og.gpu.device.limits().max_texture_dimension_2d,
            None => max_texture_side(),
        };
        let scale = fit_scale(out.width, out.height, out.scale, max_side);
        let buf_w = out.width * scale as u32;
        let buf_h = out.height * scale as u32;

        // Acknowledge any pending configure before committing a buffer.
        if let Some(serial) = out.pending_serial.take() {
            out.lock_surface.ack_configure(serial);
            out.dirty = true;
        }
        // The buffer scale has to track the buffer whatever changed it: a
        // buffer whose size doesn't divide down to the configured one is a
        // fatal protocol error for a lock surface.
        if out.applied_scale != scale {
            out.surface.set_buffer_scale(scale);
            out.applied_scale = scale;
            out.dirty = true;
        }

        if !out.dirty {
            continue;
        }
        if let Some(at) = out.retry_at.filter(|at| now < *at) {
            retry_later(at);
            continue;
        }

        let og = match gpus.entry(sid) {
            Entry::Occupied(e) => {
                let og = e.into_mut();
                og.gpu.resize(buf_w, buf_h);
                og
            }
            Entry::Vacant(e) => match OutputGpu::new(display_ptr, &out.surface, buf_w, buf_h, bg) {
                Ok(og) => e.insert(og),
                Err(err) => {
                    eprintln!("[lockscreen] GPU init failed: {err}");
                    retry_later(out.schedule_retry(now));
                    continue;
                }
            },
        };

        let (w, h) = (og.gpu.width(), og.gpu.height());
        og.painter.clear();
        og.text.clear();
        render::draw(&mut og.painter, &mut og.text, &ui, style, w as f32, h as f32, w, h);

        let mut frame = match og.gpu.begin_frame("lockscreen") {
            Ok(frame) => frame,
            Err(err) => {
                // A stale swapchain has to be rebuilt; anything else may just
                // pass. Either way the output stays dirty and gets another go,
                // instead of sitting black until the next key press.
                if matches!(err, SurfaceError::Outdated | SurfaceError::Lost) {
                    og.gpu.surface.configure(&og.gpu.device, &og.gpu.config);
                }
                eprintln!("[lockscreen] couldn't start a frame ({err}), will retry");
                retry_later(out.schedule_retry(now));
                continue;
            }
        };
        let view = frame.view().clone();
        // Wallpaper first (cover-fit fills every pixel), then the UI scrim +
        // field as an OVERLAY (LoadOp::Load) so it composites on top instead
        // of clearing the wallpaper away, then text on top of that.
        let cover = cover_rect(og.bg.width as f32, og.bg.height as f32, w as f32, h as f32);
        og.tex_pass.render_pass(
            &og.gpu,
            frame.encoder_mut(),
            &view,
            &[TextureDraw::new(&og.bg, cover.0, cover.1, cover.2, cover.3)],
            None,
        );
        og.painter
            .render_pass_overlay(&og.gpu, frame.encoder_mut(), &view);
        og.text.render_queued(&og.gpu, frame.encoder_mut(), &view);
        // Presenting attaches the buffer and commits the surface, which is
        // also what applies the ack and the buffer scale set above.
        frame.submit(&og.gpu.queue);

        out.dirty = false;
        out.retry_at = None;
        out.retry_delay = Duration::ZERO;
    }
    next_retry
}

/// Tear down outputs whose monitor went away. Without this an unplugged
/// screen keeps its whole GPU context and keeps being drawn to, and every
/// replug adds another.
pub(crate) fn reap_outputs(app: &mut App, gpus: &mut Gpus) {
    for global in std::mem::take(&mut app.removed) {
        if let Some(i) = app.outputs.iter().position(|o| o.global == global) {
            let out = app.outputs.remove(i);
            // The swapchain draws into the wl_surface: it has to go first.
            gpus.remove(&out.surface.id());
            out.lock_surface.destroy();
            out.surface.destroy();
        }
        if let Some(i) = app.discovered.iter().position(|(name, _)| *name == global) {
            let (_, output) = app.discovered.remove(i);
            app.output_scales.remove(&output.id());
            release_output(&output);
        }
    }
}

fn release_output(output: &wl_output::WlOutput) {
    // `release` arrived in wl_output v3; older ones just get forgotten.
    if output.version() >= 3 {
        output.release();
    }
}

/// The buffer scale to draw an output at: the one it asks for, stepped down
/// while that would make a buffer bigger than the GPU's largest texture. The
/// swapchain would be clamped to the limit there, and a lock surface whose
/// buffer doesn't match its configured size gets its client killed.
fn fit_scale(width: u32, height: u32, wanted: i32, max_side: u32) -> i32 {
    let longest = width.max(height) as u64;
    let mut scale = wanted.max(1);
    while scale > 1 && longest * scale as u64 > max_side as u64 {
        scale -= 1;
    }
    scale
}

/// Compute a cover-fit rect (fills the screen, preserving aspect, may crop).
fn cover_rect(iw: f32, ih: f32, sw: f32, sh: f32) -> (f32, f32, f32, f32) {
    if iw <= 0.0 || ih <= 0.0 {
        return (0.0, 0.0, sw, sh);
    }
    let scale = (sw / iw).max(sh / ih);
    let w = iw * scale;
    let h = ih * scale;
    ((sw - w) / 2.0, (sh - h) / 2.0, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_an_output_asks_for_is_used_when_it_fits() {
        assert_eq!(fit_scale(1920, 1200, 1, 8192), 1);
        assert_eq!(fit_scale(2560, 1440, 2, 8192), 2);
        assert_eq!(fit_scale(1920, 1080, 0, 8192), 1, "a bogus scale counts as 1");
    }

    #[test]
    fn a_buffer_past_the_texture_limit_steps_the_scale_down() {
        // 5120 wide at 2x would need a 10240 px buffer.
        assert_eq!(fit_scale(5120, 1440, 2, 8192), 1);
        assert_eq!(fit_scale(3000, 1000, 3, 8192), 2);
        // Tall monitors hit the limit on height.
        assert_eq!(fit_scale(1440, 5120, 2, 8192), 1);
    }

    #[test]
    fn cover_fit_fills_the_screen_and_crops_the_overflow() {
        // A square image on a wide screen: full width, cropped top and bottom.
        assert_eq!(cover_rect(100.0, 100.0, 200.0, 100.0), (0.0, -50.0, 200.0, 200.0));
        // A wide image on a square screen: full height, cropped at the sides.
        assert_eq!(cover_rect(200.0, 100.0, 100.0, 100.0), (-50.0, 0.0, 200.0, 100.0));
    }
}

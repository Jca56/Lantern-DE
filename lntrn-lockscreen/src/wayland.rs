use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, Sender};

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, GpuTexture, Painter, TextRenderer, TexturePass, TextureDraw};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_compositor, wl_output, wl_seat, wl_surface};
use wayland_client::{Connection, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    ext_session_lock_v1::ExtSessionLockV1,
};

use crate::keyboard::KeyboardState;
use crate::render::{self, Ui};

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
    pub output: wl_output::WlOutput,
    pub surface: wl_surface::WlSurface,
    pub lock_surface: ExtSessionLockSurfaceV1,
    pub scale: i32,
    pub width: u32,
    pub height: u32,
    pub pending_serial: Option<u32>,
    pub configured: bool,
    pub dirty: bool,
}

// ── GPU resources for one output (lives in the run loop, not in dispatch) ────

struct OutputGpu {
    gpu: GpuContext,
    painter: Painter,
    text: TextRenderer,
    tex_pass: TexturePass,
    bg: GpuTexture,
    buf_w: u32,
    buf_h: u32,
}

/// Decoded background image (RGBA8), shared and uploaded per-device.
pub(crate) struct BgImage {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

// ── Dispatch state ───────────────────────────────────────────────────────────

pub(crate) struct App {
    pub qh: QueueHandle<App>,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub seat: Option<wl_seat::WlSeat>,
    pub lock_mgr: Option<ExtSessionLockManagerV1>,
    pub lock: Option<ExtSessionLockV1>,
    pub outputs: Vec<OutputCtx>,
    pub discovered: Vec<wl_output::WlOutput>,
    pub output_scales: HashMap<ObjectId, i32>,
    pub keyboard: KeyboardState,
    pub key_queue: Vec<u32>,
    pub caps_lock: bool,
    pub locked: bool,
    pub finished: bool,
    pub running: bool,
    pub ui: Ui,
    pub password: String,
    pub username: String,
    pub auth_tx: Sender<bool>,
}

impl App {
    /// Build a wl_surface + lock_surface for every discovered output that
    /// doesn't have one yet. Safe to call repeatedly (handles hotplug).
    pub fn sync_outputs(&mut self) {
        let (Some(lock), Some(compositor)) = (self.lock.clone(), self.compositor.clone()) else {
            return;
        };
        let qh = self.qh.clone();
        for output in self.discovered.clone() {
            let oid = output.id();
            if self.outputs.iter().any(|o| o.output.id() == oid) {
                continue;
            }
            let surface = compositor.create_surface(&qh, ());
            let lock_surface = lock.get_lock_surface(&surface, &output, &qh, ());
            let scale = self.output_scales.get(&oid).copied().unwrap_or(1);
            self.outputs.push(OutputCtx {
                output,
                surface,
                lock_surface,
                scale,
                width: 0,
                height: 0,
                pending_serial: None,
                configured: false,
                dirty: false,
            });
        }
    }

    pub fn output_by_lock_surface(&mut self, lsid: &ObjectId) -> Option<&mut OutputCtx> {
        self.outputs.iter_mut().find(|o| &o.lock_surface.id() == lsid)
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run(bg: BgImage, style: crate::config::Style) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<App> = conn.new_event_queue();
    let qh = event_queue.handle();

    let (auth_tx, auth_rx): (Sender<bool>, Receiver<bool>) = std::sync::mpsc::channel();
    let username = crate::auth::current_username().unwrap_or_default();

    let mut app = App {
        qh: qh.clone(),
        compositor: None,
        seat: None,
        lock_mgr: None,
        lock: None,
        outputs: Vec::new(),
        discovered: Vec::new(),
        output_scales: HashMap::new(),
        keyboard: KeyboardState::new(),
        key_queue: Vec::new(),
        caps_lock: false,
        locked: false,
        finished: false,
        running: true,
        ui: Ui::new(),
        password: String::new(),
        username,
        auth_tx,
    };

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut app)?;

    let lock_mgr = app
        .lock_mgr
        .clone()
        .ok_or_else(|| anyhow!("compositor does not support ext-session-lock-v1"))?;
    app.compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;

    // Request the lock and create a surface per output.
    let lock = lock_mgr.lock(&qh, ());
    app.lock = Some(lock);
    app.sync_outputs();
    event_queue.roundtrip(&mut app)?;

    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let mut gpus: HashMap<ObjectId, OutputGpu> = HashMap::new();
    let mut auth_pending = false;
    let mut last_minute: i32 = -1;

    while app.running {
        // Drain wayland events (configure, input, locked/finished).
        event_queue.flush()?;
        if let Some(guard) = event_queue.prepare_read() {
            poll_fd(guard.connection_fd(), 500);
            let _ = guard.read();
        }
        event_queue.dispatch_pending(&mut app)?;

        // Pick up hotplugged outputs.
        app.sync_outputs();

        if app.finished && !app.locked {
            return Err(anyhow!("session lock was denied (another locker active?)"));
        }

        // Process queued key presses through the keymap.
        process_keys(&mut app, &mut auth_pending);

        // Receive async PAM result, if any.
        if auth_pending {
            if let Ok(ok) = auth_rx.try_recv() {
                auth_pending = false;
                app.ui.checking = false;
                if ok {
                    unlock_and_exit(&mut app, &conn);
                    return Ok(());
                } else {
                    app.ui.error = Some("Incorrect password".into());
                    app.ui.pw_len = 0;
                    zeroize(&mut app.password);
                    mark_all_dirty(&mut app);
                }
            }
        }

        // Force a redraw when the clock minute rolls over.
        let minute = current_minute();
        if minute != last_minute {
            last_minute = minute;
            mark_all_dirty(&mut app);
        }

        // (Re)create GPU contexts and render dirty outputs.
        render_outputs(&mut app, &qh, display_ptr, &bg, &style, &mut gpus);
    }

    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render_outputs(
    app: &mut App,
    qh: &QueueHandle<App>,
    display_ptr: *mut c_void,
    bg: &BgImage,
    style: &crate::config::Style,
    gpus: &mut HashMap<ObjectId, OutputGpu>,
) {
    for out in app.outputs.iter_mut() {
        if !out.configured || out.width == 0 || out.height == 0 {
            continue;
        }
        let buf_w = (out.width * out.scale.max(1) as u32).max(1);
        let buf_h = (out.height * out.scale.max(1) as u32).max(1);
        let sid = out.surface.id();

        // Acknowledge any pending configure before committing a buffer.
        if let Some(serial) = out.pending_serial.take() {
            out.lock_surface.ack_configure(serial);
            out.surface.set_buffer_scale(out.scale.max(1));
            out.dirty = true;
        }

        let entry = gpus.entry(sid.clone());
        let og = match entry {
            std::collections::hash_map::Entry::Occupied(e) => {
                let og = e.into_mut();
                if og.buf_w != buf_w || og.buf_h != buf_h {
                    og.gpu.resize(buf_w, buf_h);
                    og.buf_w = buf_w;
                    og.buf_h = buf_h;
                    out.dirty = true;
                }
                og
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                let surface_ptr = Proxy::id(&out.surface).as_ptr() as *mut c_void;
                let handle = WaylandHandle {
                    display: match NonNull::new(display_ptr) {
                        Some(p) => p,
                        None => continue,
                    },
                    surface: match NonNull::new(surface_ptr) {
                        Some(p) => p,
                        None => continue,
                    },
                };
                let gpu = match GpuContext::from_window(&handle, buf_w, buf_h) {
                    Ok(g) => g,
                    Err(err) => {
                        eprintln!("[lockscreen] GPU init failed: {err}");
                        continue;
                    }
                };
                let painter = Painter::new(&gpu);
                let text = TextRenderer::new(&gpu);
                let tex_pass = TexturePass::new(&gpu);
                let bg_tex = tex_pass.upload(&gpu, &bg.rgba, bg.w, bg.h);
                out.dirty = true;
                e.insert(OutputGpu {
                    gpu,
                    painter,
                    text,
                    tex_pass,
                    bg: bg_tex,
                    buf_w,
                    buf_h,
                })
            }
        };

        if !out.dirty {
            continue;
        }
        out.dirty = false;

        let w = og.buf_w as f32;
        let h = og.buf_h as f32;
        og.painter.clear();
        og.text.clear();

        app.ui.caps_lock = app.caps_lock;
        render::draw(&mut og.painter, &mut og.text, &app.ui, style, w, h, og.buf_w, og.buf_h);

        // Background image is drawn first (cover-fit), then the painter/text on top.
        let cover = cover_rect(bg.w as f32, bg.h as f32, w, h);
        if let Ok(mut frame) = og.gpu.begin_frame("lockscreen") {
            let view = frame.view().clone();
            // Wallpaper first (cover-fit fills every pixel), then the UI scrim +
            // field as an OVERLAY (LoadOp::Load) so it composites on top instead
            // of clearing the wallpaper away, then text on top of that.
            og.tex_pass.render_pass(
                &og.gpu,
                frame.encoder_mut(),
                &view,
                &[TextureDraw::new(&og.bg, cover.0, cover.1, cover.2, cover.3)],
                None,
            );
            og.painter.render_pass_overlay(&og.gpu, frame.encoder_mut(), &view);
            og.text.render_queued(&og.gpu, frame.encoder_mut(), &view);
            frame.submit(&og.gpu.queue);
        }
        out.surface.frame(qh, ());
        out.surface.commit();
    }
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

// ── Input handling ─────────────────────────────────────────────────────────────

fn process_keys(app: &mut App, auth_pending: &mut bool) {
    if app.key_queue.is_empty() {
        return;
    }
    let keys: Vec<u32> = std::mem::take(&mut app.key_queue);
    let mut changed = false;
    for keycode in keys {
        if app.ui.checking {
            continue;
        }
        let sym = app.keyboard.key_get_sym(keycode);
        let raw = sym.raw();
        const ENTER: u32 = 0xff0d; // XKB_KEY_Return
        const KP_ENTER: u32 = 0xff8d;
        const BACKSPACE: u32 = 0xff08;
        const ESCAPE: u32 = 0xff1b;
        match raw {
            ENTER | KP_ENTER => {
                if !app.password.is_empty() {
                    app.ui.checking = true;
                    app.ui.error = None;
                    *auth_pending = true;
                    spawn_auth(app);
                    changed = true;
                }
            }
            BACKSPACE => {
                app.password.pop();
                app.ui.pw_len = app.password.chars().count();
                app.ui.error = None;
                changed = true;
            }
            ESCAPE => {
                zeroize(&mut app.password);
                app.ui.pw_len = 0;
                app.ui.error = None;
                changed = true;
            }
            _ => {
                if let Some(s) = app.keyboard.key_to_utf8(keycode) {
                    app.password.push_str(&s);
                    app.ui.pw_len = app.password.chars().count();
                    app.ui.error = None;
                    changed = true;
                }
            }
        }
    }
    if changed {
        mark_all_dirty(app);
    }
}

fn spawn_auth(app: &App) {
    let tx = app.auth_tx.clone();
    let user = app.username.clone();
    let pass = app.password.clone();
    std::thread::spawn(move || {
        let ok = crate::auth::verify(&user, &pass);
        let _ = tx.send(ok);
    });
}

fn unlock_and_exit(app: &mut App, conn: &Connection) {
    for out in app.outputs.iter_mut() {
        out.lock_surface.destroy();
    }
    if let Some(lock) = app.lock.take() {
        lock.unlock_and_destroy();
    }
    let _ = conn.flush();
    zeroize(&mut app.password);
    app.running = false;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn mark_all_dirty(app: &mut App) {
    for out in app.outputs.iter_mut() {
        out.dirty = true;
    }
}

fn zeroize(s: &mut String) {
    unsafe {
        for b in s.as_bytes_mut() {
            *b = 0;
        }
    }
    s.clear();
}

fn current_minute() -> i32 {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_hour * 60 + tm.tm_min
    }
}

/// Poll a borrowed fd for readability with a timeout (milliseconds).
fn poll_fd(fd: std::os::fd::BorrowedFd<'_>, timeout_ms: i32) {
    use std::os::fd::AsRawFd;
    let mut pfd = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        libc::poll(&mut pfd, 1, timeout_ms);
    }
}

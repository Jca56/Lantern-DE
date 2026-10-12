use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_seat},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1;
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::xdg_wm_base;

use crate::event_pump::{pump, Waker};
use crate::keyboard::Keyboard;
use crate::mpris_server::{MprisCmd, PlayerState};
use crate::session::Session;
use crate::shutdown;
use crate::Gpu;

// ── WaylandHandle for wgpu ──────────────────────────────────────────────────

struct WaylandHandle {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}
impl HasDisplayHandle for WaylandHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
impl HasWindowHandle for WaylandHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(self.surface));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

// ── Wayland state ───────────────────────────────────────────────────────────
// Fields are pub(crate): the Dispatch impls that fill them live in
// wayland_dispatch.rs; session.rs and input.rs read the input they gather.

pub(crate) struct State {
    pub(crate) running: bool,
    pub(crate) configured: bool,
    /// Something on screen changed: draw when the compositor is ready.
    pub(crate) needs_redraw: bool,
    /// Our last frame's callback hasn't fired: drawing again before the
    /// compositor shows it would only be thrown away.
    pub(crate) frame_pending: bool,
    pub(crate) frame_requested_at: Instant,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Integer wl_output scale: only a fallback until the fractional arrives.
    pub(crate) scale: i32,
    /// Preferred fractional scale from the compositor, numerator over 120
    /// (e.g. 168 → 1.4×). 0 until the first `preferred_scale` event.
    pub(crate) frac_scale_120: u32,
    pub(crate) maximized: bool,
    pub(crate) fullscreen: bool,
    // Wayland objects
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) frac_scale_mgr: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    // Input
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) keyboard: Keyboard,
    // Cursor shape
    pub(crate) cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub(crate) cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub(crate) pointer_enter_serial: u32,
    pub(crate) current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            needs_redraw: true,
            frame_pending: false,
            frame_requested_at: Instant::now(),
            width: 0,
            height: 0,
            scale: 1,
            frac_scale_120: 0,
            maximized: false,
            fullscreen: false,
            compositor: None,
            wm_base: None,
            viewporter: None,
            frac_scale_mgr: None,
            seat: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            left_pressed: false,
            left_released: false,
            scroll_delta: 0.0,
            pointer_serial: 0,
            keyboard: Keyboard::new(),
            cursor_shape_mgr: None,
            cursor_shape_device: None,
            pointer_enter_serial: 0,
            current_cursor_shape: None,
        }
    }

    /// The compositor's per-surface fractional scale: correct for any window
    /// size, and it follows the surface across monitors.
    pub(crate) fn fractional_scale(&self) -> f64 {
        if self.frac_scale_120 > 0 {
            self.frac_scale_120 as f64 / 120.0
        } else {
            self.scale.max(1) as f64
        }
    }

    fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// While a frame callback is outstanding, wait at most this long before
/// drawing anyway, so a lost callback can't freeze the window. (A hidden
/// window never gets one: this is also how often it redraws at most.)
const FRAME_STALL: Duration = Duration::from_millis(250);
/// The loop wakes at least this often even with nothing to do: the watchdog
/// counts its beats, and a deadline computed wrong can't park it forever.
const MAX_SLEEP: Duration = Duration::from_secs(1);
/// …and never sleeps shorter than this unless a frame is owed, so a deadline
/// stuck in the past can't turn the loop into a busy spin.
const MIN_SLEEP: Duration = Duration::from_millis(1);

pub fn run(
    initial_path: Option<PathBuf>,
    waker: Waker,
    mpris_tx: Sender<PlayerState>,
    mpris_rx: Receiver<MprisCmd>,
) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state
        .wm_base
        .clone()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    state.width = 960;
    state.height = 540;

    let surface = compositor.create_surface(&qh, ());
    // Ask the compositor for this surface's fractional scale. Kept alive for
    // the surface's lifetime so it keeps sending updates.
    let _frac_scale = state
        .frac_scale_mgr
        .as_ref()
        .map(|mgr| mgr.get_fractional_scale(&surface, &qh, ()));
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Media Player".into());
    toplevel.set_app_id("lntrn-media-player".into());
    // min == max before the first configure asks the compositor for this
    // exact startup size; relaxed to a plain floor right after.
    toplevel.set_min_size(state.width as i32, state.height as i32);
    toplevel.set_max_size(state.width as i32, state.height as i32);
    surface.commit();

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    // Max 0,0 = unlimited per the xdg-shell protocol.
    toplevel.set_min_size(480, 320);
    toplevel.set_max_size(0, 0);

    surface.set_buffer_scale(1);
    let viewport = state
        .viewporter
        .as_ref()
        .map(|vp| vp.get_viewport(&surface, &qh, ()));

    // wgpu setup
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };
    let gpu_ctx = GpuContext::from_window(&wl_handle, state.phys_width().max(1), state.phys_height().max(1))
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut gpu = Gpu {
        painter: Painter::new(&gpu_ctx),
        text: TextRenderer::new(&gpu_ctx),
        tex_pass: TexturePass::new(&gpu_ctx),
        ctx: gpu_ctx,
    };

    let mut session = Session::new(initial_path.as_deref(), waker.clone(), mpris_tx, mpris_rx);
    // Physical size the swapchain was last configured for (forces the first).
    let mut surface_size = (0, 0);

    // Dev/test hook: close on our own after N ms so the close path can be
    // exercised from a shell without a compositor click.
    let autoclose = std::env::var("LNTRN_MEDIA_PLAYER_AUTOCLOSE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    let watchdog = shutdown::Watchdog::start();

    while state.running {
        watchdog.beat();
        let now = Instant::now();
        if autoclose.is_some_and(|at| now >= at) {
            break;
        }

        // Sleep until an event arrives, another thread pokes the waker (a
        // video frame, a spectrum, a bus message, an MPRIS command), a timer
        // is due — or, with a frame owed and the compositor ready for it,
        // not at all. A paused or empty player draws nothing.
        let timeout = if state.needs_redraw && can_draw(&state, now) {
            Duration::ZERO
        } else {
            let mut wake = sooner(session.next_deadline(&state, now), autoclose);
            if state.needs_redraw {
                wake = sooner(wake, Some(state.frame_requested_at + FRAME_STALL));
            }
            wake.map_or(MAX_SLEEP, |at| at.saturating_duration_since(now))
                .clamp(MIN_SLEEP, MAX_SLEEP)
        };
        if let Err(e) = pump(&mut event_queue, &mut state, &waker, timeout) {
            eprintln!("[media-player] dispatch error: {e}");
            break;
        }

        // Follow configures and scale changes (a late preferred_scale, the
        // window moving to another monitor).
        let target = (state.phys_width().max(1), state.phys_height().max(1));
        if state.configured || target != surface_size {
            state.configured = false;
            surface_size = target;
            gpu.ctx.resize(target.0, target.1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            xdg_surface.set_window_geometry(0, 0, state.width as i32, state.height as i32);
            state.needs_redraw = true;
        }

        // Playback upkeep, MPRIS and input run on every wake-up — minimized
        // windows get no frame callbacks, and none of this may wait for one.
        session.on_wake(&mut state, &toplevel, &gpu);

        if state.needs_redraw && can_draw(&state, Instant::now()) {
            state.needs_redraw = false;
            session.render(&state, &mut gpu, &toplevel);
            // The callback only throttles the next draw; it never asks for
            // one, so an idle window stays idle.
            surface.frame(&qh, ());
            state.frame_pending = true;
            state.frame_requested_at = Instant::now();
            surface.commit();
            if session.is_animating() {
                state.needs_redraw = true;
            }
        }
    }

    // ── Close ─────────────────────────────────────────────────────────────
    // Committed to exiting from here. Every step is bounded and the backstop
    // ends the process regardless — that is what keeps a wedged audio sink
    // from leaving an invisible zombie holding the stream.
    shutdown::exit_after(shutdown::EXIT_TIMEOUT);

    session.save_position();

    // Take the window down first so it disappears immediately (role objects
    // before the wl_surface, per xdg-shell); the wl_surface itself goes with
    // the connection when the process exits.
    toplevel.destroy();
    xdg_surface.destroy();
    let _ = conn.flush();

    session.drop_pipeline();

    // Exit here on purpose instead of returning into the wgpu/Wayland
    // destructors: NVIDIA's Wayland teardown is one more thing that can block
    // on a compositor that has already forgotten us.
    std::process::exit(0)
}

fn can_draw(state: &State, now: Instant) -> bool {
    !state.frame_pending || now >= state.frame_requested_at + FRAME_STALL
}

fn sooner(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

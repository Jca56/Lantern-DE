use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_data_device_manager, wl_data_offer, wl_output, wl_seat},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::xdg_wm_base;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1;

use crate::keyboard::Keyboard;
use crate::session::{Session, Window};
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
// wayland_dispatch.rs (protocol plumbing), dnd.rs (drag-and-drop) and
// clipboard.rs; session.rs reads the input they gather.

/// One monitor, as far as the startup window size cares.
pub(crate) struct OutputInfo {
    pub(crate) output: wl_output::WlOutput,
    /// Current mode, physical px.
    pub(crate) phys: (i32, i32),
    /// Size in logical px (scaling applied), from xdg-output.
    pub(crate) logical: Option<(i32, i32)>,
}

pub(crate) struct State {
    pub(crate) running: bool,
    /// Compositor asked us to close (xdg_toplevel.close) — the session turns
    /// this into a confirm dialog when the canvas has unsaved changes.
    pub(crate) close_requested: bool,
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
    /// The compositor's preferred scale for our surface, ×120 (0 = not yet).
    pub(crate) preferred_scale_120: u32,
    pub(crate) outputs: Vec<OutputInfo>,
    pub(crate) maximized: bool,
    // Wayland objects
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) frac_scale_mgr: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    pub(crate) xdg_output_mgr: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    pub(crate) cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub(crate) cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub(crate) current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    // Input
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) middle_pressed: bool,
    pub(crate) middle_released: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) enter_serial: u32,
    pub(crate) keyboard: Keyboard,
    /// Serial of the latest key press — clipboard set_selection needs a recent one.
    pub(crate) key_serial: u32,
    // Drag-and-drop receive (Dispatch impls live in dnd.rs)
    pub(crate) data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    /// Every offer the compositor has introduced, with its mime types, until
    /// it's used or replaced (selection offers are dropped straight away).
    pub(crate) dnd_offers: Vec<(wl_data_offer::WlDataOffer, Vec<String>)>,
    /// The offer of the drag currently over the window.
    pub(crate) dnd_offer: Option<wl_data_offer::WlDataOffer>,
    /// Drop cursor position in logical surface coords (× scale before use).
    pub(crate) dnd_x: f64,
    pub(crate) dnd_y: f64,
    /// True while a drop's pipe is being read on a worker thread — keeps the
    /// event loop polling so the result lands promptly.
    pub(crate) dnd_reading: bool,
    pub(crate) dnd_tx: mpsc::Sender<Vec<PathBuf>>,
}

impl State {
    fn new(dnd_tx: mpsc::Sender<Vec<PathBuf>>) -> Self {
        Self {
            running: true,
            close_requested: false,
            configured: false,
            needs_redraw: true,
            frame_pending: false,
            frame_requested_at: Instant::now(),
            width: 0,
            height: 0,
            scale: 1,
            preferred_scale_120: 0,
            outputs: Vec::new(),
            maximized: false,
            compositor: None,
            wm_base: None,
            viewporter: None,
            frac_scale_mgr: None,
            xdg_output_mgr: None,
            seat: None,
            cursor_shape_mgr: None,
            cursor_shape_device: None,
            current_cursor_shape: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            left_pressed: false,
            left_released: false,
            middle_pressed: false,
            middle_released: false,
            scroll_delta: 0.0,
            pointer_serial: 0,
            enter_serial: 0,
            keyboard: Keyboard::new(),
            key_serial: 0,
            data_device_manager: None,
            dnd_offers: Vec::new(),
            dnd_offer: None,
            dnd_x: 0.0,
            dnd_y: 0.0,
            dnd_reading: false,
            dnd_tx,
        }
    }

    /// The compositor's per-surface fractional scale: right for any window
    /// size and on whichever monitor the window is.
    pub(crate) fn fractional_scale(&self) -> f64 {
        if self.preferred_scale_120 > 0 {
            self.preferred_scale_120 as f64 / 120.0
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

    /// Logical size of the smallest monitor, to cap the startup window:
    /// xdg-output's figure when the compositor gives one, else the mode
    /// over the scale we know.
    fn smallest_screen(&self) -> (u32, u32) {
        let scale = self.fractional_scale();
        self.outputs
            .iter()
            .filter_map(|o| {
                o.logical.or_else(|| {
                    (o.phys.0 > 0).then(|| {
                        ((o.phys.0 as f64 / scale) as i32, (o.phys.1 as f64 / scale) as i32)
                    })
                })
            })
            .filter(|&(w, h)| w > 0 && h > 0)
            .min_by_key(|&(w, h)| w as i64 * h as i64)
            .map(|(w, h)| (w as u32, h as u32))
            .unwrap_or((1920, 1080))
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Smallest window, logical px: the floor for resizing and for the startup size.
const MIN_WINDOW: (u32, u32) = (400, 300);
/// While a frame callback is outstanding, wait at most this long before
/// drawing anyway, so a lost callback can't freeze the window.
const FRAME_STALL: Duration = Duration::from_millis(250);
/// Wake-up cadence while background work (thumbnails, a drop being read, a
/// GIF frame still decoding) finishes without telling the wayland socket.
const BUSY_POLL: Duration = Duration::from_millis(16);

pub fn run(initial_path: Option<PathBuf>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let (dnd_tx, dnd_rx) = mpsc::channel::<Vec<PathBuf>>();
    let mut state = State::new(dnd_tx);

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

    // Logical monitor sizes (xdg-output) and our surface's fractional scale
    // — the compositor answers with the scale of the monitor under the
    // pointer, where the window will open — before sizing the window.
    if let Some(mgr) = &state.xdg_output_mgr {
        for (i, o) in state.outputs.iter().enumerate() {
            mgr.get_xdg_output(&o.output, &qh, i);
        }
    }
    let surface = compositor.create_surface(&qh, ());
    let _frac_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1> = state
        .frac_scale_mgr
        .as_ref()
        .map(|mgr| mgr.get_fractional_scale(&surface, &qh, ()));
    event_queue.roundtrip(&mut state)?;

    // Start at the image's own size, capped to ~85% of the screen while
    // preserving aspect ratio. 960×640 without an image (or a readable header).
    let (screen_w, screen_h) = state.smallest_screen();
    let (init_w, init_h) = initial_path
        .as_deref()
        .and_then(crate::loaders::peek_image_dimensions)
        .map(|(w, h)| fit_to_screen(w, h, screen_w, screen_h))
        // A banner-shaped image mustn't open a window too flat for its own
        // title and status bars.
        .map(|(w, h)| (w.max(MIN_WINDOW.0), h.max(MIN_WINDOW.1)))
        .unwrap_or((960, 640));
    if state.width == 0 {
        state.width = init_w;
    }
    if state.height == 0 {
        state.height = init_h;
    }

    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Image Viewer".into());
    toplevel.set_app_id("lntrn-image-viewer".into());
    // The compositor reads a min == max declaration before the first configure
    // as an exact startup-size request (a bare min would only clamp its default
    // suggestion). Relaxed right after the first configure so the user can
    // still resize the window freely.
    toplevel.set_min_size(init_w as i32, init_h as i32);
    toplevel.set_max_size(init_w as i32, init_h as i32);
    surface.commit();

    // DnD target + clipboard source: a data device on our seat.
    let data_device = match (&state.data_device_manager, &state.seat) {
        (Some(mgr), Some(seat)) => Some(mgr.get_data_device(seat, &qh, ())),
        _ => None,
    };

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    // Max 0,0 = unlimited per the xdg-shell protocol.
    toplevel.set_min_size(MIN_WINDOW.0 as i32, MIN_WINDOW.1 as i32);
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

    let mut session = Session::new(initial_path.as_deref(), &gpu);
    let win = Window {
        toplevel: &toplevel,
        data_device: data_device.as_ref(),
        qh: &qh,
    };
    // Physical size the swapchain was last configured for (forces the first).
    let mut surface_size = (0, 0);

    while state.running {
        // Sleep until an event arrives, a timer is due, or — with a frame
        // owed and the compositor ready for it — not at all. An untouched
        // window draws nothing and wakes only for its timers.
        let now = Instant::now();
        let timeout = if state.needs_redraw && can_draw(&state, now) {
            Some(Duration::ZERO)
        } else {
            let mut wake = session.next_deadline(&state, now);
            if state.needs_redraw {
                wake = sooner(wake, Some(state.frame_requested_at + FRAME_STALL));
            }
            if session.is_busy(&state, now) {
                wake = sooner(wake, Some(now + BUSY_POLL));
            }
            wake.map(|at| at.saturating_duration_since(now))
        };
        if let Err(e) = crate::event_pump::pump(&mut event_queue, &mut state, timeout) {
            eprintln!("[image-viewer] dispatch error: {e}");
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
            state.needs_redraw = true;
        }

        // Input, timers, drops and close requests run on every wake-up —
        // minimized windows get no frame callbacks, and none of this may
        // wait for one.
        session.on_wake(&mut state, &win, &gpu, &dnd_rx);

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

    Ok(())
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

/// Cap (img_w, img_h) to 85% of the screen while preserving aspect ratio.
/// Returns native size for images that already fit.
fn fit_to_screen(img_w: u32, img_h: u32, screen_w: u32, screen_h: u32) -> (u32, u32) {
    let max_w = (screen_w as f32 * 0.85).max(320.0);
    let max_h = (screen_h as f32 * 0.85).max(240.0);
    let iw = img_w.max(1) as f32;
    let ih = img_h.max(1) as f32;
    if iw <= max_w && ih <= max_h {
        return (img_w.max(1), img_h.max(1));
    }
    let s = (max_w / iw).min(max_h / ih);
    ((iw * s).round() as u32, (ih * s).round() as u32)
}

/// `.lcanvas` files open in the collage editor instead of the viewer.
pub(crate) fn is_canvas_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lcanvas"))
}

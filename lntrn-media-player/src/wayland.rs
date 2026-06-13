use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::ptr::NonNull;
use std::time::Duration;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, Rect, TextRenderer, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    backend::WaylandError,
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::app::App;
use crate::mpris_server::{PlayerState, MprisCmd};
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_FULLSCREEN, ZONE_LOOP, ZONE_MAXIMIZE, ZONE_MINIMIZE,
    ZONE_NEXT, ZONE_PLAY_PAUSE, ZONE_PREV, ZONE_SEEK_BAR, ZONE_TITLE_BAR, ZONE_VIEW_MENU,
    ZONE_VIEW_SWATCH_BASE,
};

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

const BTN_LEFT: u32 = 0x110;

struct State {
    running: bool,
    configured: bool,
    frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    /// Preferred fractional scale from the compositor, numerator over 120
    /// (e.g. 168 → 1.4×). 0 until the first `preferred_scale` event.
    frac_scale_120: u32,
    maximized: bool,
    fullscreen: bool,
    // Wayland objects
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    frac_scale_mgr: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    // Kept alive for the surface's lifetime so the compositor keeps sending
    // us scale updates; we never call methods on it directly.
    _frac_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    seat: Option<wl_seat::WlSeat>,
    // Input
    cursor_x: f64,
    cursor_y: f64,
    pointer_in_surface: bool,
    left_pressed: bool,
    left_released: bool,
    scroll_delta: f32,
    pointer_serial: u32,
    // Keyboard
    ctrl: bool,
    key_pressed: Option<u32>,
    // Cursor shape
    cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pointer_enter_serial: u32,
    current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0,
            frac_scale_120: 0,
            maximized: false, fullscreen: false,
            compositor: None, wm_base: None, viewporter: None,
            frac_scale_mgr: None, _frac_scale: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false,
            scroll_delta: 0.0, pointer_serial: 0,
            ctrl: false, key_pressed: None,
            cursor_shape_mgr: None, cursor_shape_device: None,
            pointer_enter_serial: 0, current_cursor_shape: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        // Preferred: the compositor's per-surface fractional scale. It's
        // correct for ANY window size and follows the surface across monitors
        // — unlike dividing the output's physical width by the window width,
        // which is only right when the window happens to fill the output.
        if self.frac_scale_120 > 0 {
            self.frac_scale_120 as f64 / 120.0
        } else if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    fn phys_width(&self) -> u32 { (self.width as f64 * self.fractional_scale()).round() as u32 }
    fn phys_height(&self) -> u32 { (self.height as f64 * self.fractional_scale()).round() as u32 }
}

// ── Dispatch impls ──────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self, registry: &wl_registry::WlRegistry,
        event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => { state.compositor = Some(registry.bind(name, version.min(6), qh, ())); }
                "xdg_wm_base" => { state.wm_base = Some(registry.bind(name, version.min(5), qh, ())); }
                "wp_viewporter" => { state.viewporter = Some(registry.bind(name, version.min(1), qh, ())); }
                "wp_fractional_scale_manager_v1" => {
                    state.frac_scale_mgr = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => { let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ()); }
                "wl_seat" => { state.seat = Some(registry.bind(name, version.min(9), qh, ())); }
                "wp_cursor_shape_manager_v1" => {
                    state.cursor_shape_mgr = Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for State {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for State {
    fn event(_: &mut Self, _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _: wp_fractional_scale_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for State {
    fn event(
        state: &mut Self, _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.frac_scale_120 = scale;
            // Wake the loop so the surface re-renders (and resizes) at the
            // new scale even when paused/idle.
            state.frame_done = true;
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self, wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event { wm_base.pong(serial); }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self, xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
            state.frame_done = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self, _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                if width > 0 { state.width = width as u32; }
                if height > 0 { state.height = height as u32; }
                state.maximized = states.chunks_exact(4).any(|chunk| {
                    let val = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    val == xdg_toplevel::State::Maximized as u32
                });
                state.fullscreen = states.chunks_exact(4).any(|chunk| {
                    let val = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    val == xdg_toplevel::State::Fullscreen as u32
                });
            }
            xdg_toplevel::Event::Close => { state.running = false; }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self, _: &wl_output::WlOutput,
        event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => { state.scale = factor; }
            wl_output::Event::Mode { width, .. } => { state.output_phys_width = width as u32; }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(state: &mut Self, _: &wl_callback::WlCallback, _: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        state.frame_done = true;
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self, seat: &wl_seat::WlSeat,
        event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(cap) } = event {
            if cap.contains(wl_seat::Capability::Pointer) {
                let ptr = seat.get_pointer(qh, ());
                if let Some(mgr) = &state.cursor_shape_mgr {
                    state.cursor_shape_device = Some(mgr.get_pointer(&ptr, qh, ()));
                }
            }
            if cap.contains(wl_seat::Capability::Keyboard) { seat.get_keyboard(qh, ()); }
        }
    }
}

impl Dispatch<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, ()> for State {
    fn event(_: &mut Self, _: &wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, _: wp_cursor_shape_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, ()> for State {
    fn event(_: &mut Self, _: &wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, _: wp_cursor_shape_device_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self, _: &wl_pointer::WlPointer,
        event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { serial, surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.pointer_enter_serial = serial;
                state.current_cursor_shape = None;
                state.frame_done = true;
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_in_surface = false;
                state.frame_done = true;
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.frame_done = true;
            }
            wl_pointer::Event::Button { button, state: btn_state, serial, .. } => {
                state.pointer_serial = serial;
                let pressed = btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                let released = btn_state == WEnum::Value(wl_pointer::ButtonState::Released);
                if button == BTN_LEFT && pressed { state.left_pressed = true; }
                if button == BTN_LEFT && released { state.left_released = true; }
                state.frame_done = true;
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                if axis == WEnum::Value(wl_pointer::Axis::VerticalScroll) {
                    state.scroll_delta += value as f32;
                }
                state.frame_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self, _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.key_pressed = Some(key);
                }
                state.frame_done = true;
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.ctrl = mods_depressed & 4 != 0;
            }
            _ => {}
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run(
    initial_path: Option<String>,
    mpris_tx: std::sync::mpsc::Sender<PlayerState>,
    mpris_rx: std::sync::mpsc::Receiver<MprisCmd>,
) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state.wm_base.as_ref()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    if state.width == 0 { state.width = 960; }
    if state.height == 0 { state.height = 540; }

    let surface = compositor.create_surface(&qh, ());
    // Ask the compositor for this surface's fractional scale. Robust across
    // window sizes and monitors — unlike deriving it from the output mode.
    let frac_scale = state.frac_scale_mgr.as_ref()
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

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());
    state._frac_scale = frac_scale;

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    // Relax the exact-size declaration so the user can resize freely
    // (max 0,0 = unlimited per the xdg-shell protocol).
    toplevel.set_min_size(480, 320);
    toplevel.set_max_size(0, 0);

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

    if let Some(xs) = &state.xdg_surface {
        xs.set_window_geometry(0, 0, state.width as i32, state.height as i32);
    }

    // wgpu setup
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let phys_w = state.phys_width().max(1);
    let phys_h = state.phys_height().max(1);
    let gpu_ctx = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut gpu = Gpu {
        painter: Painter::new(&gpu_ctx),
        text: TextRenderer::new(&gpu_ctx),
        tex_pass: TexturePass::new(&gpu_ctx),
        ctx: gpu_ctx,
    };

    let mut app = App::new();
    let mut input = InteractionContext::new();

    // Load initial media if provided
    if let Some(path) = initial_path {
        app.open_file(&path);
        update_title(&toplevel, &app);
    }

    // Control rect caches (set each frame in render, read in input handling)
    let mut rects = crate::render::ControlRects {
        seek: Rect::new(0.0, 0.0, 0.0, 0.0),
    };

    // MPRIS push throttle: don't clone+send a PlayerState every frame (that was
    // 60 String allocations/sec into the channel). Push at 4 Hz to keep position
    // fresh for clients, and flush immediately on play/title/volume changes.
    let mut last_mpris_send = std::time::Instant::now();
    let mut mpris_prev_playing = false;
    let mut mpris_prev_title = String::new();
    let mut mpris_prev_volume = app.volume;

    while state.running {
        // Pump Wayland events with a bounded wait — never block forever. When
        // playing we pace to the frame interval (audio-only caps at ~30fps,
        // video ~60fps); when paused we idle longer. Either way the timeout
        // guarantees the loop keeps turning even when the window is minimized
        // and the compositor has stopped sending frame callbacks — that's what
        // lets MPRIS transport commands below still get serviced while audio
        // keeps playing on GStreamer's own thread.
        let pump_timeout = if app.is_playing() {
            Duration::from_millis(if app.audio_only { 33 } else { 16 })
        } else {
            Duration::from_millis(100)
        };
        if let Err(e) = pump_events(&mut event_queue, &mut state, pump_timeout) {
            eprintln!("[media-player] dispatch error: {e}");
            break;
        }

        app.tick(&gpu.ctx, &gpu.tex_pass);
        if app.check_eos() { update_title(&toplevel, &app); }

        // ── MPRIS commands ──────────────────────────────────────────────
        // Drained every iteration, BEFORE the frame-callback render gate
        // below, so play/pause/next/prev from the Command Center (or media
        // keys) keep working even while the window is minimized — when no
        // frame callbacks arrive and the gate would otherwise `continue`.
        while let Ok(cmd) = mpris_rx.try_recv() {
            match cmd {
                MprisCmd::PlayPause => app.toggle_play_pause(),
                MprisCmd::Play => { if let Some(p) = &app.pipeline { p.play(); } }
                MprisCmd::Pause => { if let Some(p) = &app.pipeline { p.pause(); } }
                MprisCmd::Stop => { if let Some(p) = &app.pipeline { p.pause(); } }
                MprisCmd::Next => { app.next_track(); update_title(&toplevel, &app); }
                MprisCmd::Previous => { app.prev_track(); update_title(&toplevel, &app); }
                MprisCmd::SetVolume(v) => {
                    app.volume = v;
                    if let Some(p) = &app.pipeline { p.set_volume(v); }
                }
                MprisCmd::Seek(offset_us) => {
                    app.seek_relative(offset_us * 1000); // us → ns
                }
            }
        }
        // Send state to MPRIS server (throttled — see notes above the loop).
        let playing = app.is_playing();
        let mpris_dirty = playing != mpris_prev_playing
            || app.file_name != mpris_prev_title
            || (app.volume - mpris_prev_volume).abs() > 0.001;
        if mpris_dirty || last_mpris_send.elapsed() >= Duration::from_millis(250) {
            last_mpris_send = std::time::Instant::now();
            mpris_prev_playing = playing;
            mpris_prev_title = app.file_name.clone();
            mpris_prev_volume = app.volume;
            let _ = mpris_tx.send(PlayerState {
                title: app.file_name.clone(),
                file_path: app.file_path.as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                playing,
                position_ns: app.position_ns,
                duration_ns: app.duration_ns,
                volume: app.volume,
            });
        }

        // ── Render gate ── only paint when the compositor asked for a frame.
        // Minimized windows get no frame callbacks, so we stop here (MPRIS and
        // playback above already ran). Input events flip `frame_done` too, so
        // interaction still drives a repaint.
        if !state.frame_done { continue; }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;

        // Handle resize — on an explicit reconfigure, OR when the fractional
        // scale changed the physical surface size out from under us (a late
        // `preferred_scale` arriving after the first frame).
        let target_w = state.phys_width().max(1);
        let target_h = state.phys_height().max(1);
        if state.configured || gpu.ctx.width() != target_w || gpu.ctx.height() != target_h {
            state.configured = false;
            gpu.ctx.resize(target_w, target_h);
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            if let Some(xs) = &state.xdg_surface {
                xs.set_window_geometry(0, 0, state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.ctx.width() as f32;
        let win_hf = state.phys_height() as f32;
        let s = scale_f;

        // ── Cursor ──────────────────────────────────────────────────────
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        // Controls own the bottom edge while visible; the title bar (video
        // mode, not fullscreen) owns the top edge.
        let controls_visible = app.controls_alpha > 0.05;
        // Title bar owns the top edge in video mode (always) and in audio mode
        // whenever it's faded in on hover.
        let video_mode = !app.audio_only && app.pipeline.is_some();
        let title_visible = !state.fullscreen
            && (video_mode || (app.audio_only && controls_visible));

        // ── Cursor shape (resize edges) ────────────────────────────────
        if state.pointer_in_surface {
            let desired = if !state.maximized && !state.fullscreen {
                let border = 10.0 * s;
                match edge_resize(cx, cy, wf, win_hf, border, controls_visible, title_visible) {
                    Some(edge) => resize_edge_to_cursor_shape(edge),
                    None => wp_cursor_shape_device_v1::Shape::Default,
                }
            } else {
                wp_cursor_shape_device_v1::Shape::Default
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.pointer_enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Hover-reveal controls ────────────────────────────────────
        app.pointer_in_window = state.pointer_in_surface;
        if state.pointer_in_surface {
            app.note_pointer_activity();
        }
        app.update_controls_alpha();

        // ── Seek bar drag (motion) ─────────────────────────────────────
        if app.seeking && state.pointer_in_surface && rects.seek.w > 0.0 {
            let frac = ((cx - rects.seek.x) / rects.seek.w).clamp(0.0, 1.0);
            app.seek_value = frac;
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            handle_key(&mut app, &toplevel, &mut state, key);
        }

        // ── Scroll → volume ─────────────────────────────────────────────
        if state.scroll_delta.abs() > 0.01 {
            let delta = if state.scroll_delta < 0.0 { 0.05 } else { -0.05 };
            app.adjust_volume(delta);
            state.scroll_delta = 0.0;
        }

        // ── Left press ──────────────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            let border = 10.0 * s;
            let resize_edge = if !state.maximized && !state.fullscreen {
                edge_resize(cx, cy, wf, win_hf, border, controls_visible, title_visible)
            } else {
                None
            };
            if let Some(edge) = resize_edge {
                if let Some(seat) = &state.seat {
                    toplevel.resize(seat, state.pointer_serial, edge);
                }
            } else if let Some(zone_id) = input.on_left_pressed() {
                // Any click outside the View menu/button closes it.
                let is_view = zone_id == ZONE_VIEW_MENU
                    || (zone_id >= ZONE_VIEW_SWATCH_BASE
                        && zone_id < ZONE_VIEW_SWATCH_BASE + crate::vis_theme::theme_count() as u32);
                if app.view_menu_open && !is_view {
                    app.view_menu_open = false;
                }
                match zone_id {
                    ZONE_PLAY_PAUSE => { app.toggle_play_pause(); }
                    ZONE_VIEW_MENU => { app.view_menu_open = !app.view_menu_open; }
                    z if z >= ZONE_VIEW_SWATCH_BASE
                        && z < ZONE_VIEW_SWATCH_BASE + crate::vis_theme::theme_count() as u32 =>
                    {
                        app.set_vis_theme((z - ZONE_VIEW_SWATCH_BASE) as usize);
                        app.view_menu_open = false;
                    }
                    ZONE_PREV => {
                        app.prev_track();
                        update_title(&toplevel, &app);
                    }
                    ZONE_NEXT => {
                        app.next_track();
                        update_title(&toplevel, &app);
                    }
                    ZONE_SEEK_BAR => {
                        if rects.seek.w > 0.0 {
                            let frac = ((cx - rects.seek.x) / rects.seek.w).clamp(0.0, 1.0);
                            app.seeking = true;
                            app.seek_value = frac;
                        }
                    }
                    ZONE_TITLE_BAR => {
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                    ZONE_CLOSE => {
                        state.running = false;
                    }
                    ZONE_MAXIMIZE => {
                        if state.maximized {
                            toplevel.unset_maximized();
                        } else {
                            toplevel.set_maximized();
                        }
                    }
                    ZONE_MINIMIZE => {
                        toplevel.set_minimized();
                    }
                    ZONE_FULLSCREEN => {
                        if state.fullscreen {
                            toplevel.unset_fullscreen();
                        } else {
                            toplevel.set_fullscreen(None);
                        }
                    }
                    ZONE_LOOP => { app.cycle_loop_mode(); }
                    ZONE_CANVAS => {
                        // Click anywhere on the canvas to drag the window (the
                        // whole surface is a move handle). Playback toggles via
                        // the controls or spacebar instead.
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                    _ => {}
                }
            }
        }

        // ── Left release ────────────────────────────────────────────────
        if state.left_released {
            state.left_released = false;
            if app.seeking {
                app.seek_to_fraction(app.seek_value);
            }
            input.on_left_released();
        }

        // ── Render ──────────────────────────────────────────────────────
        let palette = FoxPalette::current();
        let opacity = lntrn_theme::background_opacity();
        rects = crate::render::render_frame(
            &mut gpu, &app, &mut input, &palette, opacity, s,
            win_hf, state.fullscreen,
        );

        surface.frame(&qh, ());
        surface.commit();
    }

    // Remember where we left off so reopening the same file resumes from here.
    app.save_current_position();

    Ok(())
}

// ── Event pump ────────────────────────────────────────────────────────────

/// Dispatch Wayland events, blocking at most `timeout` for socket activity.
///
/// This replaces `EventQueue::blocking_dispatch` so the main loop never waits
/// forever: a minimized window receives no frame callbacks (and no input), so
/// an unbounded block would freeze the loop and starve MPRIS transport commands
/// arriving on the channel. A bounded `libc::poll` wakes promptly on Wayland
/// activity and otherwise falls through on timeout so the caller can service
/// MPRIS and keep playback ticking. (Mirrors the Command Center's
/// `dispatch_with_timeout`.)
fn pump_events(
    event_queue: &mut EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<()> {
    event_queue.flush()?;
    if event_queue.dispatch_pending(state)? > 0 {
        return Ok(());
    }

    // Stake the next socket read before polling so events arriving between
    // prepare and poll aren't lost (per `ReadEventsGuard`'s contract).
    let guard = match event_queue.prepare_read() {
        Some(g) => g,
        None => {
            event_queue.dispatch_pending(state)?;
            return Ok(());
        }
    };

    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut fds = [libc::pollfd {
        fd: guard.connection_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };

    if ret < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err.into());
        }
        // EINTR: drop the guard so the next iteration can re-prepare.
        drop(guard);
        return Ok(());
    }

    if fds[0].revents & libc::POLLIN != 0 {
        match guard.read() {
            Ok(_) => {}
            // Spurious wakeup: poll said ready but the socket had nothing.
            Err(WaylandError::Io(io)) if io.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        event_queue.dispatch_pending(state)?;
    } else {
        // Timed out — drop the guard to cancel the prepared read.
        drop(guard);
    }

    Ok(())
}

// ── Input helpers ───────────────────────────────────────────────────────────

fn update_title(toplevel: &xdg_toplevel::XdgToplevel, app: &App) {
    if app.file_name.is_empty() {
        toplevel.set_title("Lantern Media Player".into());
    } else {
        toplevel.set_title(format!("{} — Lantern Media Player", app.file_name));
    }
}

fn resize_edge_to_cursor_shape(
    edge: xdg_toplevel::ResizeEdge,
) -> wp_cursor_shape_device_v1::Shape {
    use wp_cursor_shape_device_v1::Shape;
    match edge {
        xdg_toplevel::ResizeEdge::Top => Shape::NResize,
        xdg_toplevel::ResizeEdge::Bottom => Shape::SResize,
        xdg_toplevel::ResizeEdge::Left => Shape::WResize,
        xdg_toplevel::ResizeEdge::Right => Shape::EResize,
        xdg_toplevel::ResizeEdge::TopLeft => Shape::NwResize,
        xdg_toplevel::ResizeEdge::TopRight => Shape::NeResize,
        xdg_toplevel::ResizeEdge::BottomLeft => Shape::SwResize,
        xdg_toplevel::ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

/// Resize on any edge / corner within `border` of the window bounds. The
/// controls overlay auto-hides, so when `controls_visible` is true the bottom
/// edge is suppressed to keep the seek bar and buttons clickable; the top edge
/// is likewise suppressed when the title bar is showing (it owns those clicks).
fn edge_resize(
    cx: f32, cy: f32, w: f32, h: f32, border: f32,
    controls_visible: bool, title_visible: bool,
) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border && !title_visible;
    let bottom = cy > h - border && !controls_visible;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(xdg_toplevel::ResizeEdge::TopLeft),
        (_, true, true, _) => Some(xdg_toplevel::ResizeEdge::TopRight),
        (true, _, _, true) => Some(xdg_toplevel::ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(xdg_toplevel::ResizeEdge::BottomRight),
        (true, _, _, _) => Some(xdg_toplevel::ResizeEdge::Left),
        (_, true, _, _) => Some(xdg_toplevel::ResizeEdge::Right),
        (_, _, true, _) => Some(xdg_toplevel::ResizeEdge::Top),
        (_, _, _, true) => Some(xdg_toplevel::ResizeEdge::Bottom),
        _ => None,
    }
}

// Linux keycodes
const KEY_Q: u32 = 16;
const KEY_N: u32 = 49;
const KEY_P: u32 = 25;
const KEY_L: u32 = 38;
const KEY_A: u32 = 30;
const KEY_D: u32 = 32;
const KEY_F: u32 = 33;
const KEY_SPACE: u32 = 57;
const KEY_ESC: u32 = 1;
const KEY_UP: u32 = 103;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;
const KEY_F11: u32 = 87;

fn handle_key(app: &mut App, toplevel: &xdg_toplevel::XdgToplevel, state: &mut State, key: u32) {
    const FIVE_SEC_NS: i64 = 5_000_000_000;
    match key {
        KEY_SPACE => { app.toggle_play_pause(); }
        KEY_LEFT | KEY_A => { app.seek_relative(-FIVE_SEC_NS); }
        KEY_RIGHT | KEY_D => { app.seek_relative(FIVE_SEC_NS); }
        KEY_UP => { app.adjust_volume(0.05); }
        KEY_DOWN => { app.adjust_volume(-0.05); }
        KEY_N => { app.next_track(); }
        KEY_P => { app.prev_track(); }
        KEY_L => { app.cycle_loop_mode(); }
        KEY_F11 | KEY_F => {
            if state.fullscreen {
                toplevel.unset_fullscreen();
            } else {
                toplevel.set_fullscreen(None);
            }
        }
        KEY_ESC => {
            if state.fullscreen {
                toplevel.unset_fullscreen();
            }
        }
        _ if state.ctrl => match key {
            KEY_Q => { state.running = false; }
            _ => {}
        },
        _ => {}
    }
}


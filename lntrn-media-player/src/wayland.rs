use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, Rect, TextRenderer, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::app::App;
use crate::{Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_PLAY_PAUSE, ZONE_SEEK_BAR};

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
    maximized: bool,
    fullscreen: bool,
    // Wayland objects
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    viewporter: Option<wp_viewporter::WpViewporter>,
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
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0,
            maximized: false, fullscreen: false,
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false,
            scroll_delta: 0.0, pointer_serial: 0,
            ctrl: false, key_pressed: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
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
                "wl_output" => { let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ()); }
                "wl_seat" => { state.seat = Some(registry.bind(name, version.min(9), qh, ())); }
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
        _: &mut Self, seat: &wl_seat::WlSeat,
        event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(cap) } = event {
            if cap.contains(wl_seat::Capability::Pointer) { seat.get_pointer(qh, ()); }
            if cap.contains(wl_seat::Capability::Keyboard) { seat.get_keyboard(qh, ()); }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self, _: &wl_pointer::WlPointer,
        event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
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

pub fn run(initial_path: Option<String>) -> Result<()> {
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
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Media Player".into());
    toplevel.set_app_id("lntrn-media-player".into());
    toplevel.set_min_size(480, 320);
    surface.commit();

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

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

    let palette = FoxPalette::dark();
    let mut app = App::new();
    let mut input = InteractionContext::new();

    // Load initial media if provided
    if let Some(path) = initial_path {
        app.open_file(&path);
        update_title(&toplevel, &app);
    }

    // Seek bar rect cache (set each frame in render, read in input handling)
    let mut seek_rect = Rect::new(0.0, 0.0, 0.0, 0.0);

    while state.running {
        // Non-blocking when playing (or audio-only visualizer), blocking when paused
        if app.is_playing() || app.audio_only {
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                eprintln!("[media-player] dispatch error: {e}");
                break;
            }
            event_queue.flush()?;

            let new_frame = app.tick(&gpu.ctx, &gpu.tex_pass);
            if new_frame {
                state.frame_done = true;
            }
            if !state.frame_done {
                std::thread::sleep(std::time::Duration::from_millis(4));
                continue;
            }
        } else {
            if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                eprintln!("[media-player] dispatch error: {e}");
                break;
            }
            // Still tick to update position display even when paused
            app.tick(&gpu.ctx, &gpu.tex_pass);
            if !state.frame_done { continue; }
        }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.ctx.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let s = scale_f;

        // ── Cursor ──────────────────────────────────────────────────────
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        // ── Seek bar drag (motion) ──────────────────────────────────────
        if app.seeking && state.pointer_in_surface && seek_rect.w > 0.0 {
            let frac = ((cx - seek_rect.x) / seek_rect.w).clamp(0.0, 1.0);
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
            if let Some(edge) = edge_resize(cx, cy, wf, hf, border) {
                if let Some(seat) = &state.seat {
                    toplevel.resize(seat, state.pointer_serial, edge);
                }
            } else if let Some(zone_id) = input.on_left_pressed() {
                match zone_id {
                    ZONE_CLOSE => { state.running = false; }
                    ZONE_MINIMIZE => { toplevel.set_minimized(); }
                    ZONE_MAXIMIZE => {
                        if state.maximized { toplevel.unset_maximized(); }
                        else { toplevel.set_maximized(); }
                    }
                    ZONE_PLAY_PAUSE => { app.toggle_play_pause(); }
                    ZONE_SEEK_BAR => {
                        if seek_rect.w > 0.0 {
                            let frac = ((cx - seek_rect.x) / seek_rect.w).clamp(0.0, 1.0);
                            app.seeking = true;
                            app.seek_value = frac;
                        }
                    }
                    ZONE_CANVAS => {
                        // Double-click handled via rapid press — or just toggle play
                        app.toggle_play_pause();
                    }
                    _ => {}
                }
            } else {
                // Title bar drag
                let title_h = 36.0 * s;
                if cy < title_h {
                    if let Some(seat) = &state.seat {
                        toplevel._move(seat, state.pointer_serial);
                    }
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
        seek_rect = crate::render::render_frame(&mut gpu, &app, &mut input, &palette, s);

        surface.frame(&qh, ());
        surface.commit();
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

fn edge_resize(cx: f32, cy: f32, w: f32, h: f32, border: f32) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
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
const KEY_A: u32 = 30;
const KEY_D: u32 = 32;
const KEY_F: u32 = 33;
const KEY_V: u32 = 47;
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
        KEY_V => { app.cycle_vis_mode(); }
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

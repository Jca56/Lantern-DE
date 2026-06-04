use std::ffi::c_void;
use std::path::Path;
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
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::app::App;
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_NAV_PREV, ZONE_NAV_NEXT,
    ZONE_SHUFFLE,
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
const BTN_MIDDLE: u32 = 0x112;

struct State {
    running: bool,
    configured: bool,
    frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    output_phys_height: u32,
    maximized: bool,
    // Wayland objects
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    seat: Option<wl_seat::WlSeat>,
    cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    // Input
    cursor_x: f64,
    cursor_y: f64,
    pointer_in_surface: bool,
    left_pressed: bool,
    left_released: bool,
    middle_pressed: bool,
    middle_released: bool,
    scroll_delta: f32,
    pointer_serial: u32,
    enter_serial: u32,
    // Keyboard
    ctrl: bool,
    key_pressed: Option<u32>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1,
            output_phys_width: 0, output_phys_height: 0, maximized: false,
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_shape_mgr: None, cursor_shape_device: None, current_cursor_shape: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false,
            middle_pressed: false, middle_released: false,
            scroll_delta: 0.0, pointer_serial: 0, enter_serial: 0,
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
impl Dispatch<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, ()> for State {
    fn event(_: &mut Self, _: &wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, _: wp_cursor_shape_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, ()> for State {
    fn event(_: &mut Self, _: &wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, _: wp_cursor_shape_device_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
            wl_output::Event::Mode { width, height, .. } => {
                state.output_phys_width = width as u32;
                state.output_phys_height = height as u32;
            }
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
                // Pair the pointer with a cursor-shape device so we can set
                // named resize cursors on hover (no XCursor theme loading).
                if let Some(mgr) = &state.cursor_shape_mgr {
                    state.cursor_shape_device = Some(mgr.get_pointer(&ptr, qh, ()));
                }
            }
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
            wl_pointer::Event::Enter { serial, surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                // set_shape must reference the enter serial; force a re-apply.
                state.enter_serial = serial;
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
                if button == BTN_MIDDLE && pressed { state.middle_pressed = true; }
                if button == BTN_MIDDLE && released { state.middle_released = true; }
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
    // Second roundtrip so wl_output.Mode/Scale events arrive before we size the window.
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state.wm_base.as_ref()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    // Compute initial window size from the image's native dimensions, capped to
    // ~85% of the output's logical size while preserving aspect ratio. Falls
    // back to 960×640 if no image arg was given or its header is unreadable.
    let scale_i = state.scale.max(1) as u32;
    let screen_logical_w = if state.output_phys_width > 0 {
        state.output_phys_width / scale_i
    } else { 1920 };
    let screen_logical_h = if state.output_phys_height > 0 {
        state.output_phys_height / scale_i
    } else { 1080 };

    let (init_w, init_h) = initial_path.as_deref()
        .and_then(|p| crate::app::peek_image_dimensions(Path::new(p)))
        .map(|(w, h)| fit_to_screen(w, h, screen_logical_w, screen_logical_h))
        .unwrap_or((960, 640));

    if state.width == 0 { state.width = init_w; }
    if state.height == 0 { state.height = init_h; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Image Viewer".into());
    toplevel.set_app_id("lntrn-image-viewer".into());
    // Compositor reads min_size as the initial size hint when no per-app rule
    // matches. We relax this back to (400, 300) right after the first configure
    // so the user can still resize the window down freely.
    toplevel.set_min_size(init_w as i32, init_h as i32);
    surface.commit();

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;
    // Relax the size hint so the user can shrink the window below the image size.
    toplevel.set_min_size(400, 300);

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

    // Load initial image if provided
    if let Some(path) = initial_path {
        app.open_image(&gpu.ctx, &gpu.tex_pass, &path);
    }

    while state.running {
        // Use non-blocking dispatch when animating a GIF, blocking otherwise
        if app.gif.is_some() {
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                eprintln!("[image-viewer] dispatch error: {e}");
                break;
            }
            event_queue.flush()?;
            // Tick GIF animation
            let gif_changed = app.tick_gif(&gpu.ctx, &gpu.tex_pass);
            if gif_changed {
                state.frame_done = true;
            }
            if !state.frame_done {
                // Sleep until next frame is due (or a short poll interval)
                let sleep = app.gif.as_ref()
                    .map(|g| {
                        let remaining = g.current_delay()
                            .saturating_sub(g.last_swap.elapsed());
                        remaining.min(std::time::Duration::from_millis(16))
                    })
                    .unwrap_or(std::time::Duration::from_millis(16));
                std::thread::sleep(sleep);
                continue;
            }
        } else {
            if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                eprintln!("[image-viewer] dispatch error: {e}");
                break;
            }
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

        // ── Scroll → zoom ──────────────────────────────────────────────
        if state.scroll_delta.abs() > 0.01 {
            let title_h = crate::TITLE_H * s;
            let status_h = crate::STATUS_H * s;
            let canvas = Rect::new(0.0, title_h, wf, hf - title_h - status_h);
            if canvas.contains(cx, cy) {
                let factor = if state.scroll_delta < 0.0 { 1.03 } else { 1.0 / 1.03 };
                app.zoom_at(factor, cx, cy, canvas.x + canvas.w * 0.5, canvas.y + canvas.h * 0.5);
            }
            state.scroll_delta = 0.0;
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            handle_key(&mut app, &mut gpu, key, state.ctrl);
        }

        // ── Left press ──────────────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            // Edge resize
            let border = crate::RESIZE_BORDER * s;
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
                    ZONE_CANVAS => {
                        // Start panning
                        app.is_panning = true;
                        app.last_pan_x = cx;
                        app.last_pan_y = cy;
                    }
                    ZONE_NAV_PREV => { app.prev_image(&gpu.ctx, &gpu.tex_pass); }
                    ZONE_NAV_NEXT => { app.next_image(&gpu.ctx, &gpu.tex_pass); }
                    ZONE_SHUFFLE => { app.toggle_shuffle(); }
                    _ => {}
                }
            } else {
                // Title bar drag
                let title_h = crate::TITLE_H * s;
                if cy < title_h {
                    if let Some(seat) = &state.seat {
                        toplevel._move(seat, state.pointer_serial);
                    }
                }
            }
        }

        // ── Middle press → pan ──────────────────────────────────────────
        if state.middle_pressed {
            state.middle_pressed = false;
            app.is_panning = true;
            app.last_pan_x = cx;
            app.last_pan_y = cy;
        }

        // ── Panning with mouse movement ─────────────────────────────────
        if app.is_panning && state.pointer_in_surface {
            let dx = cx - app.last_pan_x;
            let dy = cy - app.last_pan_y;
            app.pan_x += dx;
            app.pan_y += dy;
            app.last_pan_x = cx;
            app.last_pan_y = cy;
        }

        // ── Left/middle release ─────────────────────────────────────────
        if state.left_released {
            state.left_released = false;
            app.is_panning = false;
            input.on_left_released();
        }
        if state.middle_released {
            state.middle_released = false;
            app.is_panning = false;
        }

        // ── Keep SVGs crisp ──────────────────────────────────────────────
        // Re-rasterize the vector image to the size it's about to be drawn at,
        // mirroring the fit+zoom math in render_frame so it never looks blurry.
        if let Some(img) = &app.image {
            let canvas_w = wf;
            let canvas_h = hf - (crate::TITLE_H + crate::STATUS_H) * s;
            let fit_zoom = (canvas_w / img.width as f32).min(canvas_h / img.height as f32);
            let display_zoom = fit_zoom * app.zoom;
            let disp_w = img.width as f32 * display_zoom;
            let disp_h = img.height as f32 * display_zoom;
            app.maybe_rerender_svg(&gpu.ctx, &gpu.tex_pass, disp_w, disp_h);
        }

        // ── Resize cursor on hover ───────────────────────────────────────
        // Show a directional resize cursor whenever the pointer is over an
        // edge band — not just while an interactive resize is in progress.
        if state.pointer_in_surface {
            let border = crate::RESIZE_BORDER * s;
            let desired = match edge_resize(cx, cy, wf, hf, border) {
                Some(edge) => resize_edge_to_cursor_shape(edge),
                None => wp_cursor_shape_device_v1::Shape::Default,
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Render ──────────────────────────────────────────────────────
        crate::render::render_frame(&mut gpu, &app, &mut input, &palette, s);

        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

// ── Sizing helpers ──────────────────────────────────────────────────────────

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

// ── Input helpers ───────────────────────────────────────────────────────────

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

/// Map a resize edge to the matching directional cursor shape.
fn resize_edge_to_cursor_shape(edge: xdg_toplevel::ResizeEdge) -> wp_cursor_shape_device_v1::Shape {
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

// Linux keycodes
const KEY_Q: u32 = 16;
const KEY_0: u32 = 11;
const KEY_EQUAL: u32 = 13; // =/+ key
const KEY_MINUS: u32 = 12;
const KEY_S: u32 = 31;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;

fn handle_key(app: &mut App, gpu: &mut Gpu, key: u32, ctrl: bool) {
    match key {
        KEY_LEFT => { app.prev_image(&gpu.ctx, &gpu.tex_pass); }
        KEY_RIGHT => { app.next_image(&gpu.ctx, &gpu.tex_pass); }
        KEY_S if !ctrl => { app.toggle_shuffle(); }
        _ if ctrl => match key {
            KEY_Q => std::process::exit(0),
            KEY_EQUAL => { app.zoom = (app.zoom * 1.05).min(50.0); }
            KEY_MINUS => { app.zoom = (app.zoom / 1.05).max(0.05); }
            KEY_0 => { app.fit_to_view(); }
            _ => {}
        },
        _ => {}
    }
}

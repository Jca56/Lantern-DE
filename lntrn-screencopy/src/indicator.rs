//! On-screen recording indicator.
//!
//! A small fullscreen-relative layer surface anchored to the top-right
//! of the monitor we're recording. Shows a red dot + timer + stop
//! button while a recording is in progress; clicking the stop button
//! flips the shared `stop` flag so the capture loop finalizes the file.
//!
//! Runs in its own thread on its own wayland connection. The capture
//! loop in `capture::record_region` runs in the main thread and reads
//! the same `stop` flag; either side flipping the flag ends the
//! recording.

use std::os::fd::AsRawFd;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use std::ffi::c_void;
use wayland_client::{
    backend::ObjectId,
    protocol::{
        wl_callback, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use lntrn_render::{Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer};

// Logical (compositor) units. Indicator is sized to be readable on
// HiDPI without being intrusive.
const W: u32 = 260;
const H: u32 = 60;
const MARGIN: i32 = 16;

const BTN_LEFT: u32 = 0x110;

/// Normal redraw cadence (the timer only shows whole seconds; ~10 Hz
/// keeps the dot pulse smooth without rendering at full refresh rate).
const RENDER_INTERVAL: Duration = Duration::from_millis(100);
/// If no frame callback arrives for this long (compositor coalesced or
/// dropped one), redraw anyway so the timer never visibly stalls.
const RENDER_FALLBACK: Duration = Duration::from_millis(500);

pub fn spawn(
    output_name: String,
    stop: &'static AtomicBool,
    started_at: Instant,
    frame_count: &'static AtomicU64,
) {
    std::thread::Builder::new()
        .name("screencopy-indicator".into())
        .spawn(move || {
            if let Err(e) = run(&output_name, stop, started_at, frame_count) {
                eprintln!("indicator error: {e}");
            }
        })
        .expect("spawn indicator thread");
}

fn run(
    output_name: &str,
    stop: &'static AtomicBool,
    started_at: Instant,
    frame_count: &'static AtomicU64,
) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();
    let mut state = State::new();

    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?; // drain output Name events

    let compositor = state
        .compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let layer_shell = state
        .layer_shell
        .clone()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let target_output = state
        .outputs
        .iter()
        .find(|o| o.name.as_deref() == Some(output_name))
        .map(|o| o.proxy.clone());

    let surface = compositor.create_surface(&qh, ());
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        target_output.as_ref(),
        zwlr_layer_shell_v1::Layer::Overlay,
        "lntrn-screencopy-indicator".to_string(),
        &qh,
        (),
    );
    {
        use zwlr_layer_surface_v1::Anchor;
        layer_surface.set_anchor(Anchor::Top | Anchor::Right);
        layer_surface.set_size(W, H);
        layer_surface.set_margin(MARGIN, MARGIN, 0, 0);
        // No keyboard grab — we only need pointer clicks on the stop
        // button. The user must remain able to type into whatever
        // they're actually recording.
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::None,
        );
    }
    surface.commit();

    while !state.configured {
        queue.blocking_dispatch(&mut state)?;
    }
    queue.roundtrip(&mut state)?;
    surface.set_buffer_scale(1);

    let scale = compute_scale(&state);
    let phys_w = (state.width as f32 * scale) as u32;
    let phys_h = (state.height as f32 * scale) as u32;
    let viewport = state.viewporter.as_ref().map(|vp| {
        let v = vp.get_viewport(&surface, &qh, ());
        v.set_destination(state.width as i32, state.height as i32);
        v
    });

    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let mut gpu = GpuContext::from_window(&handle, phys_w.max(1), phys_h.max(1))
        .map_err(|e| anyhow!("indicator GPU init: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);

    state.frame_done = true;
    // Force an immediate first paint, then ~10 Hz redraws after that.
    let mut last_render = Instant::now() - RENDER_INTERVAL;

    while !stop.load(Ordering::SeqCst) {
        // Pump events. `dispatch_pending` alone never reads the socket —
        // it only processes events some other reader already queued — so
        // frame callbacks (and stop-button clicks) would sit unread
        // forever and the timer would freeze on the first frame. Do a
        // real prepare_read/poll/read cycle, with the poll timeout
        // doubling as the loop's tick.
        let _ = conn.flush();
        if let Some(guard) = queue.prepare_read() {
            let mut pfd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 100) };
            if ready > 0 {
                let _ = guard.read();
            }
            // ready == 0 → timeout; dropping the guard cancels the read.
        }
        let _ = queue.dispatch_pending(&mut state);

        // Click hit-test for the stop button.
        if state.click_pending {
            state.click_pending = false;
            let cx = state.cursor_x as f32;
            let cy = state.cursor_y as f32;
            let scale = compute_scale(&state);
            // Stop button occupies the right ~80 logical-px of the bar
            // (W=260, button width 70 + 10 margin).
            let btn_logical_x = (W as f32) - 80.0;
            if cx >= btn_logical_x && cy >= 0.0 && cy <= H as f32 {
                let _ = scale;
                stop.store(true, Ordering::SeqCst);
                break;
            }
        }

        let due = state.frame_done && last_render.elapsed() >= RENDER_INTERVAL;
        let overdue = last_render.elapsed() >= RENDER_FALLBACK;
        if due || overdue {
            state.frame_done = false;
            last_render = Instant::now();
            let _ = surface.frame(&qh, ());
            let scale = compute_scale(&state);
            let cur_phys_w = ((state.width as f32) * scale) as u32;
            let cur_phys_h = ((state.height as f32) * scale) as u32;
            if cur_phys_w != gpu.width() || cur_phys_h != gpu.height() {
                gpu.resize(cur_phys_w.max(1), cur_phys_h.max(1));
                if let Some(vp) = viewport.as_ref() {
                    vp.set_destination(state.width as i32, state.height as i32);
                }
            }
            match render(
                &mut gpu,
                &mut painter,
                &mut text,
                scale,
                started_at.elapsed(),
                frame_count.load(Ordering::Relaxed),
                state.cursor_in,
                state.cursor_x as f32,
                state.cursor_y as f32,
            ) {
                Ok(()) => {
                    surface.commit();
                    let _ = conn.flush();
                }
                Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                    let scale = compute_scale(&state);
                    gpu.resize(
                        ((state.width as f32) * scale).max(1.0) as u32,
                        ((state.height as f32) * scale).max(1.0) as u32,
                    );
                }
                Err(_) => {}
            }
        }
    }

    drop(text);
    drop(painter);
    drop(gpu);
    layer_surface.destroy();
    surface.destroy();
    let _ = conn.flush();
    Ok(())
}

fn render(
    gpu: &mut GpuContext,
    painter: &mut Painter,
    text: &mut TextRenderer,
    scale: f32,
    elapsed: Duration,
    frames: u64,
    cursor_in: bool,
    cur_x: f32,
    cur_y: f32,
) -> Result<(), SurfaceError> {
    let sw = gpu.width() as f32;
    let sh = gpu.height() as f32;
    painter.clear();

    // Background: dark rounded pill.
    let bg = Color::from_rgba8(20, 20, 24, 220);
    let border = Color::from_rgba8(255, 80, 80, 255);
    let text_color = Color::from_rgba8(0xe8, 0xdc, 0xc8, 0xff);
    let radius = 16.0 * scale.max(1.0);
    painter.rect_filled(Rect::new(0.0, 0.0, sw, sh), radius, bg);
    painter.rect_stroke(Rect::new(0.0, 0.0, sw, sh), radius, 2.0 * scale, border);

    // Red recording dot (pulses subtly with seconds).
    let pulse = ((elapsed.as_millis() % 1000) as f32 / 1000.0 * std::f32::consts::TAU).sin();
    let dot_r = (10.0 + pulse * 1.5) * scale.max(1.0);
    let dot_x = 22.0 * scale.max(1.0);
    let dot_y = sh / 2.0;
    painter.rect_filled(
        Rect::new(dot_x - dot_r, dot_y - dot_r, dot_r * 2.0, dot_r * 2.0),
        dot_r,
        Color::from_rgba8(255, 60, 60, 255),
    );

    // Timer text.
    let secs = elapsed.as_secs();
    let label = format!("{:02}:{:02}", secs / 60, secs % 60);
    let font = 24.0 * scale.max(1.0);
    text.queue(
        &label,
        font,
        42.0 * scale.max(1.0),
        sh / 2.0 - font / 2.0,
        text_color,
        140.0 * scale.max(1.0),
        sw as u32,
        sh as u32,
    );

    // Stop button on the right.
    let btn_w = 70.0 * scale.max(1.0);
    let btn_h = 38.0 * scale.max(1.0);
    let btn_x = sw - btn_w - 10.0 * scale.max(1.0);
    let btn_y = (sh - btn_h) / 2.0;
    let hovered = cursor_in
        && cur_x >= btn_x / scale.max(1.0)
        && cur_x <= (btn_x + btn_w) / scale.max(1.0)
        && cur_y >= btn_y / scale.max(1.0)
        && cur_y <= (btn_y + btn_h) / scale.max(1.0);
    let btn_bg = if hovered {
        Color::from_rgba8(220, 80, 80, 255)
    } else {
        Color::from_rgba8(160, 60, 60, 255)
    };
    painter.rect_filled(Rect::new(btn_x, btn_y, btn_w, btn_h), 8.0 * scale, btn_bg);
    text.queue(
        "STOP",
        font * 0.8,
        btn_x + 12.0 * scale.max(1.0),
        btn_y + (btn_h - font * 0.8) / 2.0,
        Color::WHITE,
        btn_w,
        sw as u32,
        sh as u32,
    );

    let _ = frames; // Reserved for future overlays (FPS readout, etc.)

    let mut frame = gpu.begin_frame("indicator")?;
    let view = frame.view().clone();
    {
        let encoder = frame.encoder_mut();
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("indicator-clear"),
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

fn compute_scale(state: &State) -> f32 {
    state
        .entered_output_id
        .as_ref()
        .and_then(|id| state.outputs.iter().find(|o| &o.proxy.id() == id))
        .and_then(|o| {
            if o.mode_width > 0 && state.width > 0 {
                Some(o.mode_width as f32 / state.width as f32)
            } else {
                None
            }
        })
        .unwrap_or(1.0)
}

// ── Wayland plumbing ────────────────────────────────────────────────────────

struct WaylandHandle {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}
impl HasDisplayHandle for WaylandHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                self.display,
            )))
        })
    }
}
impl HasWindowHandle for WaylandHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::Wayland(WaylandWindowHandle::new(
                self.surface,
            )))
        })
    }
}

struct TrackedOutput {
    proxy: wl_output::WlOutput,
    name: Option<String>,
    mode_width: u32,
}

struct State {
    configured: bool,
    frame_done: bool,
    width: u32,
    height: u32,
    outputs: Vec<TrackedOutput>,
    entered_output_id: Option<ObjectId>,
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    cursor_x: f64,
    cursor_y: f64,
    cursor_in: bool,
    click_pending: bool,
}

impl State {
    fn new() -> Self {
        Self {
            configured: false,
            frame_done: false,
            width: W,
            height: H,
            outputs: Vec::new(),
            entered_output_id: None,
            compositor: None,
            layer_shell: None,
            viewporter: None,
            cursor_x: -1.0,
            cursor_y: -1.0,
            cursor_in: false,
            click_pending: false,
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let proxy: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                    state.outputs.push(TrackedOutput {
                        proxy,
                        name: None,
                        mode_width: 0,
                    });
                }
                "wl_seat" => {
                    let _: wl_seat::WlSeat = registry.bind(name, version.min(9), qh, ());
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
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_surface::Event::Enter { output } = event {
            state.entered_output_id = Some(output.id());
        }
    }
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for State {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(_: &mut Self, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = output.id();
        let Some(tracked) = state.outputs.iter_mut().find(|o| o.proxy.id() == id) else { return; };
        match event {
            wl_output::Event::Name { name } => tracked.name = Some(name),
            wl_output::Event::Mode { width, .. } => tracked.mode_width = width as u32,
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.frame_done = true;
    }
}
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, width, height } = event {
            layer_surface.ack_configure(serial);
            if width > 0 { state.width = width; }
            if height > 0 { state.height = height; }
            state.configured = true;
            state.frame_done = true;
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: caps, .. } = event {
            if let WEnum::Value(caps) = caps {
                if caps.contains(wl_seat::Capability::Pointer) {
                    seat.get_pointer(qh, ());
                }
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.cursor_in = true;
            }
            wl_pointer::Event::Leave { .. } => state.cursor_in = false,
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Button { button, state: bs, .. } => {
                if button == BTN_LEFT && bs == WEnum::Value(wl_pointer::ButtonState::Pressed) {
                    state.click_pending = true;
                }
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

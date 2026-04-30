//! Layer-shell client + render loop for Command Center.
//!
//! Forked from `lntrn-menu/src/layershell.rs` (closest precedent: a
//! fullscreen overlay with a clickable rect inside it that dismisses on
//! click-outside). Differences:
//!
//! - We use `KeyboardInteractivity::OnDemand` (matching lntrn-menu, which
//!   works fine in our compositor — the panel grabs focus on its first
//!   pointer enter, and we drive typing for the search field).
//! - We draw a glassy panel rect via `crate::render`, not a context menu.
//! - Phase 1: no input handling beyond pointer enter/leave to get focus.
//!   Phase 1.8 adds Esc-to-close + click-outside.

use std::ffi::c_void;
use std::ptr::NonNull;

use std::os::unix::net::UnixDatagram;
use std::time::Duration;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, SurfaceError, TextRenderer, TextureDraw, TexturePass};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry,
        wl_seat, wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::app::{AppState, PanelRect};
use crate::ipc::{self, Cmd};
use crate::launcher::icons::IconCache;

/// Phys-pixel icon size used for both the result list and the pinned
/// row. Sized for the larger of the two consumers (pinned tile is 88
/// logical px @ 1.25 scale ≈ 110 phys; insets and 2x for HiDPI quality
/// land us at 144). The result-list icons get downscaled at draw time
/// so quality stays sharp.
const ICON_PHYS_SIZE: u32 = 144;

/// Evdev keycodes we care about.
const KEY_ESC: u32 = 1;
/// Left Shift / Right Shift evdev keycodes — tracked so we can forward
/// the shift state to the search input's char mapper.
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
/// Linux input button codes.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

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

struct WlState {
    running: bool,
    configured: bool,
    frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    cursor_x: f64,
    cursor_y: f64,
    pointer_in_surface: bool,
    /// Set when the user pressed Esc; consumed by the render loop.
    esc_pressed: bool,
    /// Set when the user clicked the left mouse button; consumed by
    /// the render loop, which then hit-tests against the panel rect.
    left_clicked: bool,
    /// Set when the user right-clicked. Used by Phase 2.6 to toggle
    /// pin/unpin on whatever tile/row is under the cursor.
    right_clicked: bool,
    /// Whether either Shift modifier is currently held — needed by the
    /// search input's keycode → char mapper.
    shift_held: bool,
    /// Queued key presses for the render loop to forward to `search.on_key`.
    /// Single key per dispatch is fine; we just remember the most recent
    /// one and let the loop handle it.
    pending_key: Option<u32>,
}

impl WlState {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: true,
            width: 0,
            height: 0,
            scale: 1,
            output_phys_width: 0,
            compositor: None,
            layer_shell: None,
            viewporter: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            esc_pressed: false,
            left_clicked: false,
            right_clicked: false,
            shift_held: false,
            pending_key: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
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

// ── Dispatch impls (boilerplate, mostly empty) ──────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for WlState {
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
                    let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                }
                "wl_seat" => {
                    let _: wl_seat::WlSeat = registry.bind(name, version.min(9), qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for WlState {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for WlState {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_region::WlRegion, ()> for WlState {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for WlState {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for WlState {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for WlState {
    fn event(_: &mut Self, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => state.scale = factor,
            wl_output::Event::Mode { width, .. } => state.output_phys_width = width as u32,
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WlState {
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

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for WlState {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer_surface.ack_configure(serial);
                if width > 0 {
                    state.width = width;
                }
                if height > 0 {
                    state.height = height;
                }
                state.configured = true;
                state.frame_done = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.running = false,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WlState {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: caps, .. } = event {
            if let wayland_client::WEnum::Value(caps) = caps {
                if caps.contains(wl_seat::Capability::Pointer) {
                    seat.get_pointer(qh, ());
                }
                if caps.contains(wl_seat::Capability::Keyboard) {
                    seat.get_keyboard(qh, ());
                }
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for WlState {
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
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Leave { .. } => state.pointer_in_surface = false,
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                use wayland_client::WEnum;
                let pressed = WEnum::Value(wl_pointer::ButtonState::Pressed);
                if button == BTN_LEFT && btn_state == pressed {
                    state.left_clicked = true;
                }
                if button == BTN_RIGHT && btn_state == pressed {
                    state.right_clicked = true;
                }
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::WEnum;
        match event {
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                let pressed = key_state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                let released = key_state == WEnum::Value(wl_keyboard::KeyState::Released);
                if pressed && key == KEY_ESC {
                    state.esc_pressed = true;
                }
                // Track shift directly off the key events. We could also use
                // wl_keyboard::Event::Modifiers, but tracking key state is
                // sufficient for our ASCII-only mapping in Phase 2.1.
                if key == KEY_LEFTSHIFT || key == KEY_RIGHTSHIFT {
                    if pressed {
                        state.shift_held = true;
                    } else if released {
                        state.shift_held = false;
                    }
                } else if pressed && key != KEY_ESC {
                    state.pending_key = Some(key);
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                // Shift is bit 0 of the depressed mask; refresh from this
                // truth in case we missed a key event (e.g., the user held
                // shift before the panel got focus).
                state.shift_held = (mods_depressed & 1) != 0;
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Idle tick when the panel is hidden — bound the loop to ~20Hz so we
/// promptly notice IPC commands without burning CPU. When animating
/// or visible we use the wayland frame callback for pacing.
const IDLE_TICK: Duration = Duration::from_millis(50);

/// Run the daemon. `initial_visible == true` opens the panel on startup
/// (e.g., when the user just typed `lntrn-command-center --show`).
pub fn run(sock: UnixDatagram, initial_visible: bool) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<WlState> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut wl = WlState::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut wl)?;

    let compositor = wl
        .compositor
        .as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?
        .clone();
    let layer_shell = wl
        .layer_shell
        .as_ref()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?
        .clone();

    let surface = compositor.create_surface(&qh, ());
    let empty_region = compositor.create_region(&qh, ());

    // Fullscreen overlay: anchor all four edges, size 0×0 = fill screen.
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "lntrn-command-center".to_string(),
        &qh,
        (),
    );
    {
        use zwlr_layer_surface_v1::Anchor;
        layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer_surface.set_size(0, 0);
        layer_surface.set_exclusive_zone(-1);
        // Start with keyboard interactivity off so we don't grab focus
        // away from windows below until the panel is actually visible.
        // We flip this to Exclusive on visibility transitions below.
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::None,
        );
    }
    // Empty input region during init — flip to None when visible so
    // pointer events land on us (for click-outside dismiss), and flip
    // back to empty when hidden so clicks pass through to other windows.
    surface.set_input_region(Some(&empty_region));
    surface.commit();

    while !wl.configured {
        event_queue.blocking_dispatch(&mut wl)?;
    }
    if wl.width == 0 {
        return Err(anyhow!("compositor sent zero-width configure"));
    }
    event_queue.roundtrip(&mut wl)?;

    tracing::info!(w = wl.width, h = wl.height, "command-center overlay configured");

    surface.set_buffer_scale(1);
    let viewport = wl.viewporter.as_ref().map(|vp| {
        let v = vp.get_viewport(&surface, &qh, ());
        v.set_destination(wl.width as i32, wl.height as i32);
        v
    });

    // wgpu setup.
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let phys_w = wl.phys_width().max(1);
    let phys_h = wl.phys_height().max(1);
    let mut gpu = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let tex_pass = TexturePass::new(&gpu);
    let mut icon_cache = IconCache::new(ICON_PHYS_SIZE);

    // Daemon stays in input-passthrough mode by default. We only grab
    // pointer + keyboard when the panel is visible — see
    // `set_active_input` below.
    let mut app = AppState::new();
    let mut input_active = false;

    if initial_visible {
        app.open();
        set_active_input(&surface, &layer_surface, &empty_region, true);
        input_active = true;
    }

    tracing::info!(initial_visible, "command-center daemon ready");

    while wl.running {
        // Drain any queued IPC commands and apply them.
        if let Some(cmd) = ipc::drain(&sock) {
            tracing::debug!(?cmd, "ipc command received");
            match cmd {
                Cmd::Toggle => app.toggle(),
                Cmd::Show => app.open(),
                Cmd::Hide => app.close(),
            }
        }

        // Sync input grab state with current visibility. We grab as soon
        // as we start opening (so typing during the open animation lands
        // in the search field, not the previously-focused window) and
        // release the moment we go fully hidden (so pointer events stop
        // hitting our invisible surface).
        let want_active = !app.is_hidden();
        if want_active != input_active {
            tracing::debug!(active = want_active, "switching input grab");
            set_active_input(&surface, &layer_surface, &empty_region, want_active);
            input_active = want_active;
        }

        // Pump wayland events. When the panel is animating or visible we
        // expect frame callbacks → blocking_dispatch wakes promptly. When
        // hidden we use a short idle tick so IPC stays responsive.
        if app.is_hidden() {
            // Non-blocking dispatch: process anything queued, then sleep.
            event_queue.dispatch_pending(&mut wl)?;
            event_queue.flush()?;
            std::thread::sleep(IDLE_TICK);
            continue;
        }

        // Active path: block for an event with a small timeout so we
        // still pick up IPC commands while waiting. Smithay's event_queue
        // doesn't expose a timeout directly, but we can approximate by
        // checking for prepared reads + dispatching pending first.
        event_queue.dispatch_pending(&mut wl)?;
        event_queue.flush()?;
        // Block for the next event; the frame callback usually arrives
        // every ~16ms while visible so this won't stall noticeably.
        event_queue.blocking_dispatch(&mut wl)?;

        // Tick the animation state machine.
        app.tick();

        // Handle Esc → close.
        if wl.esc_pressed {
            wl.esc_pressed = false;
            tracing::debug!("Esc pressed → close");
            app.close();
        }

        // Dispatch the next pending keypress: navigation/launch keys
        // are intercepted here; everything else falls through to the
        // search input as a typing event.
        if let Some(key) = wl.pending_key.take() {
            use crate::search::input::*;
            match key {
                KEY_UP => app.select_up(),
                KEY_DOWN => app.select_down(),
                KEY_LEFT if app.search.input.is_empty() => app.select_left(),
                KEY_RIGHT if app.search.input.is_empty() => app.select_right(),
                KEY_ENTER | KEY_KP_ENTER => {
                    app.launch_selected();
                }
                _ => {
                    let _ = app.forward_key(key, wl.shift_held);
                }
            }
        }

        // Handle left-click: if outside the panel rect → close, otherwise
        // hit-test against the launcher and launch if a tile/row was hit.
        if wl.left_clicked {
            wl.left_clicked = false;
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute(phys_w, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            if !panel.contains(phys_cx, phys_cy) {
                tracing::debug!(
                    cursor = ?(phys_cx, phys_cy),
                    panel = ?(panel.x, panel.y, panel.w, panel.h),
                    "click outside panel → close"
                );
                app.close();
            } else if let Some(target) = app.hit_test_launcher(panel, scale_f, phys_cx, phys_cy) {
                tracing::debug!(?target, "left-click on launcher entry → activate");
                app.activate_at(target);
            }
            // Click inside the panel but not on a clickable entity is
            // a no-op (e.g., hitting blank space between tiles or above
            // the first result row).
        }

        // Right-click: toggle pin/unpin on whichever tile/row was hit.
        if wl.right_clicked {
            wl.right_clicked = false;
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute(phys_w, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            if let Some(target) = app.hit_test_launcher(panel, scale_f, phys_cx, phys_cy) {
                tracing::debug!(?target, "right-click → toggle pin");
                app.toggle_pin_at(target);
            }
        }

        if !wl.frame_done {
            continue;
        }
        wl.frame_done = false;

        let scale_f = wl.fractional_scale() as f32;

        if wl.configured {
            wl.configured = false;
            gpu.resize(wl.phys_width().max(1), wl.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(wl.width as i32, wl.height as i32);
            }
        }

        let phys_w = wl.phys_width().max(1);
        let phys_h = wl.phys_height().max(1);

        // Reset both render queues at the start of every frame. The text
        // renderer in particular accumulates glyphs across calls; without
        // this, a fully-hidden frame still re-renders the last frame's
        // text on top of the transparent surface, leaving a ghost visible
        // after the close animation finishes.
        painter.clear();
        text.clear();

        let panel_draw = crate::render::draw_panel(&mut painter, &app, phys_w, scale_f);
        let icon_requests = if let Some(p) = &panel_draw {
            crate::render::draw_content(&mut painter, &mut text, &app, p, phys_w, phys_h)
        } else {
            Vec::new()
        };

        // Materialize icon requests into TextureDraws in two phases so
        // we don't ask the borrow checker to juggle &mut + & on the same
        // cache. Phase A: ensure each icon is loaded. Phase B: read-only
        // peek to build the draw list.
        for req in &icon_requests {
            icon_cache.ensure_loaded(&gpu, &tex_pass, &req.app_id, req.icon_name.as_deref());
        }
        let tex_draws: Vec<TextureDraw> = icon_requests
            .iter()
            .filter_map(|req| {
                icon_cache.peek(&req.app_id).map(|tex| {
                    let mut d = TextureDraw::new(tex, req.x, req.y, req.size, req.size);
                    d.opacity = req.opacity;
                    d
                })
            })
            .collect();

        match gpu.begin_frame("CommandCenter") {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
                if !tex_draws.is_empty() {
                    tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
                }
                text.render_queued(&gpu, frame.encoder_mut(), &view);
                frame.submit(&gpu.queue);
            }
            Err(SurfaceError::Lost | SurfaceError::Outdated) => {
                gpu.resize(wl.phys_width().max(1), wl.phys_height().max(1));
            }
            Err(_) => {}
        }

        // Schedule the next frame callback while we're still active.
        surface.frame(&qh, ());
        surface.commit();

        // After the close animation finishes, transition to idle. We
        // commit one final transparent frame so the compositor doesn't
        // keep our last visible buffer pinned.
        if app.is_hidden() {
            commit_transparent(&mut gpu, &surface);
        }
    }

    Ok(())
}

/// Switch the surface between "active" (visible / animating, grabbing
/// keyboard + pointer) and "passthrough" (hidden, pointer events fall
/// through to windows below, no keyboard focus).
///
/// Called on visibility transitions, not every frame. Layer-shell takes
/// effect on the next surface commit, no configure cycle needed.
fn set_active_input(
    surface: &wl_surface::WlSurface,
    layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    empty_region: &wl_region::WlRegion,
    active: bool,
) {
    if active {
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        );
        // None = accept input across the whole surface; we hit-test
        // against the panel rect in code for click-outside dismiss.
        surface.set_input_region(None);
    } else {
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::None,
        );
        surface.set_input_region(Some(empty_region));
    }
    surface.commit();
}

/// Submit a single fully-transparent frame so the surface goes away
/// visually without us destroying it. Lets us re-show instantly later.
fn commit_transparent(gpu: &mut GpuContext, surface: &wl_surface::WlSurface) {
    if let Ok(mut frame) = gpu.begin_frame("CommandCenter:hidden") {
        let view = frame.view().clone();
        let mut painter = Painter::new(gpu);
        painter.clear();
        painter.render_pass(gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
        frame.submit(&gpu.queue);
    }
    surface.commit();
}

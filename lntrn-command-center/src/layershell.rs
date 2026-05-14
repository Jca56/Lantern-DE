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

use std::os::unix::net::UnixListener;
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
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::toplevel::ToplevelTracker;

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
/// Left / Right Ctrl evdev keycodes. We track Ctrl so the terminal
/// view can build Ctrl-letter chord bytes (Ctrl-C → 0x03, etc.).
const KEY_LEFTCTRL: u32 = 29;
const KEY_RIGHTCTRL: u32 = 97;
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
    /// Whether the left button is currently held down. Tracked
    /// separately from `left_clicked` so the render loop can run a
    /// drag-to-scrub interaction (e.g. the audio slider).
    left_held: bool,
    /// Set on the frame the left button is released; consumed by the
    /// render loop so pin drag-reorder can commit on release.
    left_released_this_frame: bool,
    /// Set when the user right-clicked. Used by Phase 2.6 to toggle
    /// pin/unpin on whatever tile/row is under the cursor.
    right_clicked: bool,
    /// Whether either Shift modifier is currently held — needed by the
    /// search input's keycode → char mapper.
    shift_held: bool,
    ctrl_held: bool,
    /// Currently held key (raw evdev code) + the wall-clock instant
    /// at which it was pressed. Used by the render loop to synthesize
    /// auto-repeat: after a short delay, repeat the key at a steady
    /// rate so things like backspace + arrow keys can be held.
    held_key: Option<(u32, std::time::Instant)>,
    /// Last time we emitted a synthesized repeat for `held_key`. Reset
    /// each time the key changes.
    last_repeat: Option<std::time::Instant>,
    /// Queued key presses for the render loop to forward to `search.on_key`.
    /// Single key per dispatch is fine; we just remember the most recent
    /// one and let the loop handle it.
    pending_key: Option<u32>,
    /// Accumulated vertical scroll delta (Wayland axis units, ≈ pixels)
    /// since the last render-loop drain. Positive = scroll down.
    scroll_delta_v: f64,
    /// Foreign toplevel tracker — list of open windows.
    toplevels: ToplevelTracker,
    /// Last seat we saw — needed to call `activate(seat)` on a toplevel
    /// handle when the user clicks an Open tile.
    seat: Option<wl_seat::WlSeat>,
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
            left_released_this_frame: false,
            left_held: false,
            right_clicked: false,
            shift_held: false,
            ctrl_held: false,
            held_key: None,
            last_repeat: None,
            pending_key: None,
            scroll_delta_v: 0.0,
            toplevels: ToplevelTracker::new(),
            seat: None,
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
                    let seat: wl_seat::WlSeat = registry.bind(name, version.min(9), qh, ());
                    state.seat = Some(seat);
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    let _: zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1 =
                        registry.bind(name, version.min(3), qh, ());
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
                let released = WEnum::Value(wl_pointer::ButtonState::Released);
                if button == BTN_LEFT {
                    if btn_state == pressed {
                        state.left_clicked = true;
                        state.left_held = true;
                    } else if btn_state == released {
                        state.left_held = false;
                        state.left_released_this_frame = true;
                    }
                }
                if button == BTN_RIGHT && btn_state == pressed {
                    state.right_clicked = true;
                }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                use wayland_client::WEnum;
                if axis == WEnum::Value(wl_pointer::Axis::VerticalScroll) {
                    // libinput reports `value` in axis units that
                    // approximate pixels for high-res scroll wheels and
                    // ~10 per notch for discrete wheels. Accumulate
                    // raw; the render loop scales it.
                    state.scroll_delta_v += value;
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
                } else if key == KEY_LEFTCTRL || key == KEY_RIGHTCTRL {
                    if pressed {
                        state.ctrl_held = true;
                    } else if released {
                        state.ctrl_held = false;
                    }
                } else if pressed && key != KEY_ESC {
                    state.pending_key = Some(key);
                    state.held_key = Some((key, std::time::Instant::now()));
                    state.last_repeat = None;
                } else if released {
                    if let Some((held, _)) = state.held_key {
                        if held == key {
                            state.held_key = None;
                            state.last_repeat = None;
                        }
                    }
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                // Shift is bit 0 of the depressed mask; refresh from this
                // truth in case we missed a key event (e.g., the user held
                // shift before the panel got focus).
                state.shift_held = (mods_depressed & 1) != 0;
                // Ctrl is bit 2 of the depressed mask under the default
                // xkb layout. Same fallback as shift: refresh from the
                // authoritative mods state.
                state.ctrl_held = (mods_depressed & 4) != 0;
            }
            wl_keyboard::Event::Enter { .. } | wl_keyboard::Event::Leave { .. } => {
                // Focus change: any keys we previously tracked as "held"
                // are no longer guaranteed to be down (we won't get a
                // Release if focus left mid-press). Drop all key state
                // so stale auto-repeat doesn't fire on the next open.
                state.held_key = None;
                state.last_repeat = None;
                state.pending_key = None;
                state.shift_held = false;
                state.ctrl_held = false;
                state.esc_pressed = false;
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

// ── Foreign toplevel manager ───────────────────────────────────────────────

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.on_new(toplevel);
        }
    }
    wayland_client::event_created_child!(WlState, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        0 => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for WlState {
    fn event(
        state: &mut Self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.toplevels.on_app_id(handle, app_id);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                state.toplevels.on_title(handle, title);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                state.toplevels.on_state(handle, &bytes);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.toplevels.on_done(handle);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.on_closed(handle);
            }
            _ => {}
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Idle tick when the panel is hidden — bound the loop to ~20Hz so we
/// promptly notice IPC commands without burning CPU. When animating
/// or visible we use the wayland frame callback for pacing.
const IDLE_TICK: Duration = Duration::from_millis(50);

/// Run the daemon. `initial_visible == true` opens the panel on startup
/// (e.g., when the user just typed `lntrn-command-center --show`).
pub fn run(sock: UnixListener, initial_visible: bool) -> Result<()> {
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
    // Second, monospace-only text renderer used exclusively for the
    // terminal grid. Keeps the rest of the panel on the sans family
    // (where proportional metrics look right) while the terminal gets
    // proper monospace alignment.
    let mut mono_text = TextRenderer::new_monospace(&gpu);
    let tex_pass = TexturePass::new(&gpu);
    let mut icon_cache = IconCache::new(ICON_PHYS_SIZE);

    // Daemon stays in input-passthrough mode by default. We only grab
    // pointer + keyboard when the panel is visible — see
    // `set_active_input` below.
    let mut app = AppState::new();
    let mut input_active = false;
    let mut thumbs = crate::thumbs::CcThumbsClient::new();

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
            // Any externally-triggered visibility change resets the
            // keyboard-held state. This is a safety net for the stale
            // auto-repeat path: if a focus event was missed and a key
            // was still recorded as "held", the very first frame after
            // open would otherwise re-fire that key (e.g. Enter →
            // launch Pin(0)) the instant the panel becomes visible.
            wl.held_key = None;
            wl.last_repeat = None;
            wl.pending_key = None;
            match cmd {
                Cmd::Toggle => app.toggle(),
                Cmd::Show => app.open(),
                Cmd::Hide => app.close(),
            }
        }

        // Refresh toplevel snapshot for the renderer.
        app.toplevels = wl.toplevels.toplevels();

        // Dispatch any pending window actions queued by click handlers.
        if !app.window_actions.is_empty() {
            for act in app.window_actions.drain(..) {
                use crate::app::WindowActionKind;
                match act.kind {
                    WindowActionKind::Activate => {
                        if let Some(seat) = wl.seat.as_ref() {
                            wl.toplevels.activate(&act.app_id, &act.title, seat);
                        }
                    }
                    WindowActionKind::Close => {
                        wl.toplevels.close(&act.app_id, &act.title);
                    }
                    WindowActionKind::Minimize => {
                        wl.toplevels.set_minimized(&act.app_id, &act.title, true);
                    }
                }
            }
        }

        // Sync input grab state with current visibility. We grab as soon
        // as we start opening (so typing during the open animation lands
        // in the search field, not the previously-focused window) and
        // release the moment we go fully hidden (so pointer events stop
        // hitting our invisible surface).
        // Only keep keyboard / pointer exclusivity while the panel is
        // actually visible (or opening). Releasing during Closing lets
        // the compositor transfer focus to whatever window the user
        // just clicked through to (via the `focus_at` IPC) instead of
        // forcing a second click after the animation finishes.
        let want_active = matches!(
            app.visibility,
            crate::app::Visibility::Visible | crate::app::Visibility::Opening,
        );
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

            // While hidden, still tick the bluetooth control so an
            // incoming-file request can wake the panel and switch us
            // into the BT view. Other controls don't need the wake-up
            // path so we keep this cheap and BT-specific.
            app.controls.bluetooth.tick();
            if app.controls.bluetooth.incoming_request.is_some() {
                tracing::info!("incoming BT file → auto-opening panel to BT view");
                app.mode = crate::app::PanelMode::Control(
                    crate::controls::TileId::Bluetooth,
                );
                app.open();
                continue;
            }

            // Keep the terminal grid live while the panel is hidden.
            // The PTY reader thread is always pulling bytes into its
            // channel — pumping them through the VTE here means
            // long-running commands (e.g. `yay -Syu`) stay current and
            // we don't flood the grid on next open.
            app.terminal.pump();

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

        // Tick the animation state machine + control backends (battery
        // sysfs poll, etc.). Both are cheap; rate limiting lives inside
        // each tile's `tick`.
        let was_hidden_before_tick = app.is_hidden();
        app.tick();
        // If `app.tick()` just flipped us from Closing → Hidden, the
        // close animation has fully drained. Skip the rest of the
        // render path for this iteration — we don't want to submit a
        // last-minute alpha-0 frame that could race with the
        // commit_transparent / null-buffer hide below. Doing both can
        // leave the compositor displaying a transparent (but still
        // present) surface — the "ghost" panel.
        if !was_hidden_before_tick && app.is_hidden() {
            tracing::debug!("close animation finished — committing null buffer");
            commit_transparent(&mut gpu, &surface);
            // Drop input grab immediately so the ghost surface can't
            // eat clicks even if the compositor is slow to unmap.
            set_active_input(&surface, &layer_surface, &empty_region, false);
            input_active = false;
            continue;
        }
        let bt_incoming_before = app.controls.bluetooth.incoming_request.is_some();
        app.controls.tick();
        // PTY housekeeping for the Terminal view. We spawn lazily on
        // first activation and resize whenever the body geometry
        // changes so the child shell reflows correctly.
        if app.panel_view == crate::app::PanelView::Terminal {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            // Single source of truth for cell metrics + grid size so the
            // PTY's wrap column matches what we actually paint.
            let (_, _, _, cols, rows) = crate::terminal::body_metrics(
                panel_rect, top_y, scale_f, app.config.text_size,
            );
            app.terminal.ensure_spawned(cols.max(20), rows.max(5));
        }
        // Drain any pending PTY output into the grid so new bytes
        // appear in the next render.
        app.terminal.pump();

        // Flush any queued PTY input (e.g. from Files "Open in Terminal
        // tab"). Only meaningful once the PTY has been spawned.
        if app.terminal.is_spawned() {
            if let Some(s) = app.pending_terminal_input.take() {
                app.terminal.write(s.as_bytes());
            }
        }
        // Sysmon is the one control we *want* to be completely silent
        // when the panel is closed — pass visibility through so it can
        // drop its polling state instead of running on a timer.
        // Keep sysmon polling while the panel is animating in/out too,
        // not just at the steady Visible state — otherwise the temp
        // icon + sparklines pop in a second after the open animation
        // (cache wiped, waiting for first sample) and disappear before
        // the close animation finishes (cache reset on transition).
        app.controls.sysmon.tick(!app.is_hidden());

        // Update WiFi row hover so the highlight tracks the cursor.
        // Cheap (a few rect tests) and only does work when the WiFi
        // view is open.
        if matches!(app.mode, crate::app::PanelMode::Control(crate::controls::TileId::Wifi)) {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            let new_hover = match crate::controls::wifi::hit_test_network(
                &app.controls.wifi, panel_rect, view_top_y, scale_f, phys_cx, phys_cy,
            ) {
                Some(crate::controls::wifi::NetworkHit::Row(s))
                | Some(crate::controls::wifi::NetworkHit::ConnectButton(s))
                | Some(crate::controls::wifi::NetworkHit::BandPill(s, _)) => Some(s),
                None => None,
            };
            if app.controls.wifi.hovered_ssid != new_hover {
                app.controls.wifi.hovered_ssid = new_hover;
            }
        } else if app.controls.wifi.hovered_ssid.is_some() {
            app.controls.wifi.hovered_ssid = None;
        }

        // Power-column hover tracking (buttons live to the right of the
        // panel). Skipped when the panel is collapsed — the column is
        // hidden then and we don't want a phantom hover ring.
        if app.collapsed || app.panel_view != crate::app::PanelView::Default {
            app.power_hover = None;
        } else {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            app.power_hover = crate::power::hit_test(panel_rect, scale_f, phys_cx, phys_cy);
        }

        // View-switcher arrow hover. Always tracked (arrows are now
        // anchored to the row so they work in collapsed mode too).
        {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            app.view_arrow_hover = crate::view_arrows::hit_test(panel_rect, scale_f, phys_cx, phys_cy);
            app.home_hover = crate::view_indicator::hit_home(panel_rect, scale_f, phys_cx, phys_cy);
            app.grow_hover = crate::view_indicator::hit_grow(panel_rect, scale_f, phys_cx, phys_cy);
            app.gear_hover = crate::view_indicator::hit_gear(panel_rect, scale_f, phys_cx, phys_cy);
            app.emoji_hover = crate::view_indicator::hit_emoji(panel_rect, scale_f, phys_cx, phys_cy);
            app.clipboard_hover = crate::view_indicator::hit_clipboard(panel_rect, scale_f, phys_cx, phys_cy);
            app.notes_hover = crate::view_indicator::hit_notes(panel_rect, scale_f, phys_cx, phys_cy);
            // Emoji overlay hover routing.
            if app.emojis.open {
                let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                let panel_bottom = panel_rect.y + panel_rect.h;
                let hit = crate::emojis::hit_test(
                    &app.emojis, panel_rect, top_y, scale_f, panel_bottom, phys_cx, phys_cy,
                );
                app.emojis.hover_entry = None;
                app.emojis.hover_category = None;
                match hit {
                    crate::emojis::Hit::Cell(idx) => app.emojis.hover_entry = Some(idx),
                    crate::emojis::Hit::Category(c) => app.emojis.hover_category = Some(c),
                    _ => {}
                }
            } else {
                app.emojis.hover_entry = None;
                app.emojis.hover_category = None;
            }
            app.hovered_control_tile = app.controls.hit_test(
                panel_rect, scale_f, phys_cx, phys_cy, app.panel_view,
            );
            // Waffle "all apps" button hover (search row).
            let waffle = crate::search::input::waffle_rect(panel_rect, scale_f);
            app.waffle_hover = phys_cx >= waffle.x
                && phys_cx <= waffle.x + waffle.w
                && phys_cy >= waffle.y
                && phys_cy <= waffle.y + waffle.h;

            // Files view hover tracking — toolbar lives in the controls row,
            // sidebar + list live in the body.
            app.files.hover_nav = None;
            app.files.hover_entry = None;
            app.files.hover_crumb = None;
            app.files.hover_location = None;
            if app.panel_view == crate::app::PanelView::Files {
                let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                let strip_hit = files_strip_rect(&app, panel_rect, scale_f).map(|strip| {
                    crate::files::hit_strip(&app.files, strip, scale_f, phys_cx, phys_cy)
                });
                match strip_hit {
                    Some(crate::files::FilesHit::Nav(b)) => app.files.hover_nav = Some(b),
                    Some(crate::files::FilesHit::Crumb(i)) => app.files.hover_crumb = Some(i),
                    _ => {
                        match crate::files::hit_body(
                            &app.files, panel_rect, top_y, scale_f,
                            app.config.text_size, phys_cx, phys_cy,
                        ) {
                            crate::files::FilesHit::Sidebar(l) => app.files.hover_location = Some(l),
                            crate::files::FilesHit::Entry(i) => app.files.hover_entry = Some(i),
                            _ => {}
                        }
                    }
                }
            }
        }

        // Mini-dock hover tracking — only meaningful while the dock is
        // actually rendered: Default view AND collapse-progress > 0.
        // The hover index sticks while the cursor is over the preview
        // tile so users can move down onto it without it disappearing.
        if app.collapse_progress() <= 0.005
            || app.panel_view != crate::app::PanelView::Default
        {
            app.mini_dock_hover = None;
        } else {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            let pin_count = app.launcher.pinned_entries(&app.apps).len();
            let icon_hit = crate::mini_dock::hit_test(
                panel_rect, scale_f, pin_count, phys_cx, phys_cy,
            );
            if icon_hit.is_some() {
                app.mini_dock_hover = icon_hit;
            } else if let Some(prev) = app.mini_dock_hover {
                // Generous zone covering icon + every preview tile so
                // the user can drag the cursor down into any one of
                // the per-window thumbnails without the preview
                // disappearing under them.
                let pinned = app.launcher.pinned_entries(&app.apps);
                let nwin = pinned
                    .get(prev)
                    .map(|e| crate::mini_dock::windows_for_app(&app.toplevels, &e.app_id).len())
                    .unwrap_or(0)
                    .max(1);
                let still_in_zone = crate::mini_dock::hit_test_preview_zone(
                    panel_rect, scale_f, pin_count, prev, nwin, phys_cx, phys_cy,
                );
                if !still_in_zone {
                    app.mini_dock_hover = None;
                }
            }
        }
        let bt_incoming_after = app.controls.bluetooth.incoming_request.is_some();
        // Fresh incoming-file request → jump straight to the BT view so
        // the modal isn't hidden behind whatever the user was looking at.
        if bt_incoming_after && !bt_incoming_before {
            tracing::info!("incoming BT file while panel visible → switching to BT view");
            app.mode =
                crate::app::PanelMode::Control(crate::controls::TileId::Bluetooth);
        }

        // Handle Esc → close.
        if wl.esc_pressed {
            wl.esc_pressed = false;
            tracing::debug!(?app.mode, "Esc pressed");
            app.handle_esc();
        }

        // Drain accumulated scroll delta and apply to the launcher
        // result list when there's a query. We only scroll the result
        // list — control views handle their own scrolling if/when they
        // need it (none do today). Always reset the accumulator so
        // stale deltas don't leak into the next mode.
        if wl.scroll_delta_v.abs() > 0.0 {
            let dy = wl.scroll_delta_v as f32;
            wl.scroll_delta_v = 0.0;
            if app.emojis.open {
                let scale_f = wl.fractional_scale() as f32;
                let phys_w = wl.phys_width().max(1);
                let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
                let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                let panel_bottom = panel_rect.y + panel_rect.h;
                let grid = crate::emojis::grid_rect(panel_rect, top_y, scale_f, panel_bottom);
                let visible = app.emojis.visible_indices();
                let per_row = crate::emojis::cells_per_row(grid, scale_f);
                let max = crate::emojis::max_scroll(visible.len(), per_row, grid.h, scale_f) / scale_f;
                let new_offset = (app.emojis.scroll + dy).clamp(0.0, max);
                app.emojis.scroll = new_offset;
            } else if app.panel_view == crate::app::PanelView::Terminal {
                // libinput reports negative dy on wheel-up (toward
                // older history) and positive on wheel-down. Convert
                // to lines via a soft divisor so one notch ≈ 3 lines.
                let lines = (-dy / 16.0).round() as i32;
                if lines != 0 {
                    app.terminal.scroll_by(lines);
                }
            } else if app.panel_view == crate::app::PanelView::Files {
                let scale_f = wl.fractional_scale() as f32;
                let phys_w = wl.phys_width().max(1);
                let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
                let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                let max = crate::files::max_scroll(
                    &app.files, panel_rect, top_y, scale_f, app.config.text_size,
                );
                let new_offset = (app.files.scroll + dy * scale_f).clamp(0.0, max);
                app.files.scroll = new_offset;
            } else if matches!(app.mode, crate::app::PanelMode::Launcher)
                && (!app.search.input.is_empty() || app.search.all_apps_mode)
                && !app.search.results().is_empty()
            {
                let scale_f = wl.fractional_scale() as f32;
                let phys_w = wl.phys_width().max(1);
                let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
                let panel_rect =
                    lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                let max = crate::search::max_scroll(&app.search, panel_rect, scale_f);
                // libinput hi-res wheel reports ~15 per notch, low-res ~10.
                // Scaling by scale_f keeps "one notch ≈ one row" feel.
                let new_offset = (app.search.scroll_offset + dy * scale_f).clamp(0.0, max);
                app.search.scroll_offset = new_offset;
            }
        }

        // Dispatch the next pending keypress.
        //
        // Routing priority:
        //   1. WiFi password modal — typed chars into its buffer; Enter submits.
        //   2. BT pair-prompt modal — depends on prompt kind:
        //        Confirm/Authorize → Enter = Yes, no other typing accepted.
        //        Enter passkey → typed chars into the passkey buffer; Enter submits.
        //   3. Launcher-mode navigation (Up/Down/Left/Right/Enter).
        //   4. Else: key falls through to the launcher search input.
        // Key auto-repeat: hold any key past `REPEAT_DELAY` and we
        // synthesize fresh pending-key events at `REPEAT_INTERVAL`.
        if wl.pending_key.is_none() {
            if let Some((held, pressed_at)) = wl.held_key {
                const REPEAT_DELAY: std::time::Duration = std::time::Duration::from_millis(420);
                const REPEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(32);
                let now = std::time::Instant::now();
                if now.duration_since(pressed_at) >= REPEAT_DELAY {
                    let ready = match wl.last_repeat {
                        None => true,
                        Some(last) => now.duration_since(last) >= REPEAT_INTERVAL,
                    };
                    if ready {
                        wl.pending_key = Some(held);
                        wl.last_repeat = Some(now);
                    }
                }
            }
        }

        if let Some(key) = wl.pending_key.take() {
            use crate::search::input::*;
            use crate::controls::bluetooth::PairPromptKind;

            if app.controls.wifi.prompt.is_some() {
                match key {
                    KEY_ENTER | KEY_KP_ENTER => app.controls.wifi.submit_prompt(),
                    other => {
                        if let Some(prompt) = app.controls.wifi.prompt.as_mut() {
                            let _ = prompt.input.on_key(other, wl.shift_held);
                        }
                    }
                }
            } else if let Some(kind) = app
                .controls
                .bluetooth
                .pair_prompt
                .as_ref()
                .map(|p| p.kind.clone())
            {
                match (key, kind) {
                    (KEY_ENTER | KEY_KP_ENTER, PairPromptKind::Confirm(_))
                    | (KEY_ENTER | KEY_KP_ENTER, PairPromptKind::Authorize(_)) => {
                        app.controls.bluetooth.pair_confirm_yes();
                    }
                    (KEY_ENTER | KEY_KP_ENTER, PairPromptKind::Enter) => {
                        app.controls.bluetooth.pair_submit_passkey();
                    }
                    (other, PairPromptKind::Enter) => {
                        if let Some(prompt) = app.controls.bluetooth.pair_prompt.as_mut() {
                            let _ = prompt.passkey_input.on_key(other, wl.shift_held);
                        }
                    }
                    _ => {
                        // Confirm/Authorize: only Enter is accepted as
                        // a key shortcut. Y/N typing isn't wired (yet).
                    }
                }
            } else if app.emojis.open {
                // Emojis overlay: filter input always focused. Enter copies
                // the first visible emoji. Tab cycles category. Esc handled
                // elsewhere.
                match key {
                    KEY_ENTER | KEY_KP_ENTER => {
                        let visible = app.emojis.visible_indices();
                        if let Some(&entry_idx) = visible.first() {
                            let entry = &crate::emojis::data::EMOJIS[entry_idx];
                            if let Some(clip) = app.clipboard.as_ref() {
                                clip.set_text(entry.glyph);
                            }
                            app.emojis.recent_copy = Some((0, std::time::Instant::now()));
                        }
                    }
                    other => {
                        let _ = app.emojis.filter.on_key(other, wl.shift_held);
                        app.emojis.reset_scroll();
                    }
                }
            } else if app.panel_view == crate::app::PanelView::Terminal {
                // Copy / Paste chord keys take priority over normal PTY
                // input. Ctrl-Shift-C copies the current selection;
                // Ctrl-Shift-V pastes clipboard into the PTY. These
                // shadow xterm's `Ctrl-Shift-{C,V}` convention.
                use crate::search::input::keycode_to_char;
                let chord = wl.ctrl_held && wl.shift_held;
                let ch = keycode_to_char(key, false).map(|c| c.to_ascii_lowercase());
                if chord && ch == Some('c') {
                    if app.terminal.copy_selection() {
                        tracing::info!("terminal: copied selection");
                    }
                } else if chord && ch == Some('v') {
                    if app.terminal.paste_from_clipboard() {
                        tracing::info!("terminal: pasted from clipboard");
                    }
                } else if let Some(bytes) =
                    crate::terminal::keycode_to_bytes(key, wl.shift_held, wl.ctrl_held)
                {
                    app.terminal.write(&bytes);
                }
            } else if matches!(
                app.mode,
                crate::app::PanelMode::Control(crate::controls::TileId::SysMon)
                    | crate::app::PanelMode::Control(crate::controls::TileId::Temp)
            ) {
                // Sysmon process filter: typed chars edit the buffer.
                // Esc (handled above via esc_pressed) clears it; Enter
                // and arrow keys are inert here.
                if !matches!(key, KEY_ENTER | KEY_KP_ENTER | KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT) {
                    let _ = app.controls.sysmon.filter.on_key(key, wl.shift_held);
                }
            } else if app.controls.clock.add_event_input.is_some()
                && matches!(app.mode, crate::app::PanelMode::Control(crate::controls::TileId::Clock))
            {
                // Calendar add-event input has focus.
                match key {
                    KEY_ENTER | KEY_KP_ENTER => {
                        if let (Some(date), Some(input)) = (
                            app.controls.clock.selected_day,
                            app.controls.clock.add_event_input.as_ref(),
                        ) {
                            let title = input.query().to_string();
                            if !title.trim().is_empty() {
                                app.controls.events.add(date, title);
                            }
                        }
                        app.controls.clock.add_event_input = None;
                    }
                    other => {
                        if let Some(input) = app.controls.clock.add_event_input.as_mut() {
                            let _ = input.on_key(other, wl.shift_held);
                        }
                    }
                }
            } else if app.panel_view == crate::app::PanelView::Files {
                // Files view: route to the filter input when active, else
                // typing auto-activates filter mode. Ctrl+H toggles hidden
                // files; Esc exits filter mode.
                use crate::search::input::keycode_to_char;
                let chord_ctrl_h = wl.ctrl_held
                    && keycode_to_char(key, false).map(|c| c.to_ascii_lowercase()) == Some('h');
                if chord_ctrl_h {
                    app.files.toggle_hidden();
                } else {
                    match key {
                        KEY_LEFT if !app.files.filter_active => app.cycle_view_prev(),
                        KEY_RIGHT if !app.files.filter_active => app.cycle_view_next(),
                        KEY_ENTER | KEY_KP_ENTER if app.files.filter_active => {
                            if let Some(entry) = app.files.entry_for_visible(0).cloned() {
                                if entry.is_dir {
                                    app.files.navigate_to(&entry.path);
                                } else {
                                    let exec = format!(
                                        "xdg-open '{}'",
                                        entry.path.to_string_lossy().replace('\'', "'\\''"),
                                    );
                                    crate::app::spawn_detached(&exec);
                                    app.close();
                                }
                            }
                        }
                        other => {
                            // If a printable character was pressed and we're
                            // not in filter mode yet, auto-activate so the
                            // first letter shows up in the input.
                            let printable = keycode_to_char(other, wl.shift_held).is_some();
                            if printable && !app.files.filter_active {
                                app.files.activate_filter();
                            }
                            if app.files.filter_active {
                                let _ = app.files.filter.on_key(other, wl.shift_held);
                                app.files.apply_filter();
                            }
                        }
                    }
                }
            } else {
                match key {
                    KEY_UP => app.select_up(),
                    KEY_DOWN => app.select_down(),
                    // Left / Right cycle the panel-view tabs (Default ↔
                    // Terminal). Only fires when the search field is
                    // empty, so typing letters into the launcher still
                    // moves the text cursor instead of switching tabs.
                    KEY_LEFT if app.search.input.is_empty() => app.cycle_view_prev(),
                    KEY_RIGHT if app.search.input.is_empty() => app.cycle_view_next(),
                    KEY_ENTER | KEY_KP_ENTER => {
                        app.launch_selected();
                    }
                    _ => {
                        let _ = app.forward_key(key, wl.shift_held);
                    }
                }
            }
        }

        // Terminal selection: press starts a drag-select inside the body,
        // motion extends it, release finalizes it. We consume `left_clicked`
        // here so the launcher hit-tests below don't also fire.
        if app.panel_view == crate::app::PanelView::Terminal {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            let (g_cols, g_rows) = app
                .terminal
                .grid()
                .map(|g| (g.cols, g.rows))
                .unwrap_or((0, 0));
            let cell = crate::terminal::cell_at(
                panel_rect, top_y, scale_f, app.config.text_size,
                g_cols, g_rows, phys_cx, phys_cy,
            );
            if wl.left_clicked {
                if let Some((row, col)) = cell {
                    // Right-click context menu eats the click — only start
                    // a fresh selection when the body itself was clicked
                    // with the LEFT button.
                    if app.context_menu.is_none() {
                        app.terminal.begin_selection(row, col);
                        wl.left_clicked = false;
                    }
                } else {
                    // Click outside the terminal body — drop any selection
                    // so the highlight doesn't linger after a chrome click.
                    app.terminal.clear_selection();
                }
            } else if app.terminal.selecting && wl.left_held {
                if let Some((row, col)) = cell {
                    app.terminal.update_selection(row, col);
                }
            }
            if wl.left_released_this_frame && app.terminal.selecting {
                app.terminal.end_selection();
            }
        }

        // Files-view click: toolbar (controls row) + body (sidebar + list).
        if app.panel_view == crate::app::PanelView::Files && wl.left_clicked
            && app.context_menu.is_none()
        {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;

            // Toolbar strip in the top-most row takes precedence.
            let strip_hit = files_strip_rect(&app, panel_rect, scale_f).map(|s| {
                crate::files::hit_strip(&app.files, s, scale_f, phys_cx, phys_cy)
            });
            if let Some(hit) = strip_hit {
                match hit {
                    crate::files::FilesHit::Nav(crate::files::NavButton::Back) => {
                        app.files.go_back();
                        wl.left_clicked = false;
                        continue;
                    }
                    crate::files::FilesHit::Nav(crate::files::NavButton::ToggleHidden) => {
                        app.files.toggle_hidden();
                        wl.left_clicked = false;
                        continue;
                    }
                    crate::files::FilesHit::Nav(crate::files::NavButton::Magnifier) => {
                        app.files.toggle_filter();
                        if app.files.filter_active && app.collapsed {
                            app.toggle_collapsed();
                        }
                        wl.left_clicked = false;
                        continue;
                    }
                    crate::files::FilesHit::Nav(crate::files::NavButton::Sort) => {
                        let sort_r = crate::files::strip_layout(
                            files_strip_rect(&app, panel_rect, scale_f).unwrap_or(panel_rect),
                            scale_f,
                        )
                        .sort;
                        let anchor_x = sort_r.x;
                        let anchor_y = sort_r.y + sort_r.h + 6.0 * scale_f;
                        app.context_menu = Some(crate::launcher::context_menu::ContextMenu {
                            app_id: String::new(),
                            window_title: String::new(),
                            anchor_x,
                            anchor_y,
                            items: sort_menu_items(&app.files),
                        });
                        wl.left_clicked = false;
                        continue;
                    }
                    crate::files::FilesHit::Crumb(idx) => {
                        if let Some(p) = app.files.crumb_path(idx) {
                            if p != app.files.cwd && p.is_dir() {
                                app.files.navigate_to(&p);
                            }
                        }
                        wl.left_clicked = false;
                        continue;
                    }
                    crate::files::FilesHit::Pathbar => {
                        // Click on the pathbar while in filter mode just
                        // keeps focus (no-op). While in breadcrumb mode this
                        // arm isn't reached — Crumb is returned instead.
                        wl.left_clicked = false;
                        continue;
                    }
                    _ => {}
                }
            }

            // Body: sidebar + list.
            match crate::files::hit_body(
                &app.files, panel_rect, top_y, scale_f,
                app.config.text_size, phys_cx, phys_cy,
            ) {
                crate::files::FilesHit::Sidebar(loc) => {
                    let p = loc.path();
                    if p.is_dir() {
                        app.files.navigate_to(&p);
                    }
                    wl.left_clicked = false;
                }
                crate::files::FilesHit::Entry(idx) => {
                    if let Some(entry) = app.files.entry_for_visible(idx).cloned() {
                        if entry.is_dir {
                            app.files.navigate_to(&entry.path);
                        } else {
                            let exec = format!(
                                "xdg-open '{}'",
                                entry.path.to_string_lossy().replace('\'', "'\\''"),
                            );
                            crate::app::spawn_detached(&exec);
                            app.close();
                        }
                    }
                    wl.left_clicked = false;
                }
                _ => {}
            }
        }

        // Handle left-click: if outside the panel rect → close, otherwise
        // hit-test against the launcher and launch if a tile/row was hit.
        if wl.left_clicked {
            wl.left_clicked = false;
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);

            // Context menu intercepts every left-click while open: a
            // click on an item runs that action; anywhere else dismisses.
            let menu_consumed = if let Some(menu) = app.context_menu.clone() {
                if let Some(action) = crate::launcher::context_menu::hit_test(
                    &menu, panel_rect, scale_f, phys_cx, phys_cy,
                ) {
                    app.run_menu_action(action);
                } else {
                    app.context_menu = None;
                }
                true
            } else if let Some(event_menu) = app.controls.clock.event_menu.clone() {
                // Event-row menu intercepts: Delete row → remove event.
                if crate::controls::clock::event_menu_hit_delete(
                    &event_menu, panel_rect, scale_f, phys_cx, phys_cy,
                ) {
                    app.controls
                        .events
                        .remove_at(event_menu.date, event_menu.idx_in_date);
                }
                app.controls.clock.event_menu = None;
                true
            } else {
                false
            };

            // First: if a control's full-content view is up, see if the
            // click hit one of its interactive widgets (battery toggle,
            // audio slider, audio device list). Skip when an overlay
            // (context menu, power confirm) has the click.
            let consumed_by_view = !menu_consumed && app.power_confirm.is_none()
                && handle_control_view_click(
                    &mut app, &mut text, panel_rect, scale_f, phys_cx, phys_cy,
                );

            // Power confirm modal intercepts every click while open.
            let confirm_consumed = if let Some(pending) = app.power_confirm {
                let surface_w_px = wl.phys_width().max(1);
                let surface_h_px = wl.phys_height().max(1);
                match crate::power::hit_test_confirm(
                    surface_w_px, surface_h_px, scale_f, phys_cx, phys_cy,
                ) {
                    Some(crate::power::ConfirmHit::Confirm) => {
                        tracing::debug!(?pending, "power confirm → run");
                        crate::power::run(pending);
                        app.power_confirm = None;
                        app.close();
                    }
                    Some(crate::power::ConfirmHit::Cancel) => {
                        tracing::debug!(?pending, "power confirm → cancel");
                        app.power_confirm = None;
                    }
                    Some(crate::power::ConfirmHit::CardBody) => {
                        // Click inside the card but missed the buttons — eat it.
                    }
                    None => {
                        // Click outside the card → treat as cancel.
                        tracing::debug!(?pending, "power confirm → click-out cancel");
                        app.power_confirm = None;
                    }
                }
                true
            } else {
                false
            };

            let power_btn = if app.collapsed || app.panel_view != crate::app::PanelView::Default {
                None
            } else {
                crate::power::hit_test(panel_rect, scale_f, phys_cx, phys_cy)
            };
            let arrow_hit = crate::view_arrows::hit_test(panel_rect, scale_f, phys_cx, phys_cy);
            let home_hit = crate::view_indicator::hit_home(panel_rect, scale_f, phys_cx, phys_cy);
            let grow_hit = crate::view_indicator::hit_grow(panel_rect, scale_f, phys_cx, phys_cy);
            let gear_hit = crate::view_indicator::hit_gear(panel_rect, scale_f, phys_cx, phys_cy);
            let emoji_btn_hit = crate::view_indicator::hit_emoji(panel_rect, scale_f, phys_cx, phys_cy);
            let dot_hit = crate::view_indicator::hit_test_dot(panel_rect, scale_f, phys_cx, phys_cy);
            // Mini-dock hit-tests — try preview-tile first (when hover
            // points to an app with open windows) then the icon body.
            // (pinned_idx, window_idx_within_app, hit_kind)
            let mut dock_preview_hit: Option<(usize, usize, crate::mini_dock::PreviewHit)> = None;
            let mut dock_pin: Option<usize> = None;
            // Dock is only visible (and clickable) in the Default view.
            if app.collapse_progress() > 0.005
                && app.panel_view == crate::app::PanelView::Default
            {
                let pinned = app.launcher.pinned_entries(&app.apps);
                // Preview is only valid in the Default view — other
                // top-level views own the body and shouldn't be
                // covered by the dock preview tile.
                let preview_enabled = app.panel_view == crate::app::PanelView::Default;
                if preview_enabled {
                    if let Some(hover_idx) = app.mini_dock_hover {
                        if let Some(entry) = pinned.get(hover_idx) {
                            let windows = crate::mini_dock::windows_for_app(
                                &app.toplevels, &entry.app_id,
                            );
                            if !windows.is_empty() {
                                dock_preview_hit = crate::mini_dock::hit_test_preview(
                                    panel_rect, scale_f, pinned.len(), hover_idx,
                                    windows.len(), phys_cx, phys_cy,
                                )
                                .map(|(wi, h)| (hover_idx, wi, h));
                            }
                        }
                    }
                }
                if dock_preview_hit.is_none() {
                    dock_pin = crate::mini_dock::hit_test(
                        panel_rect, scale_f, pinned.len(), phys_cx, phys_cy,
                    );
                }
            }
            if menu_consumed || confirm_consumed {
                // Already handled by an overlay — fall through to render.
            } else if let Some(action) = power_btn {
                tracing::debug!(?action, "power button click → open confirm modal");
                app.power_confirm = Some(action);
            } else if app.settings_open && {
                // While the settings page is open all clicks inside the
                // panel route to the settings hit-test first.
                let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                let rows = crate::settings::layout(panel_rect, top_y, scale_f);
                if let Some(hit) = crate::settings::hit_test(&rows, phys_cx, phys_cy) {
                    match hit {
                        crate::settings::SettingHit::Toggle(key) => {
                            let cur = crate::settings::current_value(&app.config, key);
                            if let crate::settings::SettingValue::B(b) = cur {
                                crate::settings::apply_value(
                                    &mut app.config,
                                    key,
                                    crate::settings::SettingValue::B(!b),
                                );
                                app.config.save();
                            }
                        }
                        crate::settings::SettingHit::SliderSeek(key, value) => {
                            crate::settings::apply_value(
                                &mut app.config,
                                key,
                                crate::settings::SettingValue::F(value),
                            );
                            app.config.save();
                            app.settings_drag = Some(key);
                        }
                    }
                    true
                } else {
                    false
                }
            } {
                // Already handled.
            } else if gear_hit {
                // Settings page is only visible in the expanded panel —
                // un-collapse first so opening settings while collapsed
                // actually shows the page.
                if app.collapsed && !app.settings_open {
                    app.toggle_collapsed();
                }
                app.toggle_settings();
            } else if emoji_btn_hit {
                if app.collapsed && !app.emojis.open {
                    app.toggle_collapsed();
                }
                app.toggle_emojis();
            } else if app.emojis.open && {
                let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                let panel_bottom = panel_rect.y + panel_rect.h;
                let hit = crate::emojis::hit_test(
                    &app.emojis, panel_rect, top_y, scale_f, panel_bottom, phys_cx, phys_cy,
                );
                match hit {
                    crate::emojis::Hit::None => false,
                    crate::emojis::Hit::Filter => {
                        // Click on filter: just focus (already focused). No-op.
                        true
                    }
                    crate::emojis::Hit::Category(c) => {
                        app.emojis.active_category = c;
                        app.emojis.reset_scroll();
                        true
                    }
                    crate::emojis::Hit::Cell(visible_idx) => {
                        let visible = app.emojis.visible_indices();
                        if let Some(&entry_idx) = visible.get(visible_idx) {
                            let entry = &crate::emojis::data::EMOJIS[entry_idx];
                            if let Some(clip) = app.clipboard.as_ref() {
                                clip.set_text(entry.glyph);
                            }
                            app.emojis.recent_copy = Some((visible_idx, std::time::Instant::now()));
                        }
                        true
                    }
                }
            } {
                // Already handled by emojis overlay.
            } else if home_hit {
                app.set_view(crate::app::PanelView::Default);
            } else if grow_hit {
                app.toggle_grow();
            } else if let Some(dot_idx) = dot_hit {
                if let Some(v) = crate::app::PanelView::ALL.get(dot_idx) {
                    app.set_view(*v);
                }
            } else if let Some(side) = arrow_hit {
                match side {
                    crate::view_arrows::Side::Left => app.cycle_view_prev(),
                    crate::view_arrows::Side::Right => app.cycle_view_next(),
                }
            } else if let Some((idx, window_idx, hit)) = dock_preview_hit {
                let pinned = app.launcher.pinned_entries(&app.apps);
                if let Some(entry) = pinned.get(idx).map(|e| (*e).clone()) {
                    let windows = crate::mini_dock::windows_for_app(
                        &app.toplevels, &entry.app_id,
                    );
                    if let Some(target) = windows.get(window_idx).copied() {
                        match hit {
                            crate::mini_dock::PreviewHit::Body => {
                                tracing::debug!(
                                    app_id = %entry.app_id, window_idx,
                                    "dock preview → activate"
                                );
                                app.window_actions.push(crate::app::WindowAction {
                                    app_id: target.app_id.clone(),
                                    title: target.title.clone(),
                                    kind: crate::app::WindowActionKind::Activate,
                                });
                                app.close();
                            }
                            crate::mini_dock::PreviewHit::Close => {
                                tracing::debug!(
                                    app_id = %entry.app_id, window_idx,
                                    "dock preview → close"
                                );
                                app.window_actions.push(crate::app::WindowAction {
                                    app_id: target.app_id.clone(),
                                    title: target.title.clone(),
                                    kind: crate::app::WindowActionKind::Close,
                                });
                            }
                        }
                    }
                }
            } else if let Some(pin_idx) = dock_pin {
                tracing::debug!(pin = pin_idx, "mini-dock click → launch");
                app.activate_at(crate::app::HitTarget::Pin(pin_idx));
            } else if !panel.contains(phys_cx, phys_cy) {
                tracing::debug!(
                    cursor = ?(phys_cx, phys_cy),
                    panel = ?(panel.x, panel.y, panel.w, panel.h),
                    "click outside panel → close + focus underlying window"
                );
                // Ask the compositor to focus whatever toplevel is at
                // the click point so the user doesn't need a second
                // click to start typing in it. cursor_x/y from wayland
                // are already in surface-logical pixels (== compositor
                // logical for a single-output, fullscreen layer surface).
                let logical_x = wl.cursor_x.round() as i32;
                let logical_y = wl.cursor_y.round() as i32;
                thumbs.focus_at(logical_x, logical_y);
                app.close();
            } else if consumed_by_view {
                // Already handled.
            } else if let Some(tile_id) = app.controls.hit_test(
                panel_rect,
                scale_f,
                phys_cx,
                phys_cy,
                app.panel_view,
            ) {
                match tile_id {
                    crate::controls::TileId::Collapse => {
                        tracing::debug!("collapse chevron click → toggle");
                        app.toggle_collapsed();
                    }
                    crate::controls::TileId::TerminalClear => {
                        app.terminal.clear();
                    }
                    _ => {
                        tracing::debug!(?tile_id, "left-click on controls tile → switch view");
                        app.show_control(tile_id);
                    }
                }
            } else if matches!(app.mode, crate::app::PanelMode::Launcher)
                && app.panel_view == crate::app::PanelView::Default
            {
                // Waffle "all apps" button on the search bar.
                let waffle = crate::search::input::waffle_rect(panel_rect, scale_f);
                if phys_cx >= waffle.x
                    && phys_cx <= waffle.x + waffle.w
                    && phys_cy >= waffle.y
                    && phys_cy <= waffle.y + waffle.h
                {
                    tracing::debug!("waffle click → show all apps");
                    if app.search.all_apps_mode {
                        // Toggle off — back to pinned launcher.
                        app.search.reset();
                    } else {
                        app.search.show_all_apps(&app.apps, app.launcher.hidden());
                    }
                } else if let Some(target) = app.hit_test_launcher(panel, scale_f, phys_cx, phys_cy) {
                    // Pin tiles defer activation until release — that way
                    // a press-and-drag can reorder instead of launching.
                    if let crate::app::HitTarget::Pin(i) = target {
                        tracing::debug!(pin = i, "pin press → start drag candidate");
                        app.pin_drag = Some(crate::app::PinDrag {
                            from_idx: i,
                            press_x: phys_cx,
                            press_y: phys_cy,
                            current_x: phys_cx,
                            current_y: phys_cy,
                            started: false,
                        });
                    } else {
                        tracing::debug!(?target, "left-click on launcher entry → activate");
                        app.activate_at(target);
                    }
                }
            }
            // Click inside the panel but not on a clickable entity is
            // a no-op.
        }

        // Pin drag — update current cursor each frame, promote to
        // "started" once movement exceeds the threshold.
        if let Some(drag) = app.pin_drag.as_mut() {
            let scale_f = wl.fractional_scale() as f32;
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            drag.current_x = phys_cx;
            drag.current_y = phys_cy;
            if !drag.started {
                let dx = phys_cx - drag.press_x;
                let dy = phys_cy - drag.press_y;
                let threshold = crate::app::PIN_DRAG_THRESHOLD * scale_f;
                if (dx * dx + dy * dy).sqrt() > threshold {
                    drag.started = true;
                }
            }
        }

        // Pin drag — release commits a reorder when the drag actually
        // started, otherwise treats it as a plain click on the pin.
        if wl.left_released_this_frame {
            wl.left_released_this_frame = false;
            if let Some(drag) = app.pin_drag.take() {
                let scale_f = wl.fractional_scale() as f32;
                let phys_w = wl.phys_width().max(1);
                let panel = PanelRect::compute_with_dims(
                    phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
                );
                let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                if drag.started {
                    let pin_top_y = panel_rect.y
                        + crate::controls::total_logical_height() * scale_f
                        + (crate::search::input::SEARCH_HORIZONTAL_PAD * 0.5
                            + crate::search::input::SEARCH_ROW_HEIGHT)
                            * scale_f;
                    let row_top = crate::launcher::pins_row_top_y(pin_top_y, scale_f);
                    let num_pins = app.launcher.pinned_entries(&app.apps).len();
                    let to = crate::launcher::pin_drop_slot(
                        panel_rect, scale_f, row_top, num_pins,
                        drag.current_x, drag.current_y,
                    );
                    tracing::info!(from = drag.from_idx, to, "pin drag commit");
                    app.launcher.reorder_pins(drag.from_idx, to, &app.apps);
                } else {
                    // Treat as a plain click on the pin.
                    app.activate_at(crate::app::HitTarget::Pin(drag.from_idx));
                }
            }
        }

        // Right-click:
        //   • Launcher mode: toggle pin/unpin on the hovered tile/row.
        if wl.right_clicked {
            wl.right_clicked = false;
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            match app.mode {
                crate::app::PanelMode::Launcher
                    if app.panel_view == crate::app::PanelView::Default =>
                {
                    if let Some(target) =
                        app.hit_test_launcher(panel, scale_f, phys_cx, phys_cy)
                    {
                        tracing::debug!(?target, "right-click → open context menu");
                        app.open_context_menu_at(target, phys_cx, phys_cy);
                    } else {
                        app.context_menu = None;
                    }
                }
                crate::app::PanelMode::Launcher
                    if app.panel_view == crate::app::PanelView::Terminal =>
                {
                    // Right-click in the terminal body opens a small
                    // Copy / Paste / Clear menu anchored at the cursor.
                    let panel_rect =
                        lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                    let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                    let (g_cols, g_rows) = app
                        .terminal
                        .grid()
                        .map(|g| (g.cols, g.rows))
                        .unwrap_or((0, 0));
                    if crate::terminal::cell_at(
                        panel_rect, top_y, scale_f, app.config.text_size,
                        g_cols, g_rows, phys_cx, phys_cy,
                    )
                    .is_some()
                    {
                        let mut items = Vec::new();
                        if app.terminal.has_selection() {
                            items.push(crate::launcher::context_menu::MenuItem {
                                label: "Copy".into(),
                                action: crate::launcher::context_menu::MenuAction::TerminalCopy,
                            });
                            items.push(crate::launcher::context_menu::MenuItem {
                                label: "Clear selection".into(),
                                action:
                                    crate::launcher::context_menu::MenuAction::TerminalClearSelection,
                            });
                        }
                        items.push(crate::launcher::context_menu::MenuItem {
                            label: "Paste".into(),
                            action: crate::launcher::context_menu::MenuAction::TerminalPaste,
                        });
                        app.context_menu =
                            Some(crate::launcher::context_menu::ContextMenu {
                                app_id: String::new(),
                                window_title: String::new(),
                                anchor_x: phys_cx,
                                anchor_y: phys_cy,
                                items,
                            });
                    } else {
                        app.context_menu = None;
                    }
                }
                crate::app::PanelMode::Launcher
                    if app.panel_view == crate::app::PanelView::Files =>
                {
                    let panel_rect =
                        lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                    let top_y = crate::controls::content_top_y(panel_rect, scale_f);
                    if let crate::files::FilesHit::Entry(idx) = crate::files::hit_body(
                        &app.files, panel_rect, top_y, scale_f,
                        app.config.text_size, phys_cx, phys_cy,
                    ) {
                        if let Some(entry) = app.files.entry_for_visible(idx) {
                            let path_str = entry.path.to_string_lossy().into_owned();
                            let items = vec![
                                crate::launcher::context_menu::MenuItem {
                                    label: "Open".into(),
                                    action: crate::launcher::context_menu::MenuAction::FilesOpen,
                                },
                                crate::launcher::context_menu::MenuItem {
                                    label: "Open in Terminal".into(),
                                    action: crate::launcher::context_menu::MenuAction::FilesOpenInTerminal,
                                },
                                crate::launcher::context_menu::MenuItem {
                                    label: "Reveal in File Manager".into(),
                                    action: crate::launcher::context_menu::MenuAction::FilesRevealInFM,
                                },
                                crate::launcher::context_menu::MenuItem {
                                    label: "Copy path".into(),
                                    action: crate::launcher::context_menu::MenuAction::FilesCopyPath,
                                },
                            ];
                            app.context_menu =
                                Some(crate::launcher::context_menu::ContextMenu {
                                    app_id: path_str,
                                    window_title: String::new(),
                                    anchor_x: phys_cx,
                                    anchor_y: phys_cy,
                                    items,
                                });
                        } else {
                            app.context_menu = None;
                        }
                    } else {
                        app.context_menu = None;
                    }
                }
                crate::app::PanelMode::Launcher => {}
                crate::app::PanelMode::Control(crate::controls::TileId::Clock) => {
                    let panel_rect =
                        lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
                    let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
                    if app.controls.clock.selected_day.is_some() {
                        if let Some(crate::controls::clock::DetailHit::EventRow(idx)) =
                            crate::controls::clock::hit_test_detail(
                                panel_rect,
                                view_top_y,
                                scale_f,
                                &app.controls.clock,
                                &app.controls.events,
                                &mut text,
                                phys_cx,
                                phys_cy,
                            )
                        {
                            if let Some(date) = app.controls.clock.selected_day {
                                app.controls.clock.event_menu =
                                    Some(crate::controls::clock::EventContextMenu {
                                        date,
                                        idx_in_date: idx,
                                        anchor_x: phys_cx,
                                        anchor_y: phys_cy,
                                    });
                            }
                        } else {
                            app.controls.clock.event_menu = None;
                        }
                    }
                }
                _ => {}
            }
        }

        // Drag continuation: while the user has a slider grabbed, every
        // pointer-motion event re-computes the slider value from the
        // current cursor x. Released → end the drag.
        if !wl.left_held {
            app.dragging = None;
            app.settings_drag = None;
        }

        // Settings page slider drag: while the user has a settings
        // slider grabbed, motion events update the bound config field
        // and persist on release.
        if let Some(key) = app.settings_drag {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let rows = crate::settings::layout(panel_rect, top_y, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            if let Some(value) = crate::settings::hit_slider_only(&rows, key, phys_cx) {
                crate::settings::apply_value(
                    &mut app.config,
                    key,
                    crate::settings::SettingValue::F(value),
                );
                app.config.save();
            }
        }
        if let Some(target) = app.dragging {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;

            use crate::app::DragTarget;
            use crate::controls::audio::Direction;
            let track = match target {
                DragTarget::AudioOutputSlider => crate::controls::audio::slider_rect_for(
                    panel_rect, view_top_y, Direction::Output, scale_f,
                ),
                DragTarget::AudioInputSlider => crate::controls::audio::slider_rect_for(
                    panel_rect, view_top_y, Direction::Input, scale_f,
                ),
                DragTarget::BrightnessSlider => crate::controls::brightness::slider_rect(
                    panel_rect, view_top_y, scale_f,
                ),
            };
            let frac = ((phys_cx - track.x) / track.w).clamp(0.0, 1.0);
            match target {
                DragTarget::AudioOutputSlider => app.controls.audio.set_volume(frac),
                DragTarget::AudioInputSlider => app.controls.audio.set_input_volume(frac),
                DragTarget::BrightnessSlider => app.controls.brightness.set_fraction(frac),
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
        mono_text.clear();

        let panel_draw = crate::render::draw_panel(&mut painter, &app, phys_w, scale_f);
        let icon_requests = if let Some(p) = &panel_draw {
            crate::render::draw_content(
                &mut painter, &mut text, &mut mono_text, &app, p, phys_w, phys_h,
            )
        } else {
            Vec::new()
        };

        // Stream thumbnail slots to the compositor so it can paint live
        // window content into each Open-section tile. Sent only when the
        // Open section is actually being drawn — i.e. fully visible,
        // Launcher mode, and the empty/non-all-apps search state that
        // render.rs uses to draw the section. Otherwise the compositor
        // keeps painting thumbnails at orphaned rects.
        // Active whenever Default is being displayed in some form —
        // either as the resting view OR as the from/to of an in-flight
        // slide. The slide-x for the rect is computed below.
        let default_in_view = app.panel_view == crate::app::PanelView::Default
            || match app.view_slide() {
                Some(s) => s.from == crate::app::PanelView::Default
                    || s.to == crate::app::PanelView::Default,
                None => false,
            };
        let open_section_active = matches!(app.mode, crate::app::PanelMode::Launcher)
            && app.search.input.is_empty()
            && !app.search.all_apps_mode
            && !app.collapsed
            && !app.collapse_animating()
            // Skip thumbs during a view-slide so the compositor doesn't
            // keep painting window previews that have travelled past the
            // panel edge into empty space.
            && !app.view_animating()
            && default_in_view
            && !app.settings_open
            && !app.emojis.open;
        if let Some(p) = &panel_draw {
            if matches!(app.visibility, crate::app::Visibility::Visible) && open_section_active {
                let panel_logical = lntrn_render::Rect::new(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
                let pin_top_y = panel_logical.y
                    + crate::controls::total_logical_height() * scale_f
                    + (crate::search::input::SEARCH_HORIZONTAL_PAD * 0.5
                        + crate::search::input::SEARCH_ROW_HEIGHT)
                        * scale_f;
                let pinned_count = app.launcher.pinned_entries(&app.apps).len();
                let pins_bottom = crate::launcher::pins_section_bottom(
                    panel_logical,
                    pin_top_y,
                    scale_f,
                    pinned_count,
                );
                let visible_open = crate::launcher::open::visible_entries(&app.toplevels);
                let row_top = pins_bottom
                    + crate::launcher::open::OPEN_SECTION_TOP_MARGIN * scale_f
                    + crate::launcher::open::heading_advance(scale_f);

                // If we're mid-slide, shift the thumbnail rects by
                // Default's current slide offset so they glide with
                // the rest of the body instead of popping.
                let default_slide_offset = match app.view_slide() {
                    Some(s) if s.from == crate::app::PanelView::Default => s.from_offset,
                    Some(s) if s.to == crate::app::PanelView::Default => s.to_offset,
                    _ => 0.0,
                };
                let slide_x = default_slide_offset * panel_logical.w;
                let mut slots = Vec::with_capacity(visible_open.len());
                let phys_cx = wl.cursor_x as f32 * scale_f;
                let phys_cy = wl.cursor_y as f32 * scale_f;
                for (i, group) in visible_open.iter().enumerate() {
                    let Some(rep) = group.close_target() else { continue };
                    let r = crate::launcher::open::tile_rect(panel_logical, row_top, scale_f, i);
                    let inv = 1.0 / scale_f;
                    let close_btn_r = crate::launcher::open::close_button_rect(
                        panel_logical, row_top, scale_f, i,
                    );
                    let close_hovered = phys_cx >= close_btn_r.x
                        && phys_cx <= close_btn_r.x + close_btn_r.w
                        && phys_cy >= close_btn_r.y
                        && phys_cy <= close_btn_r.y + close_btn_r.h;
                    let close = crate::thumbs::CloseBtn {
                        x: ((close_btn_r.x + slide_x) * inv).round() as i32,
                        y: (close_btn_r.y * inv).round() as i32,
                        w: (close_btn_r.w * inv).round() as i32,
                        h: (close_btn_r.h * inv).round() as i32,
                        hovered: close_hovered,
                    };
                    slots.push(crate::thumbs::ThumbSlot {
                        app_id: rep.app_id.clone(),
                        title: rep.title.clone(),
                        x: ((r.x + slide_x) * inv).round() as i32,
                        y: (r.y * inv).round() as i32,
                        w: (r.w * inv).round() as i32,
                        h: (r.h * inv).round() as i32,
                        close: Some(close),
                    });
                }
                thumbs.update(&slots);
            } else if matches!(app.visibility, crate::app::Visibility::Visible)
                && app.collapse_progress() > 0.5
                && app.mini_dock_hover.is_some()
                && app.panel_view == crate::app::PanelView::Default
                && !app.settings_open
            {
                // Dock hover preview: one thumbnail slot per window of
                // the hovered pinned app, arrayed horizontally.
                let mut slots: Vec<crate::thumbs::ThumbSlot> = Vec::new();
                let panel_logical = lntrn_render::Rect::new(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
                let pinned = app.launcher.pinned_entries(&app.apps);
                let idx = app.mini_dock_hover.unwrap();
                if let Some(entry) = pinned.get(idx) {
                    let windows = crate::mini_dock::windows_for_app(
                        &app.toplevels, &entry.app_id,
                    );
                    if !windows.is_empty() {
                        let tiles = crate::mini_dock::preview_tile_rects(
                            panel_logical, scale_f, pinned.len(), idx, windows.len(),
                        );
                        let inv = 1.0 / scale_f;
                        let phys_cx = wl.cursor_x as f32 * scale_f;
                        let phys_cy = wl.cursor_y as f32 * scale_f;
                        for (tile, window) in tiles.iter().zip(windows.iter()) {
                            let close = crate::mini_dock::preview_close_button_rect(*tile, scale_f);
                            let close_hovered = phys_cx >= close.x
                                && phys_cx <= close.x + close.w
                                && phys_cy >= close.y
                                && phys_cy <= close.y + close.h;
                            slots.push(crate::thumbs::ThumbSlot {
                                app_id: window.app_id.clone(),
                                title: window.title.clone(),
                                x: (tile.x * inv).round() as i32,
                                y: (tile.y * inv).round() as i32,
                                w: (tile.w * inv).round() as i32,
                                h: (tile.h * inv).round() as i32,
                                close: Some(crate::thumbs::CloseBtn {
                                    x: (close.x * inv).round() as i32,
                                    y: (close.y * inv).round() as i32,
                                    w: (close.w * inv).round() as i32,
                                    h: (close.h * inv).round() as i32,
                                    hovered: close_hovered,
                                }),
                            });
                        }
                    }
                }
                thumbs.update(&slots);
            } else {
                thumbs.clear();
            }
        } else {
            thumbs.clear();
        }

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
                    d.clip = req.clip;
                    d
                })
            })
            .collect();

        match gpu.begin_frame("CommandCenter") {
            Ok(mut frame) => {
                let view = frame.view().clone();

                // Layered render so modals (BT pair, BT incoming, WiFi
                // password) draw over previously-queued text. Layer 0 is
                // base content; layer 1 is overlays. See
                // lntrn-render/TEXT_OCCLUSION_FIX.md.
                let layers = painter
                    .layer_count()
                    .max(text.layer_count())
                    .max(mono_text.layer_count());

                // Layer 0: base painter, textures, base text, mono text.
                painter.render_layer(
                    0,
                    &gpu,
                    frame.encoder_mut(),
                    &view,
                    Some(Color::TRANSPARENT),
                );
                if !tex_draws.is_empty() {
                    tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
                }
                text.render_layer(0, &gpu, frame.encoder_mut(), &view);
                mono_text.render_layer(0, &gpu, frame.encoder_mut(), &view);

                // Overlay layers (modals).
                if layers > 1 {
                    // Flush so the next layer's text prepare() doesn't
                    // stomp on layer-0 vertices still in the queue.
                    frame.flush(&gpu);
                    for li in 1..layers {
                        painter.render_layer(li, &gpu, frame.encoder_mut(), &view, None);
                        text.render_layer(li, &gpu, frame.encoder_mut(), &view);
                        mono_text.render_layer(li, &gpu, frame.encoder_mut(), &view);
                    }
                }

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
/// Dispatch a left-click that landed inside the panel while a control's
/// full-content view is showing. Returns `true` if the click was
/// consumed (so the caller should skip launcher hit-tests).
///
/// Currently the battery toggle, audio slider, and audio device list
/// are interactive; future controls plug in here.
/// Phys-px rect for the always-on Files quick-locations strip drawn
/// inside the controls row. Mirrors the layout used by
/// `files::render::draw_collapsed_strip`. Returns `None` when the
/// collapse chevron isn't laid out (shouldn't happen in practice).
fn files_strip_rect(
    app: &crate::app::AppState,
    panel_rect: lntrn_render::Rect,
    scale: f32,
) -> Option<lntrn_render::Rect> {
    let chev = app.controls.tile_layout(
        crate::controls::TileId::Collapse,
        panel_rect,
        scale,
        crate::app::PanelView::Files,
    )?;
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let gap = 12.0 * scale;
    let x = panel_rect.x + pad;
    let right_edge = chev.x - gap;
    let w = (right_edge - x).max(0.0);
    if w <= 0.0 {
        return None;
    }
    let h = chev.h - 4.0 * scale;
    let y = chev.y + 2.0 * scale;
    Some(lntrn_render::Rect::new(x, y, w, h))
}

/// Build the items list for the Files sort context menu. The current
/// sort column gets a small arrow indicator suggesting direction.
fn sort_menu_items(
    state: &crate::files::FilesState,
) -> Vec<crate::launcher::context_menu::MenuItem> {
    use crate::files::{SortBy, SortDir};
    use crate::launcher::context_menu::{MenuAction, MenuItem};
    let arrow = if state.sort_dir == SortDir::Asc { "↑" } else { "↓" };
    let mark = |by: SortBy, label: &str| -> String {
        if state.sort_by == by { format!("{}  {}", label, arrow) } else { label.to_string() }
    };
    vec![
        MenuItem { label: mark(SortBy::Name, "Name"), action: MenuAction::FilesSortByName },
        MenuItem { label: mark(SortBy::Size, "Size"), action: MenuAction::FilesSortBySize },
        MenuItem { label: mark(SortBy::Modified, "Date Modified"), action: MenuAction::FilesSortByDate },
        MenuItem { label: mark(SortBy::Type, "Type"), action: MenuAction::FilesSortByType },
    ]
}

fn handle_control_view_click(
    app: &mut AppState,
    text: &mut TextRenderer,
    panel: lntrn_render::Rect,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> bool {
    let crate::app::PanelMode::Control(tile_id) = app.mode else { return false };
    // The control view starts immediately beneath the controls-row underline.
    let view_top_y = crate::controls::content_top_y(panel, scale);

    match tile_id {
        crate::controls::TileId::Clock => {
            // Detail panel takes priority when open.
            if app.controls.clock.selected_day.is_some() {
                if let Some(hit) = crate::controls::clock::hit_test_detail(
                    panel,
                    view_top_y,
                    scale,
                    &app.controls.clock,
                    &app.controls.events,
                    text,
                    phys_x,
                    phys_y,
                ) {
                    match hit {
                        crate::controls::clock::DetailHit::Close => {
                            app.controls.clock.selected_day = None;
                            app.controls.clock.add_event_input = None;
                            app.controls.clock.event_menu = None;
                        }
                        crate::controls::clock::DetailHit::OpenAddInput => {
                            app.controls.clock.add_event_input =
                                Some(crate::search::input::Input::new());
                        }
                        crate::controls::clock::DetailHit::EventRow(_) => {
                            // Left-click on an event row currently does
                            // nothing — delete is via right-click menu.
                        }
                    }
                    return true;
                }
            }

            if let Some(hit) = crate::controls::clock::hit_test_view(
                panel,
                view_top_y,
                scale,
                &app.controls.clock,
                text,
                phys_x,
                phys_y,
            ) {
                match hit {
                    crate::controls::clock::CalendarHit::PrevMonth => {
                        app.controls.clock.prev_month();
                    }
                    crate::controls::clock::CalendarHit::NextMonth => {
                        app.controls.clock.next_month();
                    }
                    crate::controls::clock::CalendarHit::Day(date) => {
                        // Toggle: clicking the same day again closes.
                        if app.controls.clock.selected_day == Some(date) {
                            app.controls.clock.selected_day = None;
                            app.controls.clock.add_event_input = None;
                        } else {
                            app.controls.clock.selected_day = Some(date);
                            app.controls.clock.add_event_input = None;
                        }
                    }
                }
                return true;
            }
            false
        }
        crate::controls::TileId::Battery => {
            let toggle = crate::controls::battery::toggle_rect(panel, view_top_y, scale);
            if phys_x >= toggle.x
                && phys_x <= toggle.x + toggle.w
                && phys_y >= toggle.y
                && phys_y <= toggle.y + toggle.h
            {
                app.controls.battery.toggle_charge_limit();
                return true;
            }
            false
        }
        crate::controls::TileId::Audio => {
            use crate::controls::audio::Direction;

            // Sliders — try each direction's track. A slider click both
            // sets the volume immediately and starts a drag so motion
            // events keep updating until the button is released.
            for dir in [Direction::Output, Direction::Input] {
                let track =
                    crate::controls::audio::slider_rect_for(panel, view_top_y, dir, scale);
                let row_top = track.y - track.h * 2.0;
                let row_bot = track.y + track.h * 3.0;
                if phys_x >= track.x
                    && phys_x <= track.x + track.w
                    && phys_y >= row_top
                    && phys_y <= row_bot
                {
                    let frac = ((phys_x - track.x) / track.w).clamp(0.0, 1.0);
                    match dir {
                        Direction::Output => {
                            app.controls.audio.set_volume(frac);
                            app.dragging = Some(crate::app::DragTarget::AudioOutputSlider);
                        }
                        Direction::Input => {
                            app.controls.audio.set_input_volume(frac);
                            app.dragging = Some(crate::app::DragTarget::AudioInputSlider);
                        }
                    }
                    return true;
                }
            }

            // Speaker / mic icon click → toggle that direction's mute.
            if let Some(dir) = crate::controls::audio::hit_test_icon(
                panel, view_top_y, scale, phys_x, phys_y,
            ) {
                match dir {
                    Direction::Output => app.controls.audio.toggle_mute(),
                    Direction::Input => app.controls.audio.toggle_input_mute(),
                }
                return true;
            }

            // Device lists — click a row to set that device as default.
            if let Some((dir, dev_id)) = crate::controls::audio::hit_test_device_dir(
                &app.controls.audio,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match dir {
                    Direction::Output => app.controls.audio.set_default_sink(dev_id),
                    Direction::Input => app.controls.audio.set_default_source(dev_id),
                }
                return true;
            }
            false
        }
        crate::controls::TileId::Brightness => {
            let track =
                crate::controls::brightness::slider_rect(panel, view_top_y, scale);
            let row_top = track.y - track.h * 2.0;
            let row_bot = track.y + track.h * 3.0;
            if phys_x >= track.x
                && phys_x <= track.x + track.w
                && phys_y >= row_top
                && phys_y <= row_bot
            {
                let frac = ((phys_x - track.x) / track.w).clamp(0.0, 1.0);
                app.controls.brightness.set_fraction(frac);
                app.dragging = Some(crate::app::DragTarget::BrightnessSlider);
                return true;
            }
            false
        }
        crate::controls::TileId::Bluetooth => {
            use crate::controls::bluetooth::{
                BtClick, IncomingModalHit, PairModalHit, PairPromptKind,
            };

            // Incoming-file modal sits highest in priority.
            if app.controls.bluetooth.incoming_request.is_some() {
                let hit = crate::controls::bluetooth::hit_test_incoming_modal(
                    panel, view_top_y, scale, phys_x, phys_y,
                );
                match hit {
                    IncomingModalHit::Accept => app.controls.bluetooth.incoming_accept(),
                    IncomingModalHit::Reject | IncomingModalHit::Backdrop => {
                        app.controls.bluetooth.incoming_reject();
                    }
                    IncomingModalHit::Box => {}
                }
                return true;
            }

            // If the pair-prompt modal is up, every click in the BT
            // view goes to the modal first.
            if let Some(prompt) = app.controls.bluetooth.pair_prompt.as_ref() {
                let kind = prompt.kind.clone();
                let hit = crate::controls::bluetooth::hit_test_pair_modal(
                    prompt, panel, view_top_y, scale, phys_x, phys_y,
                );
                match hit {
                    PairModalHit::Primary => match kind {
                        PairPromptKind::Confirm(_) | PairPromptKind::Authorize(_) => {
                            app.controls.bluetooth.pair_confirm_yes();
                        }
                        PairPromptKind::Enter => {
                            app.controls.bluetooth.pair_submit_passkey();
                        }
                    },
                    PairModalHit::Secondary | PairModalHit::Backdrop => {
                        match kind {
                            PairPromptKind::Confirm(_) | PairPromptKind::Authorize(_) => {
                                app.controls.bluetooth.pair_confirm_no();
                            }
                            PairPromptKind::Enter => {
                                app.controls.bluetooth.pair_cancel();
                            }
                        }
                    }
                    PairModalHit::Field | PairModalHit::Box => {
                        // Inside the modal but not on a button — no-op.
                    }
                }
                return true;
            }

            if let Some(hit) = crate::controls::bluetooth::hit_test(
                &app.controls.bluetooth,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match hit {
                    BtClick::PowerToggle => app.controls.bluetooth.toggle_power(),
                    BtClick::DiscoverableToggle => {
                        app.controls.bluetooth.toggle_discoverable();
                    }
                    BtClick::ScanToggle => app.controls.bluetooth.toggle_scan(),
                    BtClick::DeviceRow(mac) => {
                        let is_paired = app
                            .controls
                            .bluetooth
                            .devices()
                            .iter()
                            .any(|d| d.mac == mac && d.paired);
                        if is_paired {
                            app.controls.bluetooth.toggle_connection(&mac);
                        } else {
                            app.controls.bluetooth.pair(&mac);
                        }
                    }
                    BtClick::SendButton(mac) => {
                        app.controls.bluetooth.send_file(&mac);
                    }
                }
                return true;
            }
            false
        }
        crate::controls::TileId::Wifi => {
            // If the password modal is up, every click in the WiFi
            // view goes to the modal first.
            if app.controls.wifi.prompt.is_some() {
                use crate::controls::wifi::ModalHit;
                let hit = crate::controls::wifi::hit_test_modal(
                    panel, view_top_y, scale, phys_x, phys_y,
                );
                match hit {
                    ModalHit::Connect => {
                        app.controls.wifi.submit_prompt();
                    }
                    ModalHit::Cancel | ModalHit::Backdrop => {
                        app.controls.wifi.close_prompt();
                    }
                    ModalHit::Field | ModalHit::Box => {
                        // No-op — clicks inside the box just dismiss
                        // pending hover state in a future iteration.
                    }
                }
                return true;
            }

            // Normal network-row click.
            if let Some(hit) = crate::controls::wifi::hit_test_network(
                &app.controls.wifi,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match hit {
                    crate::controls::wifi::NetworkHit::Row(ssid) => {
                        // Toggle: clicking the same row again collapses it.
                        if app.controls.wifi.expanded_ssid.as_deref() == Some(ssid.as_str()) {
                            app.controls.wifi.expanded_ssid = None;
                        } else {
                            app.controls.wifi.expanded_ssid = Some(ssid);
                        }
                    }
                    crate::controls::wifi::NetworkHit::BandPill(ssid, band) => {
                        app.controls.wifi.select_band(&ssid, band);
                    }
                    crate::controls::wifi::NetworkHit::ConnectButton(ssid) => {
                        let net = app.controls.wifi.networks()
                            .iter()
                            .find(|n| n.ssid == ssid)
                            .cloned();
                        let already_in_use = net.as_ref().is_some_and(|n| n.in_use);
                        let needs_password = match &net {
                            Some(n) => {
                                let secured = !n.security.is_empty() && n.security != "--";
                                secured && !n.saved && !n.in_use
                            }
                            None => false,
                        };
                        if already_in_use {
                            // Already connected → button is purely a label.
                        } else if needs_password {
                            app.controls.wifi.open_prompt(&ssid);
                        } else {
                            app.controls.wifi.connect(&ssid, None);
                        }
                        tracing::debug!(%ssid, needs_password, "wifi: connect button");
                    }
                }
                return true;
            }
            false
        }
        // Temp shares its expanded view (and click behavior) with
        // SysMon since they read from the same backend.
        crate::controls::TileId::SysMon | crate::controls::TileId::Temp => {
            if let Some(hit) = crate::controls::sysmon::view::hit_test_view(
                &app.controls.sysmon,
                panel,
                view_top_y,
                scale,
                phys_x,
                phys_y,
            ) {
                match hit {
                    crate::controls::sysmon::view::SysMonHit::SelectProcess(pid) => {
                        app.controls.sysmon.selected_pid = Some(pid);
                    }
                    crate::controls::sysmon::view::SysMonHit::KillProcess(pid) => {
                        crate::controls::sysmon::view::kill_process(pid);
                        // Clear selection so the user has to re-arm
                        // before another kill can fire.
                        app.controls.sysmon.selected_pid = None;
                    }
                    crate::controls::sysmon::view::SysMonHit::SortByCpu => {
                        let next = app.controls.sysmon.sort.toggle_cpu();
                        app.controls.sysmon.set_sort(next);
                    }
                    crate::controls::sysmon::view::SysMonHit::SortByMem => {
                        let next = app.controls.sysmon.sort.toggle_mem();
                        app.controls.sysmon.set_sort(next);
                    }
                    crate::controls::sysmon::view::SysMonHit::ClearFilter => {
                        app.controls.sysmon.filter.clear();
                    }
                }
                return true;
            }
            false
        }
        // No expanded view — click handling is shortcut in the press
        // path, so we never reach here for these.
        crate::controls::TileId::Collapse | crate::controls::TileId::TerminalClear => false,
    }
}

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

/// Hide the surface from the compositor without destroying it. We first
/// submit a fully-transparent wgpu frame (so any in-flight buffer is
/// cleanly transparent), then explicitly attach a NULL buffer to the
/// `wl_surface`. Per Wayland spec, attaching a NULL buffer + committing
/// unmaps the surface — the compositor stops compositing it and any
/// "ghost" of the previous visible buffer disappears immediately.
/// Re-mapping is automatic on the next `attach + commit` with a real
/// buffer (which wgpu's `present()` does for us when the user reopens).
fn commit_transparent(gpu: &mut GpuContext, surface: &wl_surface::WlSurface) {
    if let Ok(mut frame) = gpu.begin_frame("CommandCenter:hidden") {
        let view = frame.view().clone();
        let mut painter = Painter::new(gpu);
        painter.clear();
        painter.render_pass(gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
        frame.submit(&gpu.queue);
    }
    // Detach buffer → tells the compositor to unmap the surface entirely.
    surface.attach(None, 0, 0);
    surface.commit();
}

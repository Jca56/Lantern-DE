//! Layer-shell client for the screenshot UI.
//!
//! Why layer-shell instead of an xdg_toplevel: a regular toplevel would
//! (a) sit *below* the Command Center's overlay layer surface — so CC
//! would steal focus and Ctrl+C/Enter would never reach us — and
//! (b) be subject to the compositor's Super+drag move gesture, which
//! lets the user accidentally drag the screenshot window around the
//! screen revealing the desktop underneath.
//!
//! We open a fullscreen overlay-layer surface with
//! `KeyboardInteractivity::Exclusive` so we always sit on top of every
//! other surface (including CC) and always receive keyboard focus while
//! the screenshot UI is up.

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry,
        wl_seat, wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

// Evdev keycodes.
pub const KEY_ESC: u32 = 1;
pub const KEY_ENTER: u32 = 28;
pub const KEY_KPENTER: u32 = 96;
pub const KEY_C: u32 = 46;
pub const KEY_S: u32 = 31;
pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_RIGHTCTRL: u32 = 97;

// Linux button codes.
pub const BTN_LEFT: u32 = 0x110;

/// Per-frame drained input state.
#[derive(Default)]
pub struct FrameInput {
    pub cursor_moved: bool,
    pub cursor_x: f32,
    pub cursor_y: f32,
    pub left_pressed: bool,
    pub left_released: bool,
    pub esc: bool,
    pub enter: bool,
    pub ctrl_c: bool,
    pub ctrl_s: bool,
}

/// Hands wgpu a `RawDisplayHandle` / `RawWindowHandle` pair pointing at
/// the bare `wl_display` and `wl_surface` pointers.
pub struct WaylandHandle {
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

pub struct WlState {
    pub running: bool,
    pub configured: bool,
    pub frame_done: bool,
    /// Logical surface size (compositor units).
    pub width: u32,
    pub height: u32,
    /// Integer scale advertised by the bound wl_output (fallback).
    pub scale: i32,
    /// Physical mode width — combined with logical width gives us the
    /// fractional output scale.
    pub output_phys_width: u32,

    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,

    cursor_x: f64,
    cursor_y: f64,
    cursor_dirty: bool,
    left_pressed_this_frame: bool,
    left_released_this_frame: bool,
    ctrl_held: bool,
    esc_pressed: bool,
    enter_pressed: bool,
    ctrl_c_pressed: bool,
    ctrl_s_pressed: bool,
}

impl WlState {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: false,
            width: 0,
            height: 0,
            scale: 1,
            output_phys_width: 0,
            compositor: None,
            layer_shell: None,
            viewporter: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            cursor_dirty: false,
            left_pressed_this_frame: false,
            left_released_this_frame: false,
            ctrl_held: false,
            esc_pressed: false,
            enter_pressed: false,
            ctrl_c_pressed: false,
            ctrl_s_pressed: false,
        }
    }

    pub fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    pub fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    pub fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }

    /// Drain accumulated input since the last frame. Coordinates returned
    /// are in *physical* pixels (cursor position multiplied by fractional
    /// scale) so callers can compare directly against the screenshot
    /// texture which is in physical pixels.
    pub fn take_frame_input(&mut self) -> FrameInput {
        let scale = self.fractional_scale() as f32;
        let out = FrameInput {
            cursor_moved: self.cursor_dirty,
            cursor_x: self.cursor_x as f32 * scale,
            cursor_y: self.cursor_y as f32 * scale,
            left_pressed: self.left_pressed_this_frame,
            left_released: self.left_released_this_frame,
            esc: self.esc_pressed,
            enter: self.enter_pressed,
            ctrl_c: self.ctrl_c_pressed,
            ctrl_s: self.ctrl_s_pressed,
        };
        self.cursor_dirty = false;
        self.left_pressed_this_frame = false;
        self.left_released_this_frame = false;
        self.esc_pressed = false;
        self.enter_pressed = false;
        self.ctrl_c_pressed = false;
        self.ctrl_s_pressed = false;
        out
    }
}

/// The handle returned to `main` so it can drive the event loop and
/// hand wgpu the underlying surface pointer.
pub struct LayerWindow {
    pub conn: Connection,
    pub queue: EventQueue<WlState>,
    pub qh: QueueHandle<WlState>,
    pub state: WlState,
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub viewport: Option<wp_viewport::WpViewport>,
    pub handle: WaylandHandle,
}

impl LayerWindow {
    pub fn new() -> Result<Self> {
        let conn = Connection::connect_to_env()?;
        let display = conn.display();
        let mut queue: EventQueue<WlState> = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = WlState::new();

        display.get_registry(&qh, ());
        queue.roundtrip(&mut state)?;

        let compositor = state
            .compositor
            .as_ref()
            .ok_or_else(|| anyhow!("wl_compositor not available"))?
            .clone();
        let layer_shell = state
            .layer_shell
            .as_ref()
            .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?
            .clone();

        let surface = compositor.create_surface(&qh, ());

        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Overlay,
            "lntrn-screenshot".to_string(),
            &qh,
            (),
        );
        {
            use zwlr_layer_surface_v1::Anchor;
            layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer_surface.set_size(0, 0);
            layer_surface.set_exclusive_zone(-1);
            // Steal keyboard focus from CC / any other layer surface so
            // Esc/Enter/Ctrl+C always reach the screenshot UI.
            layer_surface.set_keyboard_interactivity(
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
            );
        }
        surface.commit();

        while !state.configured {
            queue.blocking_dispatch(&mut state)?;
        }
        if state.width == 0 {
            return Err(anyhow!("compositor sent zero-width configure"));
        }
        queue.roundtrip(&mut state)?;

        surface.set_buffer_scale(1);
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

        // Mark frame_done so the first iteration draws.
        state.frame_done = true;

        Ok(Self {
            conn,
            queue,
            qh,
            state,
            surface,
            layer_surface,
            viewport,
            handle,
        })
    }

    /// Block for the next batch of events. Returns when at least one
    /// event has been dispatched.
    pub fn dispatch(&mut self) -> Result<()> {
        self.queue.blocking_dispatch(&mut self.state)?;
        Ok(())
    }

    /// Request a frame callback so we know when the compositor is ready
    /// for the next buffer. Sets `frame_done = false` until the callback
    /// fires.
    pub fn request_frame(&mut self) {
        self.state.frame_done = false;
        let _ = self.surface.frame(&self.qh, ());
    }

    /// Tear down the layer surface so input grabs are released *before*
    /// we start serving the clipboard. Otherwise the compositor would
    /// keep keyboard focus pinned on our (now invisible) surface.
    pub fn destroy(self) {
        self.layer_surface.destroy();
        self.surface.destroy();
        let _ = self.conn.flush();
    }
}

// ── Dispatch impls ──────────────────────────────────────────────────────────

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
            if let WEnum::Value(caps) = caps {
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
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.cursor_dirty = true;
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.cursor_dirty = true;
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                if button == BTN_LEFT {
                    let pressed = btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                    let released = btn_state == WEnum::Value(wl_pointer::ButtonState::Released);
                    if pressed {
                        state.left_pressed_this_frame = true;
                    } else if released {
                        state.left_released_this_frame = true;
                    }
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
        match event {
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                let pressed = key_state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                let released = key_state == WEnum::Value(wl_keyboard::KeyState::Released);
                if key == KEY_LEFTCTRL || key == KEY_RIGHTCTRL {
                    if pressed { state.ctrl_held = true; }
                    else if released { state.ctrl_held = false; }
                } else if pressed {
                    match key {
                        KEY_ESC => state.esc_pressed = true,
                        KEY_ENTER | KEY_KPENTER => state.enter_pressed = true,
                        KEY_C if state.ctrl_held => state.ctrl_c_pressed = true,
                        KEY_S if state.ctrl_held => state.ctrl_s_pressed = true,
                        _ => {}
                    }
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.ctrl_held = (mods_depressed & 4) != 0;
            }
            wl_keyboard::Event::Leave { .. } => {
                state.ctrl_held = false;
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

//! Wayland protocol state, dispatch impls, and the raw window handle wrapper
//! for wgpu. Split out from `wayland.rs` to keep the per-frame run loop file
//! focused on rendering and input handling.

use std::ffi::c_void;
use std::ptr::NonNull;

use lntrn_ui::gpu::WaylandPopupBackend;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

// ── Mouse button codes (Linux input event codes) ───────────────────────────
pub(crate) const BTN_LEFT: u32 = 0x110;
pub(crate) const BTN_RIGHT: u32 = 0x111;

// ── Raw window handle wrapper for wgpu ─────────────────────────────────────

pub(crate) struct WaylandHandle {
    pub display: NonNull<c_void>,
    pub surface: NonNull<c_void>,
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

// ── Output info ────────────────────────────────────────────────────────────

/// Info about a connected monitor from wl_output events.
#[derive(Clone, Debug)]
pub(crate) struct OutputInfo {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub scale: i32,
}

// ── Wayland state ──────────────────────────────────────────────────────────

pub(crate) struct State {
    pub running: bool,
    pub configured: bool,
    pub frame_done: bool,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub output_phys_width: u32,
    pub maximized: bool,
    /// Tracked outputs from wl_output events (key = global name from registry).
    pub outputs: Vec<(u32, OutputInfo)>,
    /// Staging area for incomplete wl_output event batches (before Done).
    pub output_pending: std::collections::HashMap<u32, OutputInfo>,
    // Wayland objects
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub surface: Option<wl_surface::WlSurface>,
    pub xdg_surface: Option<xdg_surface::XdgSurface>,
    pub toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub seat: Option<wl_seat::WlSeat>,
    // Input
    pub cursor_x: f64,
    pub cursor_y: f64,
    pub pointer_in_surface: bool,
    pub left_pressed: bool,
    pub left_released: bool,
    pub right_pressed: bool,
    pub scroll_delta: f32,
    pub pointer_serial: u32,
    pub enter_serial: u32,
    // Cursor shape
    pub cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    pub pointer: Option<wl_pointer::WlPointer>,
    // Keyboard
    pub key_pressed: Option<u32>,
    pub keymap_pending: Option<(std::os::fd::RawFd, u32)>,
    pub modifiers_pending: Option<(u32, u32, u32, u32)>,
    pub shift: bool,
    // Popup
    pub popup_backend: Option<WaylandPopupBackend<State>>,
    pub popup_closed: bool,
    pub pointer_surface: Option<wl_surface::WlSurface>,
    // Output management
    pub output_mgr: crate::output_manager::OutputManagerClient,
}

impl State {
    pub fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0, maximized: false,
            outputs: Vec::new(),
            output_pending: std::collections::HashMap::new(),
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false, right_pressed: false,
            scroll_delta: 0.0, pointer_serial: 0, enter_serial: 0,
            cursor_shape_mgr: None, cursor_shape_device: None,
            current_cursor_shape: None, pointer: None,
            key_pressed: None,
            keymap_pending: None,
            modifiers_pending: None,
            shift: false,
            popup_backend: None,
            popup_closed: false,
            pointer_surface: None,
            output_mgr: crate::output_manager::OutputManagerClient::new(),
        }
    }

    pub fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    pub fn phys_width(&self) -> u32 { (self.width as f64 * self.fractional_scale()).round() as u32 }
    pub fn phys_height(&self) -> u32 { (self.height as f64 * self.fractional_scale()).round() as u32 }
}

// ── Dispatch impls ─────────────────────────────────────────────────────────

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
                "wl_output" => { let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, name); }
                "wl_seat" => { state.seat = Some(registry.bind(name, version.min(9), qh, ())); }
                "wp_cursor_shape_manager_v1" => {
                    state.cursor_shape_mgr = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "zwlr_output_manager_v1" => {
                    state.output_mgr.manager = Some(registry.bind(name, version.min(4), qh, ()));
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

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self, _: &wl_output::WlOutput,
        event: wl_output::Event, global_name: &u32, _: &Connection, _: &QueueHandle<Self>,
    ) {
        let gn = *global_name;
        let pending = state.output_pending.entry(gn).or_insert_with(|| OutputInfo {
            name: String::new(), width: 0, height: 0, x: 0, y: 0, scale: 1,
        });
        match event {
            wl_output::Event::Name { name } => { pending.name = name; }
            wl_output::Event::Mode { width, height, .. } => {
                pending.width = width;
                pending.height = height;
                // Keep backwards compat for fractional scale calculation
                state.output_phys_width = width as u32;
            }
            wl_output::Event::Scale { factor } => {
                pending.scale = factor;
                state.scale = factor;
            }
            wl_output::Event::Geometry { x, y, .. } => {
                pending.x = x;
                pending.y = y;
            }
            wl_output::Event::Done => {
                if let Some(info) = state.output_pending.remove(&gn) {
                    if let Some(existing) = state.outputs.iter_mut().find(|(n, _)| *n == gn) {
                        existing.1 = info;
                    } else {
                        state.outputs.push((gn, info));
                    }
                }
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
                if let Some(mgr) = &state.cursor_shape_mgr {
                    state.cursor_shape_device = Some(mgr.get_pointer(&ptr, qh, ()));
                }
                state.pointer = Some(ptr);
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
            wl_pointer::Event::Enter { serial, surface, surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.enter_serial = serial;
                state.current_cursor_shape = None;
                state.pointer_surface = Some(surface);
                state.frame_done = true;
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_in_surface = false;
                state.pointer_surface = None;
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
                if button == BTN_RIGHT && pressed { state.right_pressed = true; }
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
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    use std::os::fd::AsRawFd;
                    state.keymap_pending = Some((fd.as_raw_fd(), size));
                    // Leak the fd so it isn't closed when OwnedFd drops — we read it later
                    std::mem::forget(fd);
                }
            }
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.key_pressed = Some(key);
                }
                state.frame_done = true;
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                state.modifiers_pending = Some((mods_depressed, mods_latched, mods_locked, group));
                state.frame_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, ()> for State {
    fn event(_: &mut Self, _: &wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, _: wp_cursor_shape_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, ()> for State {
    fn event(_: &mut Self, _: &wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, _: wp_cursor_shape_device_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

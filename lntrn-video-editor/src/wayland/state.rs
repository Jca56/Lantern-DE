//! State struct, pending-pick types, wgpu window-handle adapter, and
//! the edge-resize helpers used by the run loop.

use std::ffi::c_void;
use std::ptr::NonNull;

use lntrn_ui::gpu::WaylandPopupBackend;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::protocol::{wl_compositor, wl_pointer, wl_seat, wl_surface};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_decoration_manager_v1;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub(super) const KEY_ESC: u32 = 1;
pub(super) const KEY_E: u32 = 18;
pub(super) const KEY_T: u32 = 20;
pub(super) const KEY_O: u32 = 24;
pub(super) const KEY_LEFTBRACE: u32 = 26;
pub(super) const KEY_RIGHTBRACE: u32 = 27;
pub(super) const KEY_ENTER: u32 = 28;
pub(super) const KEY_S: u32 = 31;
pub(super) const KEY_BACKSLASH: u32 = 43;
pub(super) const KEY_M: u32 = 50;
pub(super) const KEY_SPACE: u32 = 57;
pub(super) const KEY_DELETE: u32 = 111;

#[derive(Clone, Copy)]
pub(super) enum PickKind {
    /// Picked file is a video to import into the media library.
    ImportMedia,
    /// Picked file is a `.lproj` to load as the current project.
    OpenProject,
    /// Picked file is the destination for a project save.
    SaveProject,
}

pub(super) struct PendingPick {
    pub(super) kind: PickKind,
    pub(super) rx: crossbeam_channel::Receiver<std::path::PathBuf>,
}

// ── WaylandHandle for wgpu ─────────────────────────────────────────────────

pub(super) struct WaylandHandle {
    pub(super) display: NonNull<c_void>,
    pub(super) surface: NonNull<c_void>,
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

// ── Wayland state ──────────────────────────────────────────────────────────

pub(crate) struct State {
    pub(crate) running: bool,
    pub(crate) configured: bool,
    pub(crate) frame_done: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: i32,
    pub(crate) output_phys_width: u32,
    pub(crate) maximized: bool,
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) surface: Option<wl_surface::WlSurface>,
    pub(crate) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(crate) toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) right_pressed: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) enter_serial: u32,
    pub(crate) cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub(crate) cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub(crate) current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    pub(crate) pointer: Option<wl_pointer::WlPointer>,
    pub(crate) key_pressed: Option<u32>,
    pub(crate) ctrl_held: bool,
    pub(crate) shift_held: bool,
    pub(crate) decoration_mgr: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    pub(crate) pointer_surface: Option<wl_surface::WlSurface>,
}

impl State {
    pub(super) fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: true,
            width: 0,
            height: 0,
            scale: 1,
            output_phys_width: 0,
            maximized: false,
            compositor: None,
            wm_base: None,
            viewporter: None,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            seat: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            left_pressed: false,
            left_released: false,
            right_pressed: false,
            scroll_delta: 0.0,
            pointer_serial: 0,
            enter_serial: 0,
            cursor_shape_mgr: None,
            cursor_shape_device: None,
            current_cursor_shape: None,
            pointer: None,
            key_pressed: None,
            ctrl_held: false,
            shift_held: false,
            decoration_mgr: None,
            popup_backend: None,
            popup_closed: false,
            pointer_surface: None,
        }
    }

    pub(crate) fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    pub(super) fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    pub(super) fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Edge resize helper ─────────────────────────────────────────────────────

pub(super) fn edge_resize(
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    border: f32,
    controls_x: f32,
) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    if top && cx > controls_x {
        return None;
    }
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

pub(super) fn resize_edge_to_cursor(edge: xdg_toplevel::ResizeEdge) -> wp_cursor_shape_device_v1::Shape {
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

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, GpuTexture, Painter, Rect, TextureDraw, TexturePass, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, PopupSurface,
    WaylandPopupBackend,
};

use crate::config::LanternConfig;
use crate::display_panel::{self, DisplayPanelState};
use crate::icon_panel;
use crate::icons;
use crate::input_panel;
use crate::monitor_arrange;
use crate::panels::{self, PanelState};
use crate::text_edit::{KeyboardState, keycode_to_char};
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

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
use crate::chrome::{TITLE_BAR_H, CORNER_RADIUS};

const SIDEBAR_W: f32 = 220.0;
const SIDEBAR_ITEM_H: f32 = 48.0;
const ICON_SIZE: u32 = 48; // rasterized icon size in pixels
const SIDEBAR_ICON_DRAW: f32 = 24.0; // logical draw size for icons

const ZONE_SIDEBAR_BASE: u32 = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Panel { WindowManager, Input, Display, Power, AppIcons }

const PANELS: &[(Panel, &str)] = &[
    (Panel::WindowManager, "Window Manager"),
    (Panel::Input, "Mouse"),
    (Panel::Display, "Display"),
    (Panel::Power, "Power"),
    (Panel::AppIcons, "App Icons"),
];

fn parse_panel_arg() -> Option<Panel> {
    let args: Vec<String> = std::env::args().collect();
    let idx = args.iter().position(|a| a == "--panel")?;
    match args.get(idx + 1)?.as_str() {
        "window-manager" => Some(Panel::WindowManager),
        "input" => Some(Panel::Input),
        "display" => Some(Panel::Display),
        "power" => Some(Panel::Power),
        "app-icons" => Some(Panel::AppIcons),
        _ => None,
    }
}

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

pub(crate) struct State {
    running: bool,
    configured: bool,
    pub(crate) frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    maximized: bool,
    /// Tracked outputs from wl_output events (key = global name from registry).
    pub(crate) outputs: Vec<(u32, OutputInfo)>,
    /// Staging area for incomplete wl_output event batches (before Done).
    output_pending: std::collections::HashMap<u32, OutputInfo>,
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
    right_pressed: bool,
    scroll_delta: f32,
    pointer_serial: u32,
    enter_serial: u32,
    // Cursor shape
    cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    pointer: Option<wl_pointer::WlPointer>,
    // Keyboard
    key_pressed: Option<u32>,
    keymap_pending: Option<(std::os::fd::RawFd, u32)>,
    modifiers_pending: Option<(u32, u32, u32, u32)>,
    shift: bool,
    // Popup
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    pointer_surface: Option<wl_surface::WlSurface>,
    // Output management
    pub(crate) output_mgr: crate::output_manager::OutputManagerClient,
}

impl State {
    fn new() -> Self {
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

// ── Edge resize helper ──────────────────────────────────────────────────────

fn edge_resize(cx: f32, cy: f32, w: f32, h: f32, border: f32, controls_x: f32) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    // Don't resize in the window controls area (top-right)
    if top && cx > controls_x { return None; }
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

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state.wm_base.clone()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    if state.width == 0 { state.width = 960; }
    if state.height == 0 { state.height = 700; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("System Settings".into());
    toplevel.set_app_id("lntrn-system-settings".into());
    toplevel.set_min_size(640, 480);
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
    let mut gpu = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let mut ix = InteractionContext::new();
    let fox = FoxPalette::night_sky();

    // Initialize popup backend
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, &wm_base, &xdg_surf, vp, &gpu, scale, &qh,
        ));
    }

    // Rasterize sidebar icons into GPU textures
    let tex_pass = TexturePass::new(&gpu);
    let icon_defs: [(Vec<icons::PathCmd>, Color); 5] = [
        (icons::icon_window_manager(), Color::from_rgb8(130, 170, 255)), // soft blue
        (icons::icon_input(),          Color::from_rgb8(180, 140, 220)), // lavender
        (icons::icon_display(),        Color::from_rgb8(100, 200, 180)), // teal
        (icons::icon_power(),          Color::from_rgb8(120, 210, 120)), // green
        (icons::icon_app_icons(),      Color::from_rgb8(230, 130, 180)), // pink
    ];
    let icon_textures: Vec<GpuTexture> = icon_defs.iter().map(|(cmds, color)| {
        let rgba = icons::rasterize_path(cmds, 24.0, 24.0, ICON_SIZE, ICON_SIZE, *color);
        tex_pass.upload(&gpu, &rgba, ICON_SIZE, ICON_SIZE)
    }).collect();

    let mut active_panel = parse_panel_arg().unwrap_or(Panel::Display);
    let mut config = LanternConfig::load();
    let mut saved_config = config.clone();
    let mut panel_state = PanelState::new(&fox);
    let mut display_state = DisplayPanelState::new(&config);
    let mut icon_panel_state = icon_panel::IconPanelState::new();
    let mut input_state = input_panel::InputPanelState::new();
    let mut kbd = KeyboardState::new();

    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[system-settings] dispatch error: {e}");
            break;
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let s = state.fractional_scale() as f32;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.width() as f32;
        let hf = gpu.height() as f32;

        // Pre-compute content area layout (needed for both click handling and rendering)
        let title_h = TITLE_BAR_H * s;
        let body_y = title_h + 4.0 * s; // strip height
        let sidebar_w = SIDEBAR_W * s;
        let content_x = sidebar_w + 1.0 * s;
        let content_w = wf - content_x;
        // header_size + padding
        let panel_y = body_y + 22.0 * s + 16.0 * s + 12.0 * s + 1.0 * s + 16.0 * s;

        // Pointer routing
        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state.popup_backend.as_ref()?.find_popup_id_by_wl_surface(ps)
        });

        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if pointer_on_popup.is_some() {
            ix.on_cursor_left();
        } else if state.pointer_in_surface {
            ix.on_cursor_moved(cx, cy);
        } else {
            ix.on_cursor_left();
        }
        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface { pointer_on_popup } else { None };
            backend.route_cursor(active, cx, cy);
        }

        // Process pending keymap/modifiers
        if let Some((fd, size)) = state.keymap_pending.take() {
            kbd.update_keymap(fd, size);
        }
        if let Some((dep, lat, lock, grp)) = state.modifiers_pending.take() {
            kbd.update_modifiers(dep, lat, lock, grp);
            state.shift = dep & 1 != 0;
        }

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            let sym = kbd.key_get_sym(key);
            let utf8 = kbd.key_to_utf8(key);

            // Fallback: if xkb didn't produce a keysym, use raw keycode mapping
            let (sym, utf8) = if sym.raw() == 0 {
                let fallback_sym = match key {
                    1 => xkbcommon::xkb::Keysym::new(0xff1b),  // Escape
                    14 => xkbcommon::xkb::Keysym::new(0xff08), // Backspace
                    28 => xkbcommon::xkb::Keysym::new(0xff0d), // Return
                    _ => sym,
                };
                let fallback_utf8 = utf8.or_else(|| keycode_to_char(key, state.shift).map(|c| c.to_string()));
                (fallback_sym, fallback_utf8)
            } else {
                (sym, utf8)
            };

            // Let focused text inputs consume the key first
            let consumed = display_panel::handle_display_key(
                &mut config, &mut display_state, sym, utf8.clone(),
            );
            let consumed = consumed || icon_panel_state.handle_key(sym, utf8);
            if !consumed && key == KEY_ESC {
                state.running = false;
            }
        }

        // Left press
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else {
                let border = 10.0 * s;
                let controls_x = wf - 120.0 * s;
                if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                    if let Some(seat) = &state.seat {
                        toplevel.resize(seat, state.pointer_serial, edge);
                    }
                } else if cy < title_h {
                    // Chrome-style window controls (distance-based hit detection)
                    let hit_r = 20.0 * s;
                    let btn_y = title_h * 0.5;
                    let close_cx = wf - 28.0 * s;
                    let max_cx = wf - 66.0 * s;
                    let min_cx = wf - 104.0 * s;
                    let dist_close = ((cx - close_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    let dist_max = ((cx - max_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    let dist_min = ((cx - min_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    if dist_close < hit_r {
                        state.running = false;
                    } else if dist_max < hit_r {
                        if state.maximized { toplevel.unset_maximized(); }
                        else { toplevel.set_maximized(); }
                    } else if dist_min < hit_r {
                        toplevel.set_minimized();
                    } else {
                        // Drag to move
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                } else if let Some(zone_id) = ix.on_left_pressed() {
                    match zone_id {
                        id if id >= ZONE_SIDEBAR_BASE && id < ZONE_SIDEBAR_BASE + PANELS.len() as u32 => {
                            active_panel = PANELS[(id - ZONE_SIDEBAR_BASE) as usize].0;
                            panel_state.close_dropdown();
                        }
                        panels::ZONE_SAVE => {
                            let wifi_changed =
                                config.power.wifi_power_save != saved_config.power.wifi_power_save
                                || config.power.wifi_power_scheme != saved_config.power.wifi_power_scheme;
                            config.save();
                            if wifi_changed {
                                config.apply_wifi_power();
                            }
                            // Apply output settings via wlr-output-management
                            if display_state.monitor_settings.dirty {
                                if let Some(selected_name) = display_state.monitor_arrange.selected_output_name() {
                                    if let Some(hi) = state.output_mgr.heads.iter().position(|h| h.name == selected_name) {
                                        let changes = vec![crate::output_manager::HeadChange {
                                            head_idx: hi,
                                            mode_idx: display_state.monitor_settings.selected_mode_idx,
                                            position: None,
                                            scale: display_state.monitor_settings.selected_scale,
                                        }];
                                        crate::output_manager::apply_config(&state, &qh, &changes);
                                        display_state.monitor_settings.dirty = false;
                                    }
                                }
                            }
                            saved_config = config.clone();
                        }
                        panels::ZONE_CANCEL => {
                            config = saved_config.clone();
                        }
                        id => {
                            // If a context menu is open, let it handle its own clicks
                            let menu_consumed = panel_state.dropdown_menu.is_open()
                                && panel_state.dropdown_menu.contains(cx, cy);
                            if !menu_consumed {
                                match active_panel {
                                    Panel::WindowManager => panels::handle_wm_click(&mut config, id),
                                    Panel::Power => {
                                        let pad_r = 32.0 * s;
                                        let btn_w = 200.0 * s;
                                        let btn_x = content_x + content_w - pad_r - btn_w;
                                        let btn_h = 42.0 * s;
                                        let row_h = 48.0 * s;
                                        panels::handle_power_click(
                                            &mut config, &mut panel_state, id,
                                            btn_x, btn_w, btn_h, row_h, panel_y, s,
                                        );
                                    }
                                    Panel::Display => {
                                        display_panel::handle_display_click(
                                            &mut config, &mut display_state, id,
                                            cx, cy, &state.output_mgr,
                                        );
                                    }
                                    Panel::Input => {
                                        input_panel::handle_input_click(&mut config, &input_state, id);
                                    }
                                    Panel::AppIcons => {
                                        icon_panel_state.on_click(id);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // Left release
        if state.left_released {
            state.left_released = false;
            // End monitor drag on release
            if monitor_arrange::is_dragging(&display_state.monitor_arrange) {
                monitor_arrange::handle_arrange_release(&mut display_state.monitor_arrange);
                if display_state.monitor_arrange.dirty {
                    config.monitors = display_state.monitor_arrange.to_config();
                }
            }
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_released();
                    }
                }
            } else {
                ix.on_left_released();
            }
        }

        // Monitor drag update on pointer motion
        if monitor_arrange::is_dragging(&display_state.monitor_arrange) {
            monitor_arrange::handle_arrange_drag(&mut display_state.monitor_arrange, cx, cy);
        }

        // Right press (no context menu yet, just consume)
        if state.right_pressed {
            state.right_pressed = false;
        }

        // Handle popup_done
        if state.popup_closed {
            state.popup_closed = false;
        }

        // Capture scroll before reset
        let frame_scroll = state.scroll_delta;
        state.scroll_delta = 0.0;

        // ── Cursor shape ────────────────────────────────────────────────
        if state.pointer_in_surface {
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
            let desired = match edge_resize(cx, cy, wf, hf, border, controls_x) {
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
        ix.begin_frame();
        painter.clear();

        let sw = gpu.width();
        let sh = gpu.height();
        let r = if state.maximized { 0.0 } else { CORNER_RADIUS * s };

        // Window chrome (Night Sky background + controls)
        crate::chrome::draw_background(&mut painter, wf, hf, r);
        crate::chrome::draw_title(&mut text, "System Settings", s, wf, title_h, sw, sh);
        crate::chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h);

        // ── Sidebar ────────────────────────────────────────────────────
        let item_h = SIDEBAR_ITEM_H * s;
        let label_size = 16.0 * s;
        let icon_draw = SIDEBAR_ICON_DRAW * s;
        let mut tex_draws: Vec<TextureDraw> = Vec::new();

        // Sidebar background (slightly lighter than window bg)
        // Bottom-left corner must match the window radius so it doesn't cover the rounded corner
        let sidebar_bl_r = if state.maximized { 0.0 } else { r };
        painter.rect_4corner(
            Rect::new(0.0, body_y, sidebar_w, hf - body_y),
            [0.0, 0.0, sidebar_bl_r, 0.0], // only bottom-left rounded
            fox.surface,
        );
        // Divider line between sidebar and content
        painter.rect_filled(
            Rect::new(sidebar_w, body_y, 1.0 * s, hf - body_y),
            0.0,
            fox.muted,
        );

        for (i, (panel, label)) in PANELS.iter().enumerate() {
            let y = body_y + i as f32 * item_h;
            let zone_id = ZONE_SIDEBAR_BASE + i as u32;
            let rect = Rect::new(0.0, y, sidebar_w, item_h);
            let zone_state = ix.add_zone(zone_id, rect);
            let is_active = *panel == active_panel;

            // Highlight active or hovered item
            if is_active {
                painter.rect_filled(rect, 0.0, fox.accent.with_alpha(0.2));
                // Active indicator bar on the left
                painter.rect_filled(
                    Rect::new(0.0, y + 4.0 * s, 3.0 * s, item_h - 8.0 * s),
                    2.0 * s,
                    fox.accent,
                );
            } else if zone_state.is_hovered() {
                painter.rect_filled(rect, 0.0, fox.text.with_alpha(0.06));
            }

            // Icon
            let icon_x = 16.0 * s;
            let icon_y = y + (item_h - icon_draw) / 2.0;
            let draw = TextureDraw::new(&icon_textures[i], icon_x, icon_y, icon_draw, icon_draw);
            tex_draws.push(draw);

            // Label text
            let text_x = icon_x + icon_draw + 12.0 * s;
            let text_y = y + (item_h - label_size) / 2.0;
            let text_color = if is_active { fox.accent } else { fox.text };
            text.queue(label, label_size, text_x, text_y, text_color, sidebar_w - text_x, sw, sh);
        }

        // ── Content area header ────────────────────────────────────────
        let header_label = PANELS.iter().find(|(p, _)| *p == active_panel).map(|(_, l)| *l).unwrap_or("");
        let header_size = 22.0 * s;
        let header_y = body_y + 16.0 * s;
        text.queue(header_label, header_size, content_x + 24.0 * s, header_y, fox.text, content_w, sw, sh);

        // Separator under content header
        let sep_y = header_y + header_size + 12.0 * s;
        painter.rect_filled(
            Rect::new(content_x + 16.0 * s, sep_y, content_w - 32.0 * s, 1.0 * s),
            0.0,
            fox.muted,
        );

        // ── Panel content ───────────────────────────────────────────────
        match active_panel {
            Panel::WindowManager => {
                panels::draw_wm_panel(
                    &mut config, &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, s, sw, sh,
                );
            }
            Panel::Power => {
                let panel_h = hf - panel_y;
                panels::draw_power_panel(
                    &mut config, &mut panel_state, &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::Display => {
                display_state.sync_from_config(&config);
                let panel_h = hf - panel_y;
                display_panel::draw_display_panel(
                    &mut config, &mut display_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &state.outputs, &state.output_mgr,
                );
                let thumb_draws = display_panel::collect_thumb_draws(&display_state, s);
                for td in thumb_draws {
                    tex_draws.push(td);
                }
            }
            Panel::Input => {
                input_panel::draw_input_panel(
                    &mut config, &mut input_state,
                    &mut painter, &mut text, &mut ix,
                    &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, s, sw, sh,
                    &mut tex_draws,
                );
            }
            Panel::AppIcons => {
                let panel_h = hf - panel_y;
                icon_panel::draw_icon_panel(
                    &mut icon_panel_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &mut tex_draws,
                );
            }
        }

        // Save/Cancel bar (only when config has unsaved changes)
        let dirty = config != saved_config;
        if dirty {
            panels::draw_save_cancel_bar(
                &mut painter, &mut text, &mut ix, &fox,
                content_x, content_w, hf, s, sw, sh,
            );
        }

        // Window border (skip when maximized)
        if !state.maximized { crate::chrome::draw_border(&mut painter, wf, hf, r); }

        // ── Render pass ─────────────────────────────────────────────────
        if let Ok(mut frame) = gpu.begin_frame("system-settings") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::rgba(0.0, 0.0, 0.0, 0.0));
            if !tex_draws.is_empty() {
                tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_queued(&gpu, frame.encoder_mut(), &view);
            frame.submit(&gpu.queue);
        }

        // Render popup surfaces
        if let Some(backend) = &mut state.popup_backend {
            backend.render_all();
        }

        ix.clear_scroll();
        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

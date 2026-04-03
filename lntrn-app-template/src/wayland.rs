use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuBar, MenuItem, PopupSurface,
    WaylandPopupBackend,
};
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
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
const TITLE_BAR_H: f32 = 40.0;
const CORNER_RADIUS: f32 = 12.0;

// ── Night Sky palette (inspired by indigo sky + pink clouds wallpaper) ──────

#[allow(dead_code)]
mod sky {
    use lntrn_render::Color;

    // Backgrounds — deep purple-indigo
    pub const BG_DEEP: Color       = Color::rgba(0.01, 0.00, 0.03, 0.90);
    pub const BG_SURFACE: Color    = Color::rgba(0.021, 0.005, 0.059, 0.90);

    // Text
    pub const TEXT_PRIMARY: Color   = Color::rgb(0.80, 0.76, 0.90);
    pub const TEXT_SECONDARY: Color = Color::rgb(0.45, 0.40, 0.58);

    // Subtle glows
    pub const GLOW_PINK: Color     = Color::rgba(0.40, 0.12, 0.28, 0.04);
    pub const GLOW_CYAN: Color     = Color::rgba(0.12, 0.30, 0.45, 0.04);

    // Borders — very subtle
    pub const BORDER_SUBTLE: Color = Color::rgba(0.30, 0.20, 0.50, 0.15);

    // Window control buttons
    pub const CLOSE_BG: Color      = Color::rgb(0.65, 0.18, 0.25);
    pub const CONTROL_HOVER: Color = Color::rgba(0.50, 0.38, 0.70, 0.25);
    pub const CONTROL_ICON: Color  = Color::rgb(0.55, 0.50, 0.68);
    pub const CLOSE_HOVER: Color   = Color::rgba(0.65, 0.18, 0.25, 0.35);

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

pub(crate) struct State {
    running: bool,
    configured: bool,
    pub(crate) frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    maximized: bool,
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
    // Decoration
    decoration_mgr: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    // Popup
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    /// Which wl_surface the pointer is currently in (for routing to popups)
    pointer_surface: Option<wl_surface::WlSurface>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0, maximized: false,
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false, right_pressed: false,
            scroll_delta: 0.0, pointer_serial: 0, enter_serial: 0,
            cursor_shape_mgr: None, cursor_shape_device: None,
            current_cursor_shape: None, pointer: None,
            key_pressed: None,
            decoration_mgr: None,
            popup_backend: None,
            popup_closed: false,
            pointer_surface: None,
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
                "zxdg_decoration_manager_v1" => {
                    state.decoration_mgr = Some(registry.bind(name, version.min(1), qh, ()));
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
                // Create cursor shape device if manager is available
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
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.key_pressed = Some(key);
                }
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
impl Dispatch<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, ()> for State {
    fn event(_: &mut Self, _: &zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, _: zxdg_decoration_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for State {
    fn event(_: &mut Self, _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, _: zxdg_toplevel_decoration_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

// ── Edge resize helper ──────────────────────────────────────────────────────

/// `controls_x` is the left edge of the window controls zone — if the cursor
/// is in the top-right corner (x > controls_x AND y < border), skip resize so
/// clicks reach the close/max/min buttons instead.
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
    if state.height == 0 { state.height = 640; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern".into());
    toplevel.set_app_id("lntrn-app-template".into());
    toplevel.set_min_size(640, 480);

    // Request client-side decorations so we control the title bar
    if let Some(mgr) = &state.decoration_mgr {
        let deco = mgr.get_toplevel_decoration(&toplevel, &qh, ());
        deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
    }

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
    let fox = FoxPalette::dark();
    let mut menu_bar = MenuBar::new(&fox);
    let mut right_click_menu = lntrn_ui::gpu::ContextMenu::new(
        lntrn_ui::gpu::ContextMenuStyle::from_palette(&fox),
    );

    // Initialize popup backend (clone xdg_surface to avoid borrow conflict)
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, &wm_base, &xdg_surf, vp, &gpu, scale, &qh,
        ));
    }

    let menus: Vec<(&str, Vec<MenuItem>)> = vec![
        ("File", vec![
            MenuItem::action(1, "New"),
            MenuItem::action_with(2, "Open", "Ctrl+O"),
            MenuItem::action_with(3, "Save", "Ctrl+S"),
            MenuItem::separator(),
            MenuItem::action_with(4, "Quit", "Ctrl+Q"),
        ]),
        ("Edit", vec![
            MenuItem::action_with(10, "Undo", "Ctrl+Z"),
            MenuItem::action_with(11, "Redo", "Ctrl+Shift+Z"),
            MenuItem::separator(),
            MenuItem::action_with(12, "Cut", "Ctrl+X"),
            MenuItem::action_with(13, "Copy", "Ctrl+C"),
            MenuItem::action_with(14, "Paste", "Ctrl+V"),
        ]),
        ("View", vec![
            MenuItem::toggle(20, "Dark Mode", true),
            MenuItem::toggle(21, "Show Sidebar", true),
            MenuItem::separator(),
            MenuItem::action(22, "Zoom In"),
            MenuItem::action(23, "Zoom Out"),
        ]),
        ("Help", vec![
            MenuItem::action(30, "About"),
        ]),
    ];

    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[ui-test] dispatch error: {e}");
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

        // Determine if pointer is on main surface or popup
        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state.popup_backend.as_ref()?.find_popup_id_by_wl_surface(ps)
        });

        // Cursor — route to main or popup InteractionContext
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if pointer_on_popup.is_some() {
            // Pointer is on a popup — don't send to main ix
            ix.on_cursor_left();
        } else if state.pointer_in_surface {
            ix.on_cursor_moved(cx, cy);
        } else {
            ix.on_cursor_left();
        }
        // Route cursor to the active popup, clear it from all others
        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface { pointer_on_popup } else { None };
            backend.route_cursor(active, cx, cy);
        }

        // Tell the right-click menu which popup depth has the pointer
        {
            let depth = pointer_on_popup.and_then(|pid| {
                (0..right_click_menu.popup_count())
                    .find(|&d| right_click_menu.popup_id_at_depth(d) == Some(pid))
            });
            right_click_menu.set_pointer_depth(depth);
        }

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            if key == KEY_ESC { state.running = false; }
        }

        // Left press
        let title_h = TITLE_BAR_H * s;
        if state.left_pressed {
            state.left_pressed = false;
            // Route click to popup if pointer is on a popup
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else {
            // Close right-click popup menu on any left click outside
            if right_click_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    right_click_menu.close_popups(backend);
                }
            }
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
            if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                if let Some(seat) = &state.seat {
                    toplevel.resize(seat, state.pointer_serial, edge);
                }
            } else if cy < title_h {
                // CSD title bar — check window control buttons (right side)
                let btn_r = 14.0 * s;
                let btn_y = title_h * 0.5;
                let close_cx = wf - 24.0 * s;
                let max_cx = wf - 58.0 * s;
                let min_cx = wf - 92.0 * s;
                let dist_close = ((cx - close_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                let dist_max = ((cx - max_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                let dist_min = ((cx - min_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                if dist_close < btn_r {
                    state.running = false;
                } else if dist_max < btn_r {
                    if state.maximized {
                        toplevel.unset_maximized();
                    } else {
                        toplevel.set_maximized();
                    }
                } else if dist_min < btn_r {
                    toplevel.set_minimized();
                } else {
                    // Drag to move
                    if let Some(seat) = &state.seat {
                        toplevel._move(seat, state.pointer_serial);
                    }
                }
            } else if menu_bar.on_click(&mut ix, &menus, s) {
                // Menu bar consumed the click
            } else {
                ix.on_left_pressed();
            }
            } // end else (not on popup)
        }

        // Left release
        if state.left_released {
            state.left_released = false;
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

        // Right press — open context menu via popup surface
        if state.right_pressed {
            state.right_pressed = false;
            menu_bar.close();
            // Close any existing popup menu
            if right_click_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    right_click_menu.close_popups(backend);
                }
            }
            right_click_menu.set_scale(s);
            let items = vec![
                MenuItem::action_with(50, "Cut", "Ctrl+X"),
                MenuItem::action_with(51, "Copy", "Ctrl+C"),
                MenuItem::action_with(52, "Paste", "Ctrl+V"),
                MenuItem::separator(),
                MenuItem::action(53, "Select All"),
                MenuItem::separator(),
                MenuItem::submenu(60, "Transform", vec![
                    MenuItem::action(61, "Uppercase"),
                    MenuItem::action(62, "Lowercase"),
                    MenuItem::action(63, "Title Case"),
                    MenuItem::separator(),
                    MenuItem::action(64, "Sort Lines"),
                    MenuItem::action(65, "Reverse Lines"),
                ]),
                MenuItem::separator(),
                MenuItem::toggle(54, "Word Wrap", true),
                MenuItem::checkbox(55, "Show Line Numbers", false),
                MenuItem::separator(),
                MenuItem::action(56, "Inspect Element"),
            ];
            // Use logical coordinates for popup positioning
            let lx = state.cursor_x as i32;
            let ly = state.cursor_y as i32;
            if let Some(backend) = &mut state.popup_backend {
                right_click_menu.open_popup(lx as f32, ly as f32, items, backend);
            }
        }

        // Handle popup_done (compositor dismissed popup)
        if state.popup_closed {
            state.popup_closed = false;
            if let Some(backend) = &mut state.popup_backend {
                right_click_menu.close_popups(backend);
            }
        }

        // Reset scroll
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

        // ── Background ───────────────────────────────────────────────────
        painter.rect_gradient_linear(
            Rect::new(0.0, 0.0, wf, hf), r,
            std::f32::consts::FRAC_PI_2,
            sky::BG_DEEP,
            sky::BG_SURFACE,
        );

        // Radial glow — pink, bottom-left
        painter.rect_gradient_radial(
            Rect::new(-wf * 0.35, hf * 0.5, wf * 0.8, hf * 0.8), 0.0,
            sky::GLOW_PINK,
            Color::TRANSPARENT,
        );

        // Radial glow — cyan, top-right
        painter.rect_gradient_radial(
            Rect::new(wf * 0.5, -hf * 0.25, wf * 0.8, hf * 0.7), 0.0,
            sky::GLOW_CYAN,
            Color::TRANSPARENT,
        );

        // ── CSD Title bar (seamless with background) ────────────────────
        // No separate title bar bg — it's the same gradient as the window

        // Title text — centered, subtle
        let title_sz = 20.0 * s;
        let title_str = "Lantern";
        let title_w = title_sz * 0.55 * title_str.len() as f32;
        text.queue(
            title_str, title_sz,
            (wf - title_w) * 0.5, (title_h - title_sz) * 0.5,
            sky::TEXT_SECONDARY, wf, sw, sh,
        );

        // ── Window controls (Windows-style icons in circles, right) ─────
        let btn_r = 14.0 * s;
        let btn_y = title_h * 0.5;
        let close_cx = wf - 28.0 * s;
        let max_cx = wf - 66.0 * s;
        let min_cx = wf - 104.0 * s;
        let icon_thick = 1.5 * s;

        let hover_close = {
            let dx = cx - close_cx; let dy = cy - btn_y;
            (dx * dx + dy * dy).sqrt() < btn_r
        };
        let hover_max = {
            let dx = cx - max_cx; let dy = cy - btn_y;
            (dx * dx + dy * dy).sqrt() < btn_r
        };
        let hover_min = {
            let dx = cx - min_cx; let dy = cy - btn_y;
            (dx * dx + dy * dy).sqrt() < btn_r
        };

        // Close — X icon
        if hover_close {
            painter.circle_filled(close_cx, btn_y, btn_r, sky::CLOSE_HOVER);
        }
        let x_sz = 5.0 * s;
        let close_icon = if hover_close { sky::CLOSE_BG } else { sky::CONTROL_ICON };
        painter.line(close_cx - x_sz, btn_y - x_sz, close_cx + x_sz, btn_y + x_sz, icon_thick, close_icon);
        painter.line(close_cx - x_sz, btn_y + x_sz, close_cx + x_sz, btn_y - x_sz, icon_thick, close_icon);

        // Maximize — square icon
        if hover_max {
            painter.circle_filled(max_cx, btn_y, btn_r, sky::CONTROL_HOVER);
        }
        let sq_sz = 5.0 * s;
        let max_icon = if hover_max { sky::TEXT_PRIMARY } else { sky::CONTROL_ICON };
        painter.rect_stroke_sdf(
            Rect::new(max_cx - sq_sz, btn_y - sq_sz, sq_sz * 2.0, sq_sz * 2.0),
            1.5 * s, icon_thick, max_icon,
        );

        // Minimize — horizontal line icon
        if hover_min {
            painter.circle_filled(min_cx, btn_y, btn_r, sky::CONTROL_HOVER);
        }
        let min_icon = if hover_min { sky::TEXT_PRIMARY } else { sky::CONTROL_ICON };
        painter.line(min_cx - x_sz, btn_y, min_cx + x_sz, btn_y, icon_thick, min_icon);

        // ── Window outer border (very subtle) ───────────────────────────
        if !state.maximized {
            painter.rect_stroke_sdf(
                Rect::new(0.0, 0.0, wf, hf), r, 1.0 * s,
                sky::BORDER_SUBTLE,
            );
        }

        // Context menus (drawn into painter on top of other shapes)
        menu_bar.context_menu.update(0.016);
        if let Some(evt) = menu_bar.context_menu.draw(
            &mut painter, &mut text, &mut ix, sw, sh,
        ) {
            use lntrn_ui::gpu::MenuEvent;
            if matches!(evt, MenuEvent::Action(_)) {
                menu_bar.close();
            }
        }

        // Right-click menu draws into popup surfaces
        right_click_menu.update(0.016);
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }
        if let Some(backend) = &mut state.popup_backend {
            if let Some(evt) = right_click_menu.draw_popups(backend) {
                use lntrn_ui::gpu::MenuEvent;
                if matches!(evt, MenuEvent::Action(_)) {
                    right_click_menu.close_popups(backend);
                }
            }
        }

        // Render pass — main window
        if let Ok(mut frame) = gpu.begin_frame("app-template") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
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

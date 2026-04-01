use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TexturePass};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext, MenuEvent, MenuItem,
    PopupSurface, ScrollArea, WaylandPopupBackend,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
        wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::app::{App, ContextTarget};
use crate::desktop::{self, DesktopApp};
use crate::fs::SortBy;
use crate::icons::IconCache;
use crate::layout::{content_rect, file_item_rect, grid_columns, grid_content_height, list_content_height, tree_content_height};
use crate::{PickConfig, PickResult};
use crate::settings::Settings;
use crate::{
    ClickAction, Gpu, CTX_COMPRESS, CTX_COPY, CTX_COPY_NAME, CTX_COPY_PATH, CTX_CUT,
    CTX_DUPLICATE, CTX_EXTRACT, CTX_NEW_FILE, CTX_NEW_FOLDER, CTX_NEW_FOLDER_BLUE,
    CTX_NEW_FOLDER_GREEN, CTX_NEW_FOLDER_ORANGE, CTX_NEW_FOLDER_PLAIN, CTX_NEW_FOLDER_PURPLE,
    CTX_NEW_FOLDER_RED, CTX_NEW_FOLDER_YELLOW, CTX_OPEN, CTX_OPEN_AS_ROOT,
    CTX_OPEN_TERMINAL, CTX_OPEN_WITH, CTX_OPEN_WITH_BASE, CTX_PASTE, CTX_PROPERTIES,
    CTX_CHANGE_ICON, CTX_RENAME, CTX_SELECT_ALL, CTX_SHOW_HIDDEN, CTX_SORT_BY, CTX_SORT_DATE, CTX_SORT_NAME,
    CTX_SORT_SIZE, CTX_SORT_TYPE, CTX_TRASH, SORT_RADIO_GROUP, VIEW_SLIDER_ID, VIEW_OPACITY_SLIDER_ID, ZONE_CLOSE,
    ZONE_FILE_ITEM_BASE, ZONE_MAXIMIZE, ZONE_MENU_VIEW, ZONE_MINIMIZE, ZONE_NAV_BACK,
    ZONE_NAV_FORWARD, ZONE_NAV_UP, ZONE_NAV_SEARCH, ZONE_SIDEBAR_ITEM_BASE,
    ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE, ZONE_TAB_NEW, ZONE_PATH_INPUT,
    ZONE_DRIVE_ITEM_BASE, ZONE_NAV_VIEW_TOGGLE, ZONE_TREE_ITEM_BASE,
    ZONE_DROP_MOVE, ZONE_DROP_COPY, ZONE_DROP_CANCEL,
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
const BTN_RIGHT: u32 = 0x111;

pub(crate) struct State {
    pub(crate) running: bool,
    pub(crate) configured: bool,
    pub(crate) frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    pub(crate) maximized: bool,
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
    right_clicked: bool,
    scroll_delta: f32,
    pointer_serial: u32,
    pointer_surface: Option<wl_surface::WlSurface>,
    // Keyboard
    ctrl: bool,
    shift: bool,
    key_pressed: Option<u32>,
    // Key repeat
    held_key: Option<u32>,
    repeat_deadline: std::time::Instant,
    repeat_started: bool,
    // Popups
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    // DnD
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    data_device: Option<wl_data_device::WlDataDevice>,
    dnd_active: bool,
    dnd_start_requested: bool,
    dnd_paths: Vec<std::path::PathBuf>,
    dnd_serial: u32,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0, maximized: false,
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false, right_clicked: false,
            scroll_delta: 0.0, pointer_serial: 0, pointer_surface: None,
            ctrl: false, shift: false, key_pressed: None,
            held_key: None, repeat_deadline: std::time::Instant::now(), repeat_started: false,
            popup_backend: None, popup_closed: false,
            data_device_manager: None, data_device: None,
            dnd_active: false, dnd_start_requested: false,
            dnd_paths: Vec::new(), dnd_serial: 0,
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
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, version.min(5), qh, ()));
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind(name, version.min(9), qh, ()));
                }
                "wl_data_device_manager" => {
                    state.data_device_manager = Some(registry.bind(name, version.min(3), qh, ()));
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
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
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
                // Parse states to detect maximized
                state.maximized = states.chunks_exact(4).any(|chunk| {
                    let val = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    val == xdg_toplevel::State::Maximized as u32
                });
            }
            xdg_toplevel::Event::Close => {
                eprintln!("[fox] received xdg_toplevel Close event");
                state.running = false;
            }
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
            wl_output::Event::Mode { width, .. } => {
                state.output_phys_width = width as u32;
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
        _: &mut Self, seat: &wl_seat::WlSeat,
        event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(cap) } = event {
            if cap.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
            if cap.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self, _: &wl_pointer::WlPointer,
        event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.pointer_surface = Some(surface);
                state.frame_done = true;
            }
            wl_pointer::Event::Leave { .. } => {
                // Flag to start Wayland DnD from main loop when pointer leaves during drag
                if !state.dnd_paths.is_empty() && !state.dnd_active {
                    state.dnd_start_requested = true;
                }
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
                if button == BTN_RIGHT && pressed { state.right_clicked = true; }
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
                    state.held_key = Some(key);
                    state.repeat_started = false;
                    state.repeat_deadline = std::time::Instant::now()
                        + std::time::Duration::from_millis(300);
                } else if key_state == WEnum::Value(wl_keyboard::KeyState::Released) {
                    if state.held_key == Some(key) {
                        state.held_key = None;
                    }
                }
                state.frame_done = true;
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.ctrl = mods_depressed & 4 != 0;
                state.shift = mods_depressed & 1 != 0;
            }
            _ => {}
        }
    }
}

// ── DnD Dispatch impls ──────────────────────────────────────────────────────

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &wl_data_device_manager::WlDataDeviceManager, _: wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(
        _state: &mut Self, _: &wl_data_device::WlDataDevice,
        _event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {}

    wayland_client::event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ())
    ]);
}

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(
        state: &mut Self, _source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                use std::io::Write;
                let mut file = std::fs::File::from(fd);
                if mime_type == "text/uri-list" {
                    for path in &state.dnd_paths {
                        let uri = format!("file://{}\r\n", path.display());
                        let _ = file.write_all(uri.as_bytes());
                    }
                } else if mime_type == "text/plain" {
                    let text: Vec<String> = state.dnd_paths.iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    let _ = file.write_all(text.join("\n").as_bytes());
                }
            }
            wl_data_source::Event::DndFinished | wl_data_source::Event::Cancelled => {
                state.dnd_active = false;
                state.dnd_paths.clear();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(_: &mut Self, _: &wl_data_offer::WlDataOffer, _: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run(pick: Option<PickConfig>) -> Result<()> {
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

    // Create data device for DnD (if available)
    if let (Some(mgr), Some(seat)) = (&state.data_device_manager, &state.seat) {
        state.data_device = Some(mgr.get_data_device(seat, &qh, ()));
    }

    let settings = Settings::load();

    // Default size from settings (logical)
    if state.width == 0 { state.width = settings.window_width as u32; }
    if state.height == 0 { state.height = settings.window_height as u32; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    let title = pick.as_ref()
        .and_then(|p| p.title.as_deref())
        .or(pick.as_ref().map(|p| p.default_title()))
        .unwrap_or("Lantern File Manager");
    toplevel.set_title(title.into());
    toplevel.set_app_id("lntrn-file-manager".into());
    toplevel.set_min_size(400, 300);
    surface.commit();

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    let scale_f = state.fractional_scale() as f32;
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

    // Popup backend
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, &wm_base, &xdg_surf, vp, &gpu.ctx, scale_f, &qh,
        ));
    }

    let palette = FoxPalette::dark();
    let mut view_menu = ContextMenu::new(ContextMenuStyle::from_palette(&palette));
    let mut context_menu = ContextMenu::new(ContextMenuStyle::from_palette(&palette));
    view_menu.set_scale(scale_f);
    context_menu.set_scale(scale_f);
    let mut open_with_apps: Vec<DesktopApp> = Vec::new();

    let mut app = App::new();
    app.icon_zoom = settings.icon_zoom;
    app.show_hidden = settings.show_hidden;
    app.sort_by = settings.sort_by_enum();
    if let Some(ref p) = pick {
        if let Some(ref dir) = p.start_dir {
            app.navigate_to(dir.clone());
        } else {
            app.navigate_to_home();
        }
        app.pick = Some(p.clone());
        if let Some(ref name) = p.save_name {
            app.save_name_buf = name.clone();
        }
    } else {
        app.navigate_to_home();
        // Restore pinned tabs from settings (preserving saved order)
        let mut pinned: Vec<crate::app::DirectoryTab> = Vec::new();
        for pinned_path in &settings.pinned_tabs {
            let path = std::path::PathBuf::from(pinned_path);
            if path.is_dir() {
                let mut tab = crate::app::DirectoryTab::new(path.clone());
                tab.pinned = true;
                tab.pinned_path = Some(path.clone());
                tab.entries = crate::fs::list_directory(&path, app.show_hidden, app.sort_by);
                pinned.push(tab);
            }
        }
        if !pinned.is_empty() {
            let count = pinned.len();
            // Prepend pinned tabs before the home tab
            pinned.append(&mut app.tabs);
            app.tabs = pinned;
            app.current_tab = 0;
            app.switch_tab(0);
        }
    }

    let mut input = InteractionContext::new();
    let mut icon_cache = IconCache::new();
    let mut file_info = crate::file_info::FileInfoCache::new();
    let mut settings = settings;
    let mut last_frame = Instant::now();
    let mut needs_anim = false;
    let mut last_dir_check = Instant::now();
    let mut last_dir_mtime: Option<std::time::SystemTime> = None;
    let mut last_dir_path = app.current_dir.clone();
    let mut last_tab_click: Option<(usize, Instant)> = None;
    // Pinned tab drag reorder state
    let mut tab_drag: Option<usize> = None;          // index of tab being dragged
    let mut tab_drag_press: Option<(usize, f32)> = None; // (tab_idx, press_x) for drag detection

    eprintln!("[fox] entering main loop, size={}x{}", state.width, state.height);

    while state.running {
        // Event dispatch
        if needs_anim {
            if let Err(e) = event_queue.flush() {
                eprintln!("[fox] flush error: {e}");
                break;
            }
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                eprintln!("[fox] dispatch_pending error: {e}");
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
            state.frame_done = true;
        } else {
            if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                eprintln!("[fox] blocking_dispatch error: {e}");
                break;
            }
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32().min(0.05);
        last_frame = now;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.ctx.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            view_menu.set_scale(scale_f);
            context_menu.set_scale(scale_f);
        }

        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let s = scale_f;

        // ── Cursor routing ──────────────────────────────────────────────
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;

        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state.popup_backend.as_ref()?.find_popup_id_by_wl_surface(ps)
        });

        if pointer_on_popup.is_some() {
            input.on_cursor_left();
        } else if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface { pointer_on_popup } else { None };
            backend.route_cursor(active, cx, cy);
        }

        // Set pointer depth for submenu close logic
        {
            let depth = pointer_on_popup.and_then(|pid| {
                (0..context_menu.popup_count())
                    .find(|&d| context_menu.popup_id_at_depth(d) == Some(pid))
            });
            context_menu.set_pointer_depth(depth);

            let vdepth = pointer_on_popup.and_then(|pid| {
                (0..view_menu.popup_count())
                    .find(|&d| view_menu.popup_id_at_depth(d) == Some(pid))
            });
            view_menu.set_pointer_depth(vdepth);
        }

        // ── Rubber band update ──────────────────────────────────────────
        if state.pointer_in_surface && app.rubber_band_start.is_some() {
            app.rubber_band_end = Some((cx, cy));
            update_rubber_band(&mut app, wf, hf, s);
        }

        // ── Drag detection ──────────────────────────────────────────────
        if state.pointer_in_surface && app.drag_item.is_none() {
            if let (Some(idx), Some((px, py))) = (app.pending_open, app.press_pos) {
                let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
                if dist > 5.0 {
                    app.drag_item = Some(idx);
                    app.drag_pos = Some((cx, cy));
                    app.pending_open = None;
                    app.press_pos = None;

                    // Prepare DnD paths for cross-window drag (deferred until pointer leaves)
                    let paths: Vec<std::path::PathBuf> = {
                        let selected = app.selected_paths();
                        if selected.is_empty() || !app.entries[idx].selected {
                            vec![app.entries[idx].path.clone()]
                        } else {
                            selected
                        }
                    };
                    state.dnd_paths = paths;
                    state.dnd_serial = state.pointer_serial;
                }
            }

            // Pinned tab drag detection
            if tab_drag.is_none() {
                if let Some((tab_idx, press_x)) = tab_drag_press {
                    if (cx - press_x).abs() > 5.0 {
                        tab_drag = Some(tab_idx);
                        tab_drag_press = None;
                    }
                }
            }
        }
        if app.drag_item.is_some() && state.pointer_in_surface {
            app.drag_pos = Some((cx, cy));
        }

        // ── Start Wayland DnD when pointer leaves during internal drag ─
        if state.dnd_start_requested {
            state.dnd_start_requested = false;
            if app.drag_item.is_some() && !state.dnd_active {
                if let (Some(mgr), Some(dd), Some(surf)) = (
                    &state.data_device_manager,
                    &state.data_device,
                    &state.surface,
                ) {
                    let source = mgr.create_data_source(&qh, ());
                    source.offer("text/uri-list".to_string());
                    source.offer("text/plain".to_string());
                    source.set_actions(
                        wl_data_device_manager::DndAction::Copy
                        | wl_data_device_manager::DndAction::Move,
                    );
                    dd.start_drag(Some(&source), surf, None, state.dnd_serial);
                    state.dnd_active = true;
                    // Clear internal drag — compositor owns the drag now
                    app.drag_item = None;
                    app.drag_pos = None;
                }
            }
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            handle_key(&mut app, &mut settings, &mut context_menu, &mut state.popup_backend, key, state.ctrl, state.shift, &mut state.running);
        }

        // Key repeat (for text editing modes)
        if let Some(key) = state.held_key {
            if (app.renaming.is_some() || app.path_editing || app.save_name_editing || app.searching)
                && std::time::Instant::now() >= state.repeat_deadline
            {
                handle_key(&mut app, &mut settings, &mut context_menu, &mut state.popup_backend, key, state.ctrl, state.shift, &mut state.running);
                let interval = if state.repeat_started { 30 } else { 300 };
                state.repeat_deadline = std::time::Instant::now()
                    + std::time::Duration::from_millis(interval);
                state.repeat_started = true;
                state.frame_done = true;
            }
        }

        // ── Scroll ──────────────────────────────────────────────────────
        if state.scroll_delta.abs() > 0.01 {
            let scroll = state.scroll_delta * s;
            input.on_scroll(scroll);
            let content = content_rect(wf, hf, s);
            let zoom = app.icon_zoom;
            let total_h = match app.view_mode {
                crate::app::ViewMode::Grid => {
                    let cols = grid_columns(content.w, s, zoom);
                    grid_content_height(app.entries.len(), cols, s, zoom)
                }
                crate::app::ViewMode::List => list_content_height(app.entries.len(), s),
                crate::app::ViewMode::Tree => tree_content_height(app.tree_entries.len(), s),
            };
            ScrollArea::apply_scroll(&mut app.scroll_offset, scroll, total_h, content.h);
            state.scroll_delta = 0.0;
        }

        // ── Left press ──────────────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(pid) = pointer_on_popup {
                // Click is on a popup surface — route to popup interaction
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else if app.pending_drop.is_some() {
                // Drop confirmation modal — handle buttons
                if let Some(zone) = input.on_left_pressed() {
                    match zone {
                        ZONE_DROP_MOVE => {
                            if let Some(drop) = app.pending_drop.take() {
                                for src in &drop.sources {
                                    if let Some(name) = src.file_name() {
                                        let dest = drop.dest_dir.join(name);
                                        let _ = std::fs::rename(src, &dest);
                                    }
                                }
                                app.reload();
                                if let Some(tab) = drop.reload_tab {
                                    app.reload_tab(tab);
                                }
                            }
                        }
                        ZONE_DROP_COPY => {
                            if let Some(drop) = app.pending_drop.take() {
                                let sources = drop.sources.clone();
                                let dest_dir = drop.dest_dir.clone();
                                std::thread::spawn(move || {
                                    for src in &sources {
                                        if let Some(name) = src.file_name() {
                                            let dest = dest_dir.join(name);
                                            if src.is_dir() {
                                                copy_dir_recursive(src, &dest);
                                            } else {
                                                let _ = std::fs::copy(src, &dest);
                                            }
                                        }
                                    }
                                });
                                app.reload();
                                if let Some(tab) = drop.reload_tab {
                                    app.reload_tab(tab);
                                }
                            }
                        }
                        ZONE_DROP_CANCEL => {
                            app.pending_drop = None;
                        }
                        _ => {}
                    }
                }
            } else if app.properties.is_some() {
                // Properties dialog is open — close on any click
                // (the close button zone handles itself via on_left_pressed)
                if let Some(zone) = input.on_left_pressed() {
                    if zone == 800 || zone == 801 {
                        // Close button or backdrop
                        app.properties = None;
                    }
                } else {
                    app.properties = None;
                }
            } else if context_menu.is_open() {
                // Click outside popup — close it
                if let Some(backend) = &mut state.popup_backend {
                    context_menu.close_popups(backend);
                }
            } else if view_menu.is_open() {
                // View menu popup is open — click outside closes it
                if let Some(backend) = &mut state.popup_backend {
                    view_menu.close_popups(backend);
                }
            } else {
                // Edge resize
                let border = 10.0 * s;
                let resize_edge = edge_resize(cx, cy, wf, hf, border);
                if let Some(edge) = resize_edge {
                    if let Some(seat) = &state.seat {
                        toplevel.resize(seat, state.pointer_serial, edge);
                    }
                } else {
                    let action = handle_click(
                        &mut input, &mut app, &mut view_menu, &mut state.popup_backend,
                        &mut last_tab_click, &mut tab_drag_press, s,
                        settings.bg_opacity,
                    );
                    match action {
                        ClickAction::None => {
                            // Title bar drag
                            let title_h = crate::layout::title_bar_rect(0.0, s).h;
                            if cy < title_h && !view_menu.is_open() {
                                if let Some(seat) = &state.seat {
                                    toplevel._move(seat, state.pointer_serial);
                                }
                            } else if app.pending_open.is_none() {
                                let cr = content_rect(wf, hf, s);
                                if cr.contains(cx, cy) {
                                    app.clear_selection();
                                    app.rubber_band_start = Some((cx, cy));
                                    app.rubber_band_end = Some((cx, cy));
                                }
                            }
                        }
                        ClickAction::Close => {
                            eprintln!("[fox] close button clicked");
                            state.running = false;
                        }
                        ClickAction::Minimize => {
                            toplevel.set_minimized();
                        }
                        ClickAction::ToggleMaximize => {
                            if state.maximized {
                                toplevel.unset_maximized();
                            } else {
                                toplevel.set_maximized();
                            }
                        }
                    }
                }
            }
        }

        // ── Left release ────────────────────────────────────────────────
        if state.left_released {
            state.left_released = false;
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_released();
                    }
                }
            } else {
                if app.rubber_band_start.is_some() {
                    app.rubber_band_start = None;
                    app.rubber_band_end = None;
                }
                // Pinned tab drag release — reorder
                if let Some(src_idx) = tab_drag.take() {
                    let tab_bar_rect = crate::layout::tab_bar_rect(wf, s);
                    let tab_labels = app.tab_labels();
                    let tab_label_refs: Vec<&str> = tab_labels.iter().map(|s| s.as_str()).collect();
                    let rects = lntrn_ui::gpu::TabBar::new(tab_bar_rect)
                        .tabs(&tab_label_refs)
                        .scale(s)
                        .tab_rects();
                    // Find which tab slot the cursor is over
                    if let Some((cursor_x, _)) = input.cursor() {
                        let target_idx = rects.iter().position(|r| r.contains(cursor_x, r.y + r.h * 0.5))
                            .unwrap_or(src_idx);
                        // Only reorder among pinned tabs
                        if target_idx != src_idx
                            && target_idx < app.tabs.len()
                            && app.tabs[target_idx].pinned
                        {
                            let tab = app.tabs.remove(src_idx);
                            app.tabs.insert(target_idx, tab);
                            // Fix current_tab index
                            if app.current_tab == src_idx {
                                app.current_tab = target_idx;
                            } else if src_idx < app.current_tab && target_idx >= app.current_tab {
                                app.current_tab -= 1;
                            } else if src_idx > app.current_tab && target_idx <= app.current_tab {
                                app.current_tab += 1;
                            }
                        }
                    }
                } else if let Some(drag_idx) = app.drag_item.take() {
                    handle_drop(&mut app, &input, wf, hf, s, drag_idx);
                    app.pending_open = None;
                    // Clean up DnD state (internal drop, Wayland DnD never started)
                    state.dnd_paths.clear();
                    state.dnd_start_requested = false;
                } else if let Some(idx) = app.pending_open.take() {
                    app.on_item_click(idx);
                }
                tab_drag_press = None;
                input.on_left_released();
            }
        }

        // ── Right click ─────────────────────────────────────────────────
        if state.right_clicked {
            state.right_clicked = false;
            // Close existing menus first
            if view_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    view_menu.close_popups(backend);
                }
            }
            if context_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    context_menu.close_popups(backend);
                }
            }
            handle_right_click(&mut app, &mut context_menu, &mut state.popup_backend, &input, &mut open_with_apps, wf, hf, s);
        }

        // ── Popup closed by compositor ──────────────────────────────────
        if state.popup_closed {
            state.popup_closed = false;
            if let Some(backend) = &mut state.popup_backend {
                view_menu.close_popups(backend);
                context_menu.close_popups(backend);
            }
        }

        // ── Update menus ────────────────────────────────────────────────
        view_menu.update(dt);
        context_menu.update(dt);

        // ── Begin popup frames ──────────────────────────────────────────
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }

        // ── Render ──────────────────────────────────────────────────────
        let render_palette = palette.with_bg_opacity(settings.bg_opacity);
        crate::render::render_frame(
            &mut gpu, &mut app, &mut input, &mut icon_cache, &mut file_info,
            &render_palette, s, state.maximized, &mut view_menu, tab_drag,
            settings.bg_opacity,
        );

        // ── Draw & render popup surfaces ────────────────────────────────
        if let Some(backend) = &mut state.popup_backend {
            // View menu popup
            if let Some(evt) = view_menu.draw_popups(backend) {
                if let MenuEvent::SliderChanged { id, value } = evt {
                    if id == VIEW_SLIDER_ID {
                        app.icon_zoom = value;
                        let content = content_rect(wf, hf, s);
                        ScrollArea::apply_scroll(
                            &mut app.scroll_offset, 0.0,
                            grid_content_height(app.entries.len(),
                                grid_columns(content.w, s, value), s, value),
                            content.h,
                        );
                    } else if id == VIEW_OPACITY_SLIDER_ID {
                        settings.bg_opacity = value.clamp(0.0, 1.0);
                        settings.save();
                    }
                } else if matches!(evt, MenuEvent::Action(_)) {
                    view_menu.close_popups(backend);
                }
            }
            // Right-click context menu popup
            if let Some(evt) = context_menu.draw_popups(backend) {
                if matches!(evt, MenuEvent::Action(_)) {
                    context_menu.close_popups(backend);
                }
                handle_ctx_event(&mut app, &mut settings, &mut context_menu, &mut state.popup_backend, &open_with_apps, evt);
            }
            // Render popup surfaces, injecting folder icon textures for swatch items
            let swatches = context_menu.swatch_rects();
            let root_pid = context_menu.root_popup_id();
            if let Some(backend) = &mut state.popup_backend {
                backend.render_all_except(root_pid.filter(|_| !swatches.is_empty()));

                // Render the root popup with texture icons for swatches
                if !swatches.is_empty() {
                    if let Some(pid) = root_pid {
                        if let Some(ctx) = backend.popup_render(pid) {
                            if let Ok(mut frame) = ctx.gpu.begin_frame("popup") {
                                let view = frame.view().clone();
                                // Pass 1: shapes
                                ctx.painter.render_pass(
                                    &ctx.gpu, frame.encoder_mut(), &view,
                                    lntrn_render::Color::TRANSPARENT,
                                );
                                // Pre-load all folder color textures into cache
                                for &(sid, _, _, _) in &swatches {
                                    let color_name = match sid {
                                        CTX_NEW_FOLDER_RED => "red",
                                        CTX_NEW_FOLDER_ORANGE => "orange",
                                        CTX_NEW_FOLDER_YELLOW => "yellow",
                                        CTX_NEW_FOLDER_GREEN => "green",
                                        CTX_NEW_FOLDER_BLUE => "blue",
                                        CTX_NEW_FOLDER_PURPLE => "purple",
                                        _ => "",
                                    };
                                    icon_cache.get_or_load_folder_color(
                                        color_name, &ctx.gpu, &ctx.tex_pass,
                                    );
                                }
                                // Pass 2: folder icon textures (all loaded, only immutable borrows now)
                                let mut tex_draws = Vec::new();
                                for &(sid, ix, iy, isz) in &swatches {
                                    let color_name = match sid {
                                        CTX_NEW_FOLDER_RED => "red",
                                        CTX_NEW_FOLDER_ORANGE => "orange",
                                        CTX_NEW_FOLDER_YELLOW => "yellow",
                                        CTX_NEW_FOLDER_GREEN => "green",
                                        CTX_NEW_FOLDER_BLUE => "blue",
                                        CTX_NEW_FOLDER_PURPLE => "purple",
                                        _ => "",
                                    };
                                    if let Some(tex) = icon_cache.get_folder_color(color_name) {
                                        let (dx, dy, dw, dh) = crate::icons::fit_in_box(tex, ix, iy, isz, isz);
                                        tex_draws.push(lntrn_render::TextureDraw::new(tex, dx, dy, dw, dh));
                                    }
                                }
                                if !tex_draws.is_empty() {
                                    ctx.tex_pass.render_pass(
                                        &ctx.gpu, frame.encoder_mut(), &view, &tex_draws, None,
                                    );
                                }
                                // Pass 3: text
                                ctx.text.render_queued(&ctx.gpu, frame.encoder_mut(), &view);
                                frame.submit(&ctx.gpu.queue);
                            }
                            backend.commit_popup(pid);
                        }
                    }
                }
            }
        }

        surface.frame(&qh, ());
        surface.commit();

        // Poll search results from background thread
        app.poll_search();

        // ── Auto-refresh: check directory mtime every 3 seconds ─────
        if app.current_dir != last_dir_path {
            // Directory changed (navigation) — reset tracker, don't reload
            last_dir_path = app.current_dir.clone();
            last_dir_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified()).ok();
            last_dir_check = Instant::now();
        } else if last_dir_check.elapsed() >= Duration::from_secs(3) {
            last_dir_check = Instant::now();
            let current_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified())
                .ok();
            if current_mtime != last_dir_mtime {
                last_dir_mtime = current_mtime;
                app.reload();
            }
        }

        needs_anim = view_menu.is_open() || context_menu.is_open()
            || app.drag_item.is_some() || app.rubber_band_start.is_some()
            || state.held_key.is_some()
            || app.search_rx.is_some()
            || tab_drag.is_some();
    }

    eprintln!("[fox] exited main loop");

    // Pick mode: output results
    if pick.is_some() {
        match app.pick_result.take() {
            Some(PickResult::Selected(paths)) => {
                for p in &paths {
                    println!("{}", p.display());
                }
                return Ok(());
            }
            _ => {
                // Cancelled — exit with code 1
                std::process::exit(1);
            }
        }
    }

    // Save settings on exit (normal mode only)
    settings.icon_zoom = app.icon_zoom;
    settings.show_hidden = app.show_hidden;
    settings.set_sort_by(app.sort_by);
    settings.window_width = state.width as f32;
    settings.window_height = state.height as f32;
    settings.pinned_tabs = app.tabs.iter()
        .filter(|t| t.pinned)
        .map(|t| {
            t.pinned_path.as_ref().unwrap_or(&t.path)
                .to_string_lossy().to_string()
        })
        .collect();
    settings.save();

    Ok(())
}

// ── Helper functions ────────────────────────────────────────────────────────

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

fn handle_click(
    input: &mut InteractionContext,
    app: &mut App,
    view_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    last_tab_click: &mut Option<(usize, Instant)>,
    tab_drag_press: &mut Option<(usize, f32)>,
    s: f32,
    bg_opacity: f32,
) -> ClickAction {
    if let Some(zone_id) = input.on_left_pressed() {
        // If path editing, commit on any click outside the path input
        if app.path_editing && zone_id != ZONE_PATH_INPUT {
            app.commit_path_edit();
            return ClickAction::None;
        }
        // If renaming, commit on any click outside the rename input
        if app.renaming.is_some() && zone_id != crate::ZONE_RENAME_INPUT {
            app.commit_rename();
            return ClickAction::None;
        }
        match zone_id {
            ZONE_CLOSE => return ClickAction::Close,
            ZONE_MINIMIZE => return ClickAction::Minimize,
            ZONE_MAXIMIZE => return ClickAction::ToggleMaximize,
            ZONE_MENU_VIEW => {
                if !view_menu.is_open() {
                    // Open as popup surface (like right-click menu)
                    let label_x = 10.0; // logical coords for popup positioner
                    let label_y = (crate::layout::title_bar_rect(0.0, s).h / s) as f32;
                    view_menu.set_scale(s);
                    if let Some(backend) = popup_backend {
                        view_menu.open_popup(
                            label_x as f32, label_y,
                            vec![
                                MenuItem::slider(VIEW_SLIDER_ID, "Icon Size", app.icon_zoom),
                                MenuItem::slider(VIEW_OPACITY_SLIDER_ID, "Opacity", bg_opacity),
                            ],
                            backend,
                        );
                    }
                } else {
                    if let Some(backend) = popup_backend {
                        view_menu.close_popups(backend);
                    }
                }
            }
            ZONE_NAV_VIEW_TOGGLE => {
                app.cycle_view_mode();
            }
            ZONE_PATH_INPUT => {
                if !app.path_editing {
                    app.start_path_edit();
                }
            }
            ZONE_NAV_BACK => app.go_back(),
            ZONE_NAV_FORWARD => app.go_forward(),
            ZONE_NAV_UP => app.go_up(),
            ZONE_NAV_SEARCH => {
                if app.searching {
                    app.close_search();
                } else {
                    app.start_search();
                }
            }
            ZONE_TAB_NEW => {
                app.new_tab();
            }
            id if id >= ZONE_TAB_CLOSE_BASE && id < ZONE_TAB_NEW => {
                let idx = (id - ZONE_TAB_CLOSE_BASE) as usize;
                app.close_tab(idx);
            }
            id if id >= ZONE_TAB_BASE && id < ZONE_TAB_CLOSE_BASE => {
                let idx = (id - ZONE_TAB_BASE) as usize;
                let now = Instant::now();
                let is_double = if let Some((prev_idx, prev_time)) = *last_tab_click {
                    prev_idx == idx && now.duration_since(prev_time).as_millis() < 400
                } else {
                    false
                };
                if is_double {
                    app.toggle_pin(idx);
                    *last_tab_click = None;
                } else {
                    app.switch_tab(idx);
                    *last_tab_click = Some((idx, now));
                    // Record press for pinned tab drag detection
                    if idx < app.tabs.len() && app.tabs[idx].pinned {
                        if let Some((cx, _)) = input.cursor() {
                            *tab_drag_press = Some((idx, cx));
                        }
                    }
                }
            }
            id if id >= ZONE_TREE_ITEM_BASE => {
                let idx = (id - ZONE_TREE_ITEM_BASE) as usize;
                if idx < app.tree_entries.len() {
                    let te = &app.tree_entries[idx];
                    if te.entry.is_dir {
                        let path = te.entry.path.clone();
                        app.toggle_tree_expand(path);
                    } else {
                        let path = te.entry.path.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                        });
                    }
                }
            }
            id if id >= ZONE_FILE_ITEM_BASE => {
                let idx = (id - ZONE_FILE_ITEM_BASE) as usize;
                if app.searching && !app.search_buf.is_empty() {
                    // Search result clicked — navigate to parent and highlight,
                    // or open file directly
                    if idx < app.search_results.len() {
                        let entry = app.search_results[idx].clone();
                        if entry.is_dir {
                            app.close_search();
                            app.navigate_to(entry.path);
                        } else {
                            let path = entry.path.clone();
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(&path).spawn();
                            });
                        }
                    }
                } else {
                    if idx < app.entries.len() {
                        app.select_item(idx);
                        app.pending_open = Some(idx);
                        if let Some((cx, cy)) = input.cursor() {
                            app.press_pos = Some((cx, cy));
                        }
                    }
                }
            }
            id if id >= ZONE_SIDEBAR_ITEM_BASE && id < ZONE_DRIVE_ITEM_BASE => {
                let idx = (id - ZONE_SIDEBAR_ITEM_BASE) as usize;
                app.on_sidebar_click(idx);
            }
            id if id >= ZONE_DRIVE_ITEM_BASE && id < ZONE_TAB_BASE => {
                let idx = (id - ZONE_DRIVE_ITEM_BASE) as usize;
                app.on_drive_click(idx);
            }
            crate::ZONE_PICK_CONFIRM => {
                app.confirm_pick();
                return ClickAction::Close;
            }
            crate::ZONE_PICK_CANCEL => {
                app.cancel_pick();
                return ClickAction::Close;
            }
            crate::ZONE_PICK_FILTER => {
                app.cycle_filter();
            }
            crate::ZONE_PICK_FILENAME => {
                if !app.save_name_editing {
                    app.save_name_editing = true;
                    app.save_name_cursor = app.save_name_buf.len();
                }
            }
            _ => {}
        }
    }
    ClickAction::None
}

fn handle_right_click(
    app: &mut App,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    input: &InteractionContext,
    open_with_apps: &mut Vec<DesktopApp>,
    wf: f32, hf: f32, s: f32,
) {
    let Some((cx, cy)) = input.cursor() else { return };
    let cr = content_rect(wf, hf, s);
    if !cr.contains(cx, cy) { return; }

    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;

    let clicked_item = (0..app.entries.len()).find(|&i| {
        file_item_rect(i, cols, cr.x, base_y, s, zoom).contains(cx, cy)
    });

    let has_clipboard = app.clipboard.is_some();
    let items = if let Some(idx) = clicked_item {
        app.select_item(idx);
        app.context_target = Some(ContextTarget::Item(idx));
        let is_dir = app.entries[idx].is_dir;
        let mut v = vec![MenuItem::action(CTX_OPEN, "Open")];
        if !is_dir {
            // Discover apps for this file's MIME type
            let ext = app.entries[idx].extension();
            *open_with_apps = desktop::apps_for_extension(&ext);
            if !open_with_apps.is_empty() {
                let children: Vec<MenuItem> = open_with_apps.iter().enumerate()
                    .map(|(i, a)| MenuItem::action(CTX_OPEN_WITH_BASE + i as u32, &a.name))
                    .collect();
                v.push(MenuItem::submenu(CTX_OPEN_WITH, "Open With", children));
            }
        }
        v.push(MenuItem::action(CTX_OPEN_AS_ROOT, "Open as Root"));
        v.push(MenuItem::separator());
        v.push(MenuItem::action_with(CTX_CUT, "Cut", "Ctrl+X"));
        v.push(MenuItem::action_with(CTX_COPY, "Copy", "Ctrl+C"));
        if has_clipboard {
            v.push(MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"));
        }
        v.push(MenuItem::action(CTX_DUPLICATE, "Duplicate"));
        v.push(MenuItem::separator());
        v.push(MenuItem::action(CTX_COPY_PATH, "Copy Path"));
        v.push(MenuItem::action(CTX_COPY_NAME, "Copy Name"));
        v.push(MenuItem::separator());
        if !is_dir && crate::file_ops::is_archive(&app.entries[idx].path) {
            v.push(MenuItem::action(CTX_EXTRACT, "Extract Here"));
        }
        v.push(MenuItem::action(CTX_COMPRESS, "Compress"));
        v.push(MenuItem::separator());
        v.push(MenuItem::action(CTX_RENAME, "Rename"));
        v.push(MenuItem::action_danger(CTX_TRASH, "Move to Trash"));
        v.push(MenuItem::separator());
        if is_dir {
            v.push(MenuItem::action(CTX_CHANGE_ICON, "Change Icon"));
        }
        v.push(MenuItem::action(CTX_PROPERTIES, "Properties"));
        v
    } else {
        app.clear_selection();
        app.context_target = Some(ContextTarget::Empty);
        let mut v = Vec::new();
        if has_clipboard {
            v.push(MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"));
            v.push(MenuItem::separator());
        }
        v.push(MenuItem::action(CTX_NEW_FILE, "New File"));
        v.push(MenuItem::color_swatches("New Folder", vec![
            (CTX_NEW_FOLDER_PLAIN,  Color::from_rgb8(140, 140, 140)),
            (CTX_NEW_FOLDER_RED,    Color::from_rgb8(220, 60, 60)),
            (CTX_NEW_FOLDER_ORANGE, Color::from_rgb8(230, 150, 40)),
            (CTX_NEW_FOLDER_YELLOW, Color::from_rgb8(220, 200, 50)),
            (CTX_NEW_FOLDER_GREEN,  Color::from_rgb8(70, 180, 80)),
            (CTX_NEW_FOLDER_BLUE,   Color::from_rgb8(60, 130, 220)),
            (CTX_NEW_FOLDER_PURPLE, Color::from_rgb8(160, 80, 210)),
        ]));
        v.push(MenuItem::separator());
        v.push(MenuItem::submenu(CTX_SORT_BY, "Sort By", vec![
            MenuItem::radio(CTX_SORT_NAME, SORT_RADIO_GROUP, "Name", app.sort_by == SortBy::Name),
            MenuItem::radio(CTX_SORT_SIZE, SORT_RADIO_GROUP, "Size", app.sort_by == SortBy::Size),
            MenuItem::radio(CTX_SORT_DATE, SORT_RADIO_GROUP, "Date Modified", app.sort_by == SortBy::Date),
            MenuItem::radio(CTX_SORT_TYPE, SORT_RADIO_GROUP, "Type", app.sort_by == SortBy::Type),
        ]));
        v.push(MenuItem::separator());
        v.push(MenuItem::action(CTX_SELECT_ALL, "Select All"));
        v.push(MenuItem::action(CTX_OPEN_TERMINAL, "Open Terminal Here"));
        v.push(MenuItem::separator());
        v.push(MenuItem::checkbox(CTX_SHOW_HIDDEN, "Show Hidden Files", app.show_hidden));
        v
    };

    // Use logical coordinates for popup positioning
    let lx = (cx / s) as f32;
    let ly = (cy / s) as f32;
    context_menu.set_scale(s);
    if let Some(backend) = popup_backend {
        context_menu.open_popup(lx, ly, items, backend);
    }
}

fn handle_ctx_event(
    app: &mut App, settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    open_with_apps: &[DesktopApp],
    event: MenuEvent,
) {
    match event {
        MenuEvent::Action(id) => {
            match id {
                CTX_OPEN => app.open_selected(),
                CTX_CUT => app.cut_selected(),
                CTX_COPY => app.copy_selected(),
                CTX_PASTE => app.paste(),
                CTX_RENAME => {
                    if let Some(ContextTarget::Item(idx)) = app.context_target {
                        app.start_rename(idx);
                    }
                }
                CTX_TRASH => app.trash_selected(),
                CTX_COPY_PATH => app.copy_path_to_clipboard(),
                CTX_COPY_NAME => app.copy_name_to_clipboard(),
                CTX_DUPLICATE => app.duplicate_selected(),
                CTX_COMPRESS => app.compress_selected(),
                CTX_EXTRACT => app.extract_selected(),
                CTX_OPEN_AS_ROOT => app.open_as_root(),
                CTX_CHANGE_ICON => {
                    // Spawn a file picker to choose an icon image
                    if let Some(crate::app::ContextTarget::Item(idx)) = app.context_target.clone() {
                        if idx < app.entries.len() && app.entries[idx].is_dir {
                            let folder_path = app.entries[idx].path.clone();
                            std::thread::spawn(move || {
                                let output = std::process::Command::new("lntrn-file-manager")
                                    .args([
                                        "--pick",
                                        "--title", "Choose Folder Icon",
                                        "--filters", "Images:*.png,*.svg,*.jpg,*.jpeg,*.webp,*.ico",
                                    ])
                                    .output();
                                if let Ok(out) = output {
                                    if out.status.success() {
                                        let chosen = String::from_utf8_lossy(&out.stdout)
                                            .trim().to_string();
                                        if !chosen.is_empty() {
                                            crate::icons::set_folder_icon(&folder_path, &chosen);
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
                CTX_PROPERTIES => {
                    let path = if let Some(ref target) = app.context_target {
                        match target {
                            crate::app::ContextTarget::Item(idx) => {
                                if *idx < app.entries.len() {
                                    Some(app.entries[*idx].path.clone())
                                } else { None }
                            }
                            crate::app::ContextTarget::Empty => {
                                Some(app.current_dir.clone())
                            }
                        }
                    } else { None };
                    if let Some(path) = path {
                        app.properties = crate::properties::FileProperties::from_path(&path);
                    }
                }
                CTX_NEW_FOLDER => {
                    let target = app.current_dir.join("New Folder");
                    if app.root_mode {
                        let _ = std::process::Command::new("pkexec")
                            .args(["mkdir", "--"]).arg(&target).status();
                    } else {
                        let _ = std::fs::create_dir(&target);
                    }
                    app.reload();
                }
                CTX_NEW_FOLDER_PLAIN | CTX_NEW_FOLDER_RED | CTX_NEW_FOLDER_ORANGE
                | CTX_NEW_FOLDER_YELLOW | CTX_NEW_FOLDER_GREEN | CTX_NEW_FOLDER_BLUE
                | CTX_NEW_FOLDER_PURPLE => {
                    let target = app.current_dir.join("New Folder");
                    if app.root_mode {
                        let _ = std::process::Command::new("pkexec")
                            .args(["mkdir", "--"]).arg(&target).status();
                    } else {
                        let _ = std::fs::create_dir(&target);
                    }
                    let color = match id {
                        CTX_NEW_FOLDER_RED => "red",
                        CTX_NEW_FOLDER_ORANGE => "orange",
                        CTX_NEW_FOLDER_YELLOW => "yellow",
                        CTX_NEW_FOLDER_GREEN => "green",
                        CTX_NEW_FOLDER_BLUE => "blue",
                        CTX_NEW_FOLDER_PURPLE => "purple",
                        _ => "",
                    };
                    if !color.is_empty() {
                        crate::icons::set_folder_color(&target, color);
                    }
                    app.reload();
                }
                CTX_NEW_FILE => {
                    let target = app.current_dir.join("New File");
                    if app.root_mode {
                        let _ = std::process::Command::new("pkexec")
                            .args(["touch", "--"]).arg(&target).status();
                    } else {
                        let _ = std::fs::write(&target, "");
                    }
                    app.reload();
                }
                CTX_SELECT_ALL => app.select_all(),
                CTX_OPEN_TERMINAL => app.open_in_terminal(),
                id if id >= CTX_OPEN_WITH_BASE => {
                    let app_idx = (id - CTX_OPEN_WITH_BASE) as usize;
                    if app_idx < open_with_apps.len() {
                        let selected: Vec<_> = app.entries.iter()
                            .filter(|e| e.selected)
                            .map(|e| e.path.clone())
                            .collect();
                        for file_path in &selected {
                            desktop::launch_app(&open_with_apps[app_idx].exec, file_path);
                        }
                    }
                }
                _ => {}
            }
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        MenuEvent::CheckboxToggled { id, checked } => {
            if id == CTX_SHOW_HIDDEN {
                app.show_hidden = checked;
                settings.show_hidden = checked;
                app.reload();
                if let Some(backend) = popup_backend {
                    context_menu.close_popups(backend);
                }
            }
        }
        MenuEvent::RadioSelected { id, .. } => {
            let sort = match id {
                CTX_SORT_NAME => SortBy::Name,
                CTX_SORT_SIZE => SortBy::Size,
                CTX_SORT_DATE => SortBy::Date,
                CTX_SORT_TYPE => SortBy::Type,
                _ => return,
            };
            app.sort_by = sort;
            settings.set_sort_by(sort);
            app.reload();
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        _ => {}
    }
}

fn update_rubber_band(app: &mut App, wf: f32, hf: f32, s: f32) {
    let (Some(start), Some(end)) = (app.rubber_band_start, app.rubber_band_end) else { return };
    let cr = content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;
    let band = Rect::new(
        start.0.min(end.0), start.1.min(end.1),
        (start.0 - end.0).abs(), (start.1 - end.1).abs(),
    );
    for i in 0..app.entries.len() {
        let ir = file_item_rect(i, cols, cr.x, base_y, s, zoom);
        app.entries[i].selected = ir.intersect(&band).is_some();
    }
}

fn handle_drop(app: &mut App, input: &InteractionContext, wf: f32, hf: f32, s: f32, drag_idx: usize) {
    use crate::app::PendingDrop;
    let Some((cx, cy)) = input.cursor() else { return };

    // Collect all selected paths (or just the dragged one if not selected)
    let sources: Vec<std::path::PathBuf> = {
        let selected = app.selected_paths();
        if selected.is_empty() || !app.entries[drag_idx].selected {
            vec![app.entries[drag_idx].path.clone()]
        } else {
            selected
        }
    };

    // Check if dropped on a zone (tab, sidebar, or file item)
    if let Some(zone_id) = input.zone_at(cx, cy) {
        // ── Drop on a tab ───────────────────────────────────────────
        if zone_id >= ZONE_TAB_BASE && zone_id < ZONE_TAB_CLOSE_BASE {
            let tab_idx = (zone_id - ZONE_TAB_BASE) as usize;
            if tab_idx < app.tabs.len() {
                let dest_dir = app.tabs[tab_idx].path.clone();
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: Some(tab_idx),
                });
            }
            return;
        }
        // ── Drop on a sidebar place ─────────────────────────────────
        if zone_id >= ZONE_SIDEBAR_ITEM_BASE && zone_id < ZONE_DRIVE_ITEM_BASE {
            let place_idx = (zone_id - ZONE_SIDEBAR_ITEM_BASE) as usize;
            let places = app.sidebar_places();
            if place_idx < places.len() {
                let dest_dir = places[place_idx].path.clone();
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: None,
                });
            }
            return;
        }
    }

    // ── Drop on a folder in the content grid ────────────────────────
    let cr = content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;
    for i in 0..app.entries.len() {
        if i == drag_idx { continue; }
        if sources.iter().any(|s| s == &app.entries[i].path) { continue; }
        let ir = file_item_rect(i, cols, cr.x, base_y, s, zoom);
        if ir.contains(cx, cy) && app.entries[i].is_dir {
            let dest_dir = app.entries[i].path.clone();
            app.pending_drop = Some(PendingDrop {
                sources, dest_dir, reload_tab: None,
            });
            return;
        }
    }
}

fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) {
    let _ = std::fs::create_dir_all(dest);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let from = entry.path();
            let to = dest.join(entry.file_name());
            if from.is_dir() {
                copy_dir_recursive(&from, &to);
            } else {
                let _ = std::fs::copy(&from, &to);
            }
        }
    }
}

// Linux keycodes
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
const KEY_A: u32 = 30;
const KEY_C: u32 = 46;
const KEY_V: u32 = 47;
const KEY_X: u32 = 45;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;
const KEY_F2: u32 = 60;
const KEY_DELETE: u32 = 111;
const KEY_HOME: u32 = 102;
const KEY_END: u32 = 107;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;

/// Map an evdev keycode to a character for filename entry.
fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    // Number row: keycodes 2=1, 3=2, ..., 10=9, 11=0
    let ch = match key {
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift {
                b"!@#$%^&*()"[(key - 2) as usize]
            } else {
                base
            }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        // Letters (a=30..z)
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        // Punctuation
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ', // space
        _ => return None,
    };
    Some(ch as char)
}

fn handle_key(
    app: &mut App, _settings: &mut Settings,
    context_menu: &mut ContextMenu,
    popup_backend: &mut Option<WaylandPopupBackend<State>>,
    key: u32, ctrl: bool, shift: bool,
    running: &mut bool,
) {
    // Drop confirmation modal — ESC cancels
    if app.pending_drop.is_some() {
        if key == KEY_ESC {
            app.pending_drop = None;
        }
        return;
    }

    if context_menu.is_open() {
        if key == KEY_ESC {
            if let Some(backend) = popup_backend {
                context_menu.close_popups(backend);
            }
        }
        return;
    }

    // ── Search mode ──────────────────────────────────────────────────
    if app.searching {
        match key {
            KEY_ESC => app.close_search(),
            KEY_BACKSPACE => {
                if app.search_cursor > 0 {
                    app.search_cursor -= 1;
                    app.search_buf.remove(app.search_cursor);
                    app.run_search();
                }
            }
            KEY_LEFT => {
                if app.search_cursor > 0 { app.search_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.search_cursor < app.search_buf.len() { app.search_cursor += 1; }
            }
            KEY_HOME => app.search_cursor = 0,
            KEY_END => app.search_cursor = app.search_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.search_buf.insert(app.search_cursor, ch);
                    app.search_cursor += 1;
                    app.run_search();
                }
            }
        }
        return;
    }

    // ── Path bar editing ─────────────────────────────────────────────
    if app.path_editing {
        match key {
            KEY_ENTER => app.commit_path_edit(),
            KEY_ESC => app.cancel_path_edit(),
            KEY_BACKSPACE => {
                if app.path_cursor > 0 {
                    app.path_cursor -= 1;
                    app.path_buf.remove(app.path_cursor);
                }
            }
            KEY_DELETE => {
                if app.path_cursor < app.path_buf.len() {
                    app.path_buf.remove(app.path_cursor);
                }
            }
            KEY_LEFT => {
                if app.path_cursor > 0 { app.path_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.path_cursor < app.path_buf.len() { app.path_cursor += 1; }
            }
            KEY_HOME => app.path_cursor = 0,
            KEY_END => app.path_cursor = app.path_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.path_buf.insert(app.path_cursor, ch);
                    app.path_cursor += 1;
                }
            }
        }
        return;
    }

    // ── Rename mode ─────────────────────────────────────────────────
    if app.renaming.is_some() {
        match key {
            KEY_ENTER => app.commit_rename(),
            KEY_ESC => app.cancel_rename(),
            KEY_BACKSPACE => {
                if app.rename_cursor > 0 {
                    app.rename_cursor -= 1;
                    app.rename_buf.remove(app.rename_cursor);
                }
            }
            KEY_DELETE => {
                if app.rename_cursor < app.rename_buf.len() {
                    app.rename_buf.remove(app.rename_cursor);
                }
            }
            KEY_LEFT => {
                if app.rename_cursor > 0 { app.rename_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.rename_cursor < app.rename_buf.len() { app.rename_cursor += 1; }
            }
            KEY_HOME => app.rename_cursor = 0,
            KEY_END => app.rename_cursor = app.rename_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.rename_buf.insert(app.rename_cursor, ch);
                    app.rename_cursor += 1;
                }
            }
        }
        return;
    }

    // ── Pick mode: save name editing ───────────────────────────────
    if app.save_name_editing {
        match key {
            KEY_ENTER => {
                app.save_name_editing = false;
                app.confirm_pick();
                *running = false;
            }
            KEY_ESC => {
                app.save_name_editing = false;
            }
            KEY_BACKSPACE => {
                if app.save_name_cursor > 0 {
                    app.save_name_cursor -= 1;
                    app.save_name_buf.remove(app.save_name_cursor);
                }
            }
            KEY_DELETE => {
                if app.save_name_cursor < app.save_name_buf.len() {
                    app.save_name_buf.remove(app.save_name_cursor);
                }
            }
            KEY_LEFT => {
                if app.save_name_cursor > 0 { app.save_name_cursor -= 1; }
            }
            KEY_RIGHT => {
                if app.save_name_cursor < app.save_name_buf.len() { app.save_name_cursor += 1; }
            }
            KEY_HOME => app.save_name_cursor = 0,
            KEY_END => app.save_name_cursor = app.save_name_buf.len(),
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    app.save_name_buf.insert(app.save_name_cursor, ch);
                    app.save_name_cursor += 1;
                }
            }
        }
        return;
    }

    if ctrl {
        match key {
            KEY_A => app.select_all(),
            KEY_C => app.copy_selected(),
            KEY_X => app.cut_selected(),
            KEY_V => app.paste(),
            KEY_T if app.pick.is_none() => app.new_tab(),
            KEY_W if app.pick.is_none() => app.close_tab(app.current_tab),
            _ => {}
        }
    } else {
        match key {
            KEY_BACKSPACE => app.go_up(),
            KEY_ESC if app.pick.is_some() => {
                app.cancel_pick();
                *running = false;
            }
            KEY_ESC => app.clear_selection(),
            KEY_ENTER if app.pick.is_some() => {
                app.confirm_pick();
                *running = false;
            }
            KEY_F2 if app.pick.is_none() => {
                if let Some(idx) = app.entries.iter().position(|e| e.selected) {
                    app.start_rename(idx);
                }
            }
            KEY_DELETE if app.pick.is_none() => app.trash_selected(),
            _ => {}
        }
    }
}

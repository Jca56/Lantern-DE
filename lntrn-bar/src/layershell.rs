use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext, MenuEvent, MenuItem,
};
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

use crate::audio::{Audio, ZONE_AUDIO_ICON};
use crate::battery::{Battery, ZONE_BATTERY, ZONE_CHARGE_LIMIT_TOGGLE};
use crate::bluetooth::{Bluetooth, ZONE_BT_ICON};
use crate::clock::Clock;
use crate::svg_icon::IconCache;
use crate::temperature::{Temperature, ZONE_TEMP};
use crate::toplevel::ToplevelTracker;
use crate::wifi::{Wifi, ZONE_WIFI_ICON};

pub(crate) const BAR_HEIGHT_DEFAULT: u32 = 72;
const BAR_HEIGHT_MIN: u32 = 48;
const BAR_HEIGHT_MAX: u32 = 120;
const MENU_OVERFLOW: u32 = 700;
const FLOAT_GAP: f32 = 20.0;
/// Extra surface pixels beyond the bar for the drop-shadow to render into.
const SHADOW_PAD: u32 = 30;
const CORNER_RADIUS: f32 = 16.0;
/// Float/dock animation duration in seconds.
const ANIM_DURATION: f32 = 0.35;

const MENU_FLOAT_CHECKBOX: u32 = 1;
const MENU_HEIGHT_SLIDER: u32 = 2;
const MENU_OPEN_TERMINAL: u32 = 3;
const MENU_OPEN_FILES: u32 = 4;
const MENU_AUTOHIDE_CHECKBOX: u32 = 5;
const MENU_OPACITY_SLIDER: u32 = 6;
const MENU_MOVE_POSITION: u32 = 7;
const MENU_LAVA_LAMP: u32 = 7;
const MENU_TRAY_LEFT: u32 = 8;
const MENU_APP_PIN: u32 = 10;
const MENU_APP_LAUNCH: u32 = 11;
const MENU_APP_CLOSE: u32 = 12;
const MENU_PROC_PIN: u32 = 13;
const MENU_PROC_UNPIN: u32 = 14;
const MENU_PROC_KILL: u32 = 15;
/// Seconds to wait after pointer leaves before hiding.
const AUTOHIDE_DELAY: f32 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarStyle {
    Floating,
    Docked,
}

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

pub(crate) struct State {
    running: bool,
    configured: bool,
    frame_done: bool,
    input_dirty: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    seat: Option<wl_seat::WlSeat>,
    cursor_x: f64,
    cursor_y: f64,
    pointer_in_surface: bool,
    right_clicked: bool,
    left_pressed: bool,
    left_released: bool,
    scroll_delta: f32,
    // Keyboard state
    key_pressed: Option<u32>,
    held_key: Option<u32>,
    repeat_deadline: Instant,
    repeat_started: bool,
    ctrl: bool,
    shift: bool,
    pub(crate) tracker: ToplevelTracker,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true, input_dirty: false,
            width: 0, height: BAR_HEIGHT_DEFAULT + MENU_OVERFLOW + SHADOW_PAD,
            scale: 1, output_phys_width: 0,
            compositor: None, layer_shell: None, viewporter: None,
            surface: None, layer_surface: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            right_clicked: false, left_pressed: false, left_released: false, scroll_delta: 0.0,
            key_pressed: None, held_key: None, repeat_deadline: Instant::now(),
            repeat_started: false, ctrl: false, shift: false,
            tracker: ToplevelTracker::new(),
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

// ── Dispatch impls ───────────────────────────────────────────────────────────

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
                    state.seat = Some(registry.bind(name, version.min(9), qh, ()));
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

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
        state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => state.scale = factor,
            wl_output::Event::Mode { width, .. } => state.output_phys_width = width as u32,
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(state: &mut Self, _: &wl_callback::WlCallback, _: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        state.frame_done = true;
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self, layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer_surface.ack_configure(serial);
                if width > 0 { state.width = width; }
                if height > 0 { state.height = height; }
                state.configured = true;
                state.frame_done = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.running = false,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event,
        _: &(), _: &Connection, qh: &QueueHandle<Self>,
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

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
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
                if button == BTN_RIGHT && btn_state == pressed { state.right_clicked = true; }
                if button == BTN_LEFT && btn_state == pressed { state.left_pressed = true; }
                if button == BTN_LEFT && btn_state == released { state.left_released = true; }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                use wayland_client::WEnum;
                if axis == WEnum::Value(wl_pointer::Axis::VerticalScroll) {
                    state.scroll_delta += value as f32;
                }
            }
            _ => {}
        }
        state.input_dirty = true;
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self, _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                use wayland_client::WEnum;
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.key_pressed = Some(key);
                    state.held_key = Some(key);
                    state.repeat_started = false;
                    state.repeat_deadline = Instant::now() + Duration::from_millis(300);
                } else if key_state == WEnum::Value(wl_keyboard::KeyState::Released) {
                    if state.held_key == Some(key) {
                        state.held_key = None;
                    }
                }
                state.input_dirty = true;
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.ctrl = mods_depressed & 4 != 0;
                state.shift = mods_depressed & 1 != 0;
            }
            _ => {}
        }
    }
}

// ── Foreign toplevel dispatch ────────────────────────────────────────────────

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self, _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.tracker.on_new(toplevel);
            state.frame_done = true;
        }
    }

    wayland_client::event_created_child!(State, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        0 => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.tracker.on_app_id(handle, app_id);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                state.tracker.on_title(handle, title);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                state.tracker.on_state(handle, &bytes);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.tracker.on_done(handle);
                state.frame_done = true;
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.tracker.on_closed(handle);
                state.frame_done = true;
            }
            _ => {}
        }
    }
}

// ── Layer surface helpers ────────────────────────────────────────────────────

fn surface_height(bar_height: u32) -> u32 {
    bar_height + MENU_OVERFLOW + SHADOW_PAD
}

fn apply_layer_config(
    layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    width: u32, bar_height: u32, anim_t: f32, auto_hide: bool,
    position_top: bool,
) {
    use zwlr_layer_surface_v1::Anchor;
    let gap = (FLOAT_GAP * (1.0 - anim_t)).round() as i32;
    // Surface extends past the bar by SHADOW_PAD for the shadow to render into,
    // so subtract it from the margin to keep the bar's visual position unchanged.
    let shadow_pad = (SHADOW_PAD as f32 * (1.0 - anim_t)).round() as i32;
    let margin = if auto_hide { 0 } else { (gap - shadow_pad).max(0) };
    if position_top {
        layer_surface.set_anchor(Anchor::Top | Anchor::Left | Anchor::Right);
        layer_surface.set_margin(margin, 0, 0, 0);
    } else {
        layer_surface.set_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer_surface.set_margin(0, 0, margin, 0);
    }
    layer_surface.set_size(width, surface_height(bar_height));
    let zone = if auto_hide { 0 } else { bar_height as i32 + gap };
    layer_surface.set_exclusive_zone(zone);
}

fn set_bar_input_region(
    surface: &wl_surface::WlSurface,
    region: &wl_region::WlRegion,
    bar_height: u32,
    auto_hide: bool,
    hide_t: f32,
    anim_t: f32,
    position_top: bool,
) {
    region.subtract(0, 0, 100000, 100000);
    let pill_space = 40i32;
    if position_top {
        if auto_hide && hide_t > 0.5 {
            region.add(0, SHADOW_PAD as i32, 100000, 2);
        } else {
            // Bar is at top of surface; shadow pad is above, pills below
            region.add(0, SHADOW_PAD as i32, 100000, bar_height as i32 + pill_space);
        }
    } else {
        if auto_hide && hide_t > 0.5 {
            let surface_h = (MENU_OVERFLOW + bar_height + SHADOW_PAD) as i32;
            region.add(0, surface_h - 2, 100000, 2);
        } else {
            // Bar renders at MENU_OVERFLOW + SHADOW_PAD - float_gap from top.
            // When docked (anim_t=1): bar_y = MENU_OVERFLOW + SHADOW_PAD
            // When floating (anim_t=0): bar_y = MENU_OVERFLOW + SHADOW_PAD - FLOAT_GAP
            let float_gap = (FLOAT_GAP * (1.0 - anim_t)).round() as i32;
            let bar_y = MENU_OVERFLOW as i32 + SHADOW_PAD as i32 - float_gap;
            region.add(0, bar_y - pill_space, 100000, bar_height as i32 + pill_space);
        }
    }
    surface.set_input_region(Some(region));
}

fn height_to_slider(h: u32) -> f32 {
    ((h - BAR_HEIGHT_MIN) as f32 / (BAR_HEIGHT_MAX - BAR_HEIGHT_MIN) as f32).clamp(0.0, 1.0)
}

fn slider_to_height(v: f32) -> u32 {
    let range = (BAR_HEIGHT_MAX - BAR_HEIGHT_MIN) as f32;
    BAR_HEIGHT_MIN + (range * v.clamp(0.0, 1.0)).round() as u32
}

fn save_settings(
    style: BarStyle, auto_hide: bool, height: u32, opacity: f32,
    lava_lamp: bool, position_top: bool, tray_left: bool,
    pinned: &[crate::appmenu::sysmon::PinnedProcess],
) {
    let s = crate::bar_settings::BarSettings {
        floating: style == BarStyle::Floating,
        auto_hide, height, opacity, lava_lamp, position_top, tray_left,
        pinned_procs: pinned.iter().map(|p| p.name.clone()).collect(),
    };
    s.save();
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let layer_shell = state.layer_shell.as_ref()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let surface = compositor.create_surface(&qh, ());
    let input_region = compositor.create_region(&qh, ());

    let saved = crate::bar_settings::BarSettings::load();
    let mut user_style = if saved.floating { BarStyle::Floating } else { BarStyle::Docked };
    let mut bar_height = saved.height.clamp(BAR_HEIGHT_MIN, BAR_HEIGHT_MAX);
    // 0.0 = floating, 1.0 = docked
    let mut anim_t: f32 = if saved.floating { 0.0 } else { 1.0 };
    // Auto-hide state
    let mut auto_hide = saved.auto_hide;
    let mut position_top = saved.position_top;
    let mut tray_left = saved.tray_left;
    let mut bar_opacity: f32 = saved.opacity.clamp(0.0, 1.0);
    let mut lava = crate::lava::LavaLamp::new();
    lava.enabled = saved.lava_lamp;
    // 0.0 = fully visible, 1.0 = fully hidden
    let mut hide_t: f32 = 0.0;
    let mut hide_timer: f32 = 0.0;

    let layer_surface = layer_shell.get_layer_surface(
        &surface, None, zwlr_layer_shell_v1::Layer::Top,
        "lntrn-bar".to_string(), &qh, (),
    );
    apply_layer_config(&layer_surface, 0, bar_height, anim_t, auto_hide, position_top);
    set_bar_input_region(&surface, &input_region, bar_height, auto_hide, 0.0, anim_t, position_top);
    surface.commit();

    state.surface = Some(surface.clone());
    state.layer_surface = Some(layer_surface.clone());

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    if state.width == 0 {
        return Err(anyhow!("compositor sent zero-width configure"));
    }
    event_queue.roundtrip(&mut state)?;

    tracing::info!(logical_w = state.width, bar_h = bar_height, "bar configured");

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

    apply_layer_config(&layer_surface, state.width, bar_height, anim_t, auto_hide, position_top);
    set_bar_input_region(&surface, &input_region, bar_height, auto_hide, 0.0, anim_t, position_top);
    surface.commit();

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

    let palette = FoxPalette::dark();
    let menu_style = ContextMenuStyle::from_palette(&palette);
    let mut context_menu = ContextMenu::new(menu_style);
    context_menu.set_scale(state.fractional_scale() as f32);
    let mut last_frame = Instant::now();

    let tex_pass = TexturePass::new(&gpu);
    let mut system_tray = crate::tray::SystemTray::start();
    let mut pending_tray_menu_pos: Option<(f32, f32)> = None;
    let mut tray_menu_bus: Option<String> = None;
    let mut tray_menu_path: Option<String> = None;
    let mut pending_dbusmenu_click: Option<i32> = None;
    let mut tray_menu_just_opened: bool = false;

    let mut icon_cache = IconCache::new();
    let mut battery = Battery::new();
    if battery.is_some() {
        tracing::info!("battery widget active");
    }
    let mut temperature = Temperature::new();
    tracing::info!("temperature widget active");
    let mut wifi = Wifi::new();
    let mut bluetooth = Bluetooth::new();
    let mut audio = Audio::new();
    tracing::info!("wifi + bluetooth + audio widgets active");

    let mut ix = InteractionContext::new();
    let mut clock = Clock::new();
    let mut menu_was_open = false;

    let mut kb_was_active = false;
    let mut app_tray = crate::apptray::AppTray::new();
    let mut app_menu = crate::appmenu::AppMenu::new();
    // Restore pinned sysmon processes from saved settings
    for name in &saved.pinned_procs {
        app_menu.sysmon.pinned.push(crate::appmenu::sysmon::PinnedProcess {
            name: name.clone(), pid: 0, cpu_pct: 0.0, mem_kb: 0,
            cpu_history: [0.0; crate::appmenu::sysmon::SPARKLINE_LEN],
            history_idx: 0, warning_phase: 0.0,
        });
    }
    // The app_id of the app whose context menu is currently open (if any).
    let mut ctx_menu_app: Option<String> = None;
    let mut hover_client = crate::hover::HoverClient::new();
    let mut hover_pending: Option<(String, f32, f32, f32)> = None; // (app_id, icon_x, icon_w, bar_h)
    let mut hover_debounce = Instant::now();
    // Track widget positions for popup alignment
    let mut bat_draw_x: f32 = 0.0;
    let mut bat_draw_w: f32 = 0.0;
    let mut temp_draw_x: f32 = 0.0;
    let mut temp_draw_w: f32 = 0.0;
    let mut wifi_draw_x: f32 = 0.0;
    let mut wifi_draw_w: f32 = 0.0;
    let mut bt_draw_x: f32 = 0.0;
    let mut bt_draw_w: f32 = 0.0;
    let mut audio_draw_x: f32 = 0.0;
    let mut audio_draw_w: f32 = 0.0;

    tracing::info!("bar ready, entering render loop");

    let mut needs_anim = false;
    while state.running {
        if needs_anim {
            // Non-blocking: process any pending events, then continue for animation
            event_queue.flush()?;
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            event_queue.dispatch_pending(&mut state)?;
            std::thread::sleep(Duration::from_millis(16));
            state.frame_done = true;
        } else {
            // Poll the Wayland fd with a 1-second timeout so we wake up
            // periodically to check if any widget has new data to display.
            event_queue.flush()?;
            let Some(guard) = event_queue.prepare_read() else {
                event_queue.dispatch_pending(&mut state)?;
                continue;
            };
            let fd = guard.connection_fd().as_raw_fd();
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let ret = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if ret > 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
            event_queue.dispatch_pending(&mut state)?;

            if state.input_dirty || state.frame_done {
                // Input or Wayland event arrived — throttle and render.
                if !state.frame_done {
                    std::thread::sleep(Duration::from_millis(16));
                    event_queue.dispatch_pending(&mut state)?;
                }
                state.input_dirty = false;
                state.frame_done = true;
            } else {
                // Timeout with no Wayland events — tick widgets to check for
                // new data (clock minute change, temperature poll, mpsc events).
                let mut dirty = clock.tick()
                    | temperature.tick()
                    | wifi.tick()
                    | bluetooth.tick()
                    | audio.tick()
                    | app_menu.clipboard.tick()
                    | battery.as_mut().map_or(false, |b| b.tick())
                    | system_tray.as_mut().map_or(false, |t| t.tick());
                if app_menu.is_open() || !app_menu.sysmon.pinned.is_empty() {
                    dirty |= app_menu.sysmon.tick();
                }
                state.frame_done = dirty;
            }
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32().min(0.05);
        last_frame = now;

        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            context_menu.set_scale(scale_f);
        }

        // ── Animation ────────────────────────────────────────────────
        let should_dock = user_style == BarStyle::Docked
            || (user_style == BarStyle::Floating
                && (state.tracker.any_maximized() || state.tracker.any_fullscreen()));

        let target = if should_dock { 1.0f32 } else { 0.0 };
        let animating = (anim_t - target).abs() > 0.001;
        let anim_step = dt / ANIM_DURATION;
        let prev_anim_t = anim_t;
        if anim_t < target {
            anim_t = (anim_t + anim_step).min(1.0);
        } else if anim_t > target {
            anim_t = (anim_t - anim_step).max(0.0);
        }
        // Update input region when float/dock animation changes bar position
        if !position_top && (prev_anim_t - anim_t).abs() > 0.001 {
            set_bar_input_region(&surface, &input_region, bar_height, auto_hide, hide_t, anim_t, position_top);
        }

        // ── Auto-hide animation ──────────────────────────────────────
        let cursor_phys_y = (state.cursor_y as f32) * scale_f;
        let surface_bottom = state.phys_height().max(1) as f32;
        let pointer_on_bar = if auto_hide && hide_t > 0.5 {
            // When hidden, only reveal at the very bottom edge (last 4px)
            state.pointer_in_surface && cursor_phys_y >= surface_bottom - 4.0
        } else {
            // When visible, count as on-bar if in the actual bar area
            let base_bar_top = surface_bottom - (bar_height as f32 * scale_f);
            state.pointer_in_surface && cursor_phys_y >= base_bar_top
        };
        let bat_popup_open = battery.as_ref().map_or(false, |b| b.open);
        let temp_popup_open = temperature.open;
        let hide_target: f32 = if auto_hide && !pointer_on_bar && !context_menu.is_open() && !app_menu.is_open() && !bat_popup_open && !temp_popup_open {
            hide_timer += dt;
            if hide_timer >= AUTOHIDE_DELAY { 1.0 } else { 0.0 }
        } else {
            hide_timer = 0.0;
            0.0
        };
        let hide_animating = (hide_t - hide_target).abs() > 0.001;
        let hide_step = dt / ANIM_DURATION;
        let prev_hide_t = hide_t;
        if hide_t < hide_target {
            hide_t = (hide_t + hide_step).min(1.0);
        } else if hide_t > hide_target {
            hide_t = (hide_t - hide_step).max(0.0);
        }

        // Update input region when crossing the hidden/visible threshold
        if auto_hide && !menu_was_open
            && ((prev_hide_t <= 0.5 && hide_t > 0.5) || (prev_hide_t > 0.5 && hide_t <= 0.5))
        {
            set_bar_input_region(&surface, &input_region, bar_height, auto_hide, hide_t, anim_t, position_top);
            surface.commit();
        }

        // Tick app tray slide animations
        let tray_animating = app_tray.update_anim(dt);
        let bat_animating = battery.as_mut().map_or(false, |b| b.update_anim(dt));
        let bt_animating = bluetooth.update_anim(dt);

        // Keep rendering during animation
        if animating || hide_animating || tray_animating || bat_animating || bt_animating || app_tray.is_dragging() || audio.is_dragging() {
            state.frame_done = true;
        }

        let gap = FLOAT_GAP * (1.0 - anim_t);
        let gap_phys = gap * scale_f;
        let radius = CORNER_RADIUS * (1.0 - anim_t) * scale_f;

        let phys_w = state.phys_width().max(1);
        let total_phys_h = state.phys_height().max(1);
        let bar_phys_h = (bar_height as f32 * scale_f).round() as u32;
        let phys_w_f = phys_w as f32;
        let bar_h_f = bar_phys_h as f32;
        // Slide the bar visually when hiding.
        // Shadow pad in physical pixels — shrinks to 0 when docked.
        let shadow_pad_phys = SHADOW_PAD as f32 * scale_f * (1.0 - anim_t);
        // The layer-shell margin is (gap - shadow_pad), clamped to 0.
        // When shadow_pad > gap the surface sits flush against the edge,
        // so only gap_phys worth of inset is needed — not the full shadow_pad.
        let edge_inset = gap_phys.min(shadow_pad_phys);
        // When auto_hide, margin is always 0 so we draw the float gap visually.
        let visual_gap = if auto_hide { gap_phys } else { edge_inset };
        let bar_y_offset = if position_top {
            let hide_slide = hide_t * (bar_h_f + visual_gap);
            visual_gap - hide_slide
        } else {
            let hide_slide = hide_t * (bar_h_f + visual_gap);
            (total_phys_h - bar_phys_h) as f32 - visual_gap + hide_slide
        };

        clock.tick();

        let phys_cx = (state.cursor_x as f32) * scale_f;
        let phys_cy = (state.cursor_y as f32) * scale_f;

        // ── Menu resize drag ───────────────────────────────────────────
        if app_menu.is_dragging() {
            app_menu.update_resize(phys_cx, phys_cy, scale_f);
        }

        // ── Input handling ───────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            // Check for menu resize edge drag before other input
            if let Some(edge) = app_menu.resize_edge_at(phys_cx, phys_cy, scale_f) {
                app_menu.start_resize(edge, phys_cx, phys_cy);
            }
            app_menu.on_left_click(phys_cx, phys_cy, &ix, scale_f);
            ix.on_left_pressed();
            // Start potential drag on app tray icons
            let toplevels = state.tracker.toplevels();
            app_tray.on_press(&ix, phys_cx, phys_cy, &toplevels);
            // Check launcher button click
            if ix.active_zone_id() == Some(0xBB_FFFF) {
                app_menu.toggle();
            }
            // Check app tray clicks — activate if running, launch if pinned but not running
            let toplevels = state.tracker.toplevels();
            if let Some(clicked_app_id) = app_tray.handle_click(&ix, phys_cx, phys_cy, &toplevels) {
                let is_running = toplevels.iter().any(|t| t.app_id == clicked_app_id);
                if is_running {
                    if let Some(seat) = &state.seat {
                        state.tracker.activate(&clicked_app_id, seat);
                    }
                } else {
                    // Pinned but not running — launch it
                    let apps = crate::desktop::scan_apps();
                    if let Some(entry) = apps.iter().find(|e| e.app_id == clicked_app_id) {
                        crate::appmenu::launch_app(&entry.exec);
                    } else {
                        crate::appmenu::launch_app(&clicked_app_id);
                    }
                }
            }
            // Check system tray icon clicks
            if let Some(tray) = &system_tray {
                if tray.handle_click(&ix, phys_cx, phys_cy) {
                    // Menu fetch in progress — store click position
                    pending_tray_menu_pos = Some((phys_cx, bar_y_offset));
                }
            }
            // Check clock / calendar clicks
            if !clock.handle_click(&ix, phys_cx, phys_cy) && clock.open {
                // Clicked outside calendar — close it
                clock.open = false;
            }
            // Check temperature click → toggle popup
            if ix.active_zone_id() == Some(ZONE_TEMP) {
                temperature.open = !temperature.open;
            } else if temperature.open {
                let in_popup = temperature.popup_rect(temp_draw_x, temp_draw_w, bar_y_offset, bar_h_f, position_top, scale_f, phys_w)
                    .map_or(false, |r| r.contains(phys_cx, phys_cy));
                if !in_popup {
                    temperature.open = false;
                }
            }
            // Check battery click → toggle popup
            if let Some(bat) = &mut battery {
                if ix.active_zone_id() == Some(ZONE_BATTERY) {
                    bat.open = !bat.open;
                } else if ix.active_zone_id() == Some(ZONE_CHARGE_LIMIT_TOGGLE) {
                    bat.toggle_charge_limit();
                } else if bat.open {
                    let in_popup = bat.popup_rect(bat_draw_x, bat_draw_w, bar_y_offset, bar_h_f, position_top, scale_f, phys_w)
                        .map_or(false, |r| r.contains(phys_cx, phys_cy));
                    if !in_popup {
                        bat.open = false;
                    }
                }
            }
            // Check wifi click → toggle popup or handle network click
            if ix.active_zone_id() == Some(ZONE_WIFI_ICON) {
                wifi.open = !wifi.open;
                if wifi.open {
                    wifi.request_scan();
                }
            } else if wifi.open {
                let in_popup = wifi.popup_rect(wifi_draw_x, wifi_draw_w, bar_y_offset, bar_h_f, position_top, scale_f, phys_w)
                    .map_or(false, |r| r.contains(phys_cx, phys_cy));
                if in_popup {
                    wifi.handle_network_click(&ix, phys_cx, phys_cy);
                } else {
                    wifi.open = false;
                    wifi.password_focused = false;
                }
            }
            // Check bluetooth click → toggle popup or handle device click
            if ix.active_zone_id() == Some(ZONE_BT_ICON) {
                bluetooth.open = !bluetooth.open;
            } else if bluetooth.open {
                let in_popup = bluetooth.popup_rect(bt_draw_x, bt_draw_w, bar_y_offset, bar_h_f, position_top, scale_f, phys_w)
                    .map_or(false, |r| r.contains(phys_cx, phys_cy));
                if in_popup {
                    bluetooth.handle_click(&ix, phys_cx, phys_cy);
                } else {
                    bluetooth.open = false;
                }
            }
            // Check audio click → toggle popup or handle slider/mute/sink click
            if ix.active_zone_id() == Some(ZONE_AUDIO_ICON) {
                audio.open = !audio.open;
            } else if audio.open {
                let in_popup = audio.popup_rect(audio_draw_x, audio_draw_w, bar_y_offset, bar_h_f, position_top, scale_f, phys_w)
                    .map_or(false, |r| r.contains(phys_cx, phys_cy));
                if in_popup {
                    audio.handle_click(&ix, phys_cx, phys_cy);
                } else {
                    audio.open = false;
                }
            }
        }
        // Audio slider drag — every frame while held
        if audio.is_dragging() {
            audio.handle_drag(phys_cx);
            state.frame_done = true;
        }

        if state.left_released {
            state.left_released = false;
            audio.on_release();
            // End menu resize if active
            app_menu.end_resize();
            // Finish drag-to-reorder before other release handling
            let toplevels = state.tracker.toplevels();
            let was_drag = app_tray.on_release(&toplevels);
            if !was_drag {
                if context_menu.is_open() && !context_menu.contains(phys_cx, phys_cy)
                    && !tray_menu_just_opened
                {
                    context_menu.close();
                }
                tray_menu_just_opened = false;
                if app_menu.is_open() && !app_menu.contains(phys_cx, phys_cy)
                    && ix.active_zone_id() != Some(0xBB_FFFF)
                {
                    app_menu.close();
                }
            }
            ix.on_left_released();
        }
        if state.right_clicked {
            state.right_clicked = false;
            // Right-click in app menu → app or process context menu
            if app_menu.is_open() && app_menu.contains(phys_cx, phys_cy) {
                // Check if right-click hit a sysmon process row first
                if app_menu.sysmon.on_right_click(&ix, phys_cx, phys_cy) {
                    let is_pinned = if let Some((ref name, _)) = app_menu.sysmon.right_clicked_proc {
                        app_menu.sysmon.pinned.iter().any(|p| p.name == *name)
                    } else { false };
                    let label = if is_pinned { "Unpin from Bar" } else { "Pin to Bar" };
                    let action = if is_pinned { MENU_PROC_UNPIN } else { MENU_PROC_PIN };
                    context_menu.open(phys_cx, phys_cy, vec![
                        MenuItem::action(action, label),
                        MenuItem::separator(),
                        MenuItem::action_danger(MENU_PROC_KILL, "Kill Process"),
                    ]);
                } else {
                    app_menu.on_right_click(phys_cx, phys_cy, &ix);
                }
            } else {
                // ── Unified bar right-click menu ──
                // Build context-specific items, then append common bar settings.
                let mut items: Vec<MenuItem> = Vec::new();
                ctx_menu_app = None;

                // Check pinned sysmon pill
                let mut handled = false;
                if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
                    use crate::appmenu::sysmon::ZONE_PINNED_BASE;
                    if zone >= ZONE_PINNED_BASE && zone < ZONE_PINNED_BASE + 64 {
                        let idx = (zone - ZONE_PINNED_BASE) as usize;
                        if let Some(pinned) = app_menu.sysmon.pinned.get(idx) {
                            app_menu.sysmon.right_clicked_proc = Some((pinned.name.clone(), pinned.pid));
                            items.push(MenuItem::action(MENU_PROC_UNPIN, "Unpin from Bar"));
                            items.push(MenuItem::action(MENU_PROC_KILL, "Kill Process"));
                            handled = true;
                        }
                    }
                }

                // Check app tray icon
                if !handled {
                    let toplevels = state.tracker.toplevels();
                    if let Some((app_id, pinned, running)) =
                        app_tray.handle_right_click(&ix, phys_cx, phys_cy, &toplevels)
                    {
                        let pin_label = if pinned { "Unpin" } else { "Pin to Bar" };
                        items.push(MenuItem::action(MENU_APP_PIN, pin_label));
                        items.push(MenuItem::action(MENU_APP_LAUNCH, "New Window"));
                        if running {
                            items.push(MenuItem::separator());
                            items.push(MenuItem::action(MENU_APP_CLOSE, "Close"));
                        }
                        ctx_menu_app = Some(app_id);
                        handled = true;
                    }
                }

                // Colored separator between context items and common items
                if handled {
                    items.push(MenuItem::colored_separator(
                        lntrn_render::Color::from_rgb8(234, 179, 8).with_alpha(0.5),
                    ));
                }

                // Common bar items (always present)
                let is_floating = user_style == BarStyle::Floating;
                items.push(MenuItem::action(MENU_OPEN_TERMINAL, "Open Terminal"));
                items.push(MenuItem::action(MENU_OPEN_FILES, "Open File Manager"));
                items.push(MenuItem::separator());
                let pos_label = if position_top { "Move to Bottom" } else { "Move to Top" };
                items.push(MenuItem::action(MENU_MOVE_POSITION, pos_label));
                items.push(MenuItem::checkbox(MENU_FLOAT_CHECKBOX, "Float", is_floating));
                items.push(MenuItem::checkbox(MENU_AUTOHIDE_CHECKBOX, "Auto Hide", auto_hide));
                items.push(MenuItem::checkbox(MENU_LAVA_LAMP, "Lava Lamp", lava.enabled));
                items.push(MenuItem::checkbox(MENU_TRAY_LEFT, "App Tray Left", tray_left));
                items.push(MenuItem::slider(MENU_HEIGHT_SLIDER, "Bar Height", height_to_slider(bar_height)));
                items.push(MenuItem::slider(MENU_OPACITY_SLIDER, "Opacity", bar_opacity));

                context_menu.open(phys_cx, bar_y_offset, items);
                if !position_top { context_menu.clamp_bottom_bar(phys_w_f, total_phys_h as f32); }
                surface.set_input_region(None);
                menu_was_open = true;
            }
        }
        if state.scroll_delta != 0.0 {
            if wifi.open {
                wifi.on_scroll(state.scroll_delta);
            } else if bluetooth.open {
                bluetooth.on_scroll(state.scroll_delta);
            } else if audio.open {
                audio.on_scroll(state.scroll_delta);
            } else {
                app_menu.on_scroll(state.scroll_delta * scale_f);
            }
            state.scroll_delta = 0.0;
        }

        // Keyboard input
        if let Some(key) = state.key_pressed.take() {
            if wifi.wants_keyboard() {
                wifi.on_key(key, state.shift);
                if key == 1 && !wifi.password_focused {
                    wifi.open = false;
                }
            } else if app_menu.wants_keyboard() {
                app_menu.on_key(key, state.shift);
            } else if key == 1 {
                // Esc closes any open popup
                if wifi.open { wifi.open = false; }
                if bluetooth.open { bluetooth.open = false; }
                if audio.open { audio.open = false; }
                if temperature.open { temperature.open = false; }
            }
        }
        // Key repeat for held keys
        if let Some(held) = state.held_key {
            let wants = wifi.wants_keyboard() || app_menu.wants_keyboard();
            if wants && state.repeat_deadline <= Instant::now() {
                if wifi.wants_keyboard() {
                    wifi.on_key(held, state.shift);
                } else if app_menu.wants_keyboard() {
                    app_menu.on_key(held, state.shift);
                }
                let interval = if state.repeat_started { 30 } else { 300 };
                state.repeat_deadline = Instant::now() + Duration::from_millis(interval);
                state.repeat_started = true;
                state.frame_done = true;
            }
        }

        ix.begin_frame();
        if state.pointer_in_surface {
            ix.on_cursor_moved(phys_cx, phys_cy);
        } else {
            ix.on_cursor_left();
        }

        // Input region: full surface when any menu open, bar-only otherwise
        let bat_open = battery.as_ref().map_or(false, |b| b.open);
        let any_menu_open = context_menu.is_open() || app_menu.is_open() || clock.open || bat_open || temperature.open || wifi.open || bluetooth.open || audio.open;
        if any_menu_open && !menu_was_open {
            surface.set_input_region(None);
        } else if !any_menu_open && menu_was_open {
            set_bar_input_region(&surface, &input_region, bar_height, auto_hide, hide_t, anim_t, position_top);
            surface.commit();
        }
        // Keyboard focus: grab when app menu wants typing, release otherwise
        let wants_kb = app_menu.wants_keyboard() || wifi.wants_keyboard();
        if wants_kb && !kb_was_active {
            layer_surface.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
            surface.commit();
        } else if !wants_kb && kb_was_active {
            layer_surface.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
            // Clear held key — we won't receive the release event after losing keyboard
            state.held_key = None;
            surface.commit();
        }
        kb_was_active = wants_kb;
        menu_was_open = any_menu_open;

        // ── Draw ─────────────────────────────────────────────────────
        painter.clear();

        // Visual bar rect with animated gap
        let vis_x = gap_phys;
        let vis_y = bar_y_offset;
        let vis_w = phys_w_f - gap_phys * 2.0;

        // Drag motion — feed cursor to app tray every frame while dragging
        {
            let icon_sz = (bar_h_f * 0.75).max(36.0);
            let gap_icon = crate::apptray::ICON_GAP * scale_f;
            let toplevels = state.tracker.toplevels();
            if app_tray.on_motion(phys_cx, &toplevels, icon_sz, gap_icon, vis_x, vis_w) {
                state.frame_done = true;
            }
        }
        let vis_h = bar_h_f;

        // ── Bar background ────────────────────────────────────────
        lava.update(dt);
        let (bar_rect, bar_r) = if gap_phys < 0.5 {
            (Rect::new(0.0, bar_y_offset, phys_w_f, bar_h_f), 0.0)
        } else {
            (Rect::new(vis_x, vis_y, vis_w, vis_h), radius)
        };

        // Drop shadow — all around when floating, only inner edge when docked
        let is_floating = gap_phys > 0.5;
        if is_floating {
            // Floating: shadow on all sides, no offset
            let shadow_sigma = 10.0 * scale_f;
            painter.shadow(bar_rect, bar_r, shadow_sigma,
                Color::BLACK.with_alpha(0.50 * bar_opacity), 0.0, 0.0);
        } else {
            // Docked: shadow only on inner edge (toward center of screen)
            let shadow_sigma = 12.0 * scale_f;
            let offset_y = if position_top { 3.0 * scale_f } else { -3.0 * scale_f };
            painter.shadow(bar_rect, bar_r, shadow_sigma,
                Color::BLACK.with_alpha(0.55 * bar_opacity), 0.0, offset_y);
        }

        if lava.enabled {
            lava.draw_background(&mut painter, bar_rect.x, bar_rect.y, bar_rect.w, bar_rect.h, bar_r, bar_opacity);
            lava.draw_blobs(&mut painter, bar_rect.x, bar_rect.y, bar_rect.w, bar_rect.h, 1.0);
        } else {
            let bar_bg = palette.bg.with_alpha(bar_opacity);
            painter.rect_filled(bar_rect, bar_r, bar_bg);
        }

        // Inset edge: uniform black on all four sides
        let bevel_sigma = 3.5 * scale_f;
        painter.inner_shadow(bar_rect, bar_r, bevel_sigma,
            Color::BLACK.with_alpha(0.40 * bar_opacity), 0.0, 0.0);

        // ── Left: launcher button ─────────────────────────────────
        let launcher_w = app_menu.draw_button(
            &mut painter, &mut ix, &palette,
            vis_x, vis_y, vis_h, scale_f,
        );

        // ── Left-side widgets (app tray left mode swaps order) ───
        let left_after_launcher = vis_x + launcher_w + 8.0 * scale_f;
        let tray_left_x;
        if tray_left {
            // Compute app tray width to position pinned processes after it
            let toplevels = state.tracker.toplevels();
            let tray_w = app_tray.measure_width(&toplevels, vis_h, scale_f);
            tray_left_x = Some(left_after_launcher);
            let pinned_x = left_after_launcher + tray_w + 8.0 * scale_f;
            let _pinned_w = app_menu.sysmon.draw_pinned(
                &mut painter, &mut text, &mut ix, &palette,
                pinned_x, vis_y, vis_h, scale_f,
                phys_w, total_phys_h,
            );
        } else {
            tray_left_x = None;
            let pinned_x = left_after_launcher;
            let _pinned_w = app_menu.sysmon.draw_pinned(
                &mut painter, &mut text, &mut ix, &palette,
                pinned_x, vis_y, vis_h, scale_f,
                phys_w, total_phys_h,
            );
        }

        // ── Right-aligned widgets: [tray] [battery] [clock] ──────
        // We lay out right-to-left, accumulating consumed width.
        let mut right_used = 0.0f32;
        let widget_gap = 6.0 * scale_f;

        // Clock (rightmost)
        let font_size = vis_h * 0.78;
        let clock_padding = font_size * 0.35;
        let clock_char_w = font_size * 0.52;
        let clock_text = clock.time_text_len();
        let clock_w = clock_text as f32 * clock_char_w + clock_padding;
        clock.draw(
            &mut text, &mut ix, font_size, palette.text,
            vis_w, vis_h, vis_x, vis_y,
            phys_w, total_phys_h,
        );
        right_used += clock_w + 10.0 * scale_f; // extra gap before system tray

        // Load icons & tick widgets (mutable icon_cache borrows here)
        {
            let toplevels = state.tracker.toplevels();
            let app_icon_sz = (vis_h * 0.75).max(36.0) as u32;
            app_tray.load_icons(&toplevels, &mut icon_cache, &tex_pass, &gpu, app_icon_sz);
        }
        if let Some(bat) = &mut battery {
            bat.tick();
            let pad = 5.0 * scale_f;
            let usable = vis_h - pad * 2.0;
            let font_sz = (usable * 0.35).max(14.0);
            let ih = usable - font_sz - 5.0 * scale_f;
            let iw = ih * 1.5;
            bat.load_icons(&mut icon_cache, &tex_pass, &gpu, iw as u32, ih as u32);
        }
        if app_menu.is_open() || !app_menu.sysmon.pinned.is_empty() {
            app_menu.sysmon.tick();
        }
        app_menu.clipboard.tick();
        temperature.tick();
        {
            let pad = 5.0 * scale_f;
            let usable = vis_h - pad * 2.0;
            let icon_sz = (usable * 0.85) as u32;
            temperature.load_icons(&mut icon_cache, &tex_pass, &gpu, icon_sz);
        }
        wifi.tick();
        {
            let wifi_sz = (vis_h - 10.0 * scale_f).max(16.0) as u32;
            wifi.load_icons(&mut icon_cache, &tex_pass, &gpu, wifi_sz);
        }
        bluetooth.tick();
        {
            let bt_sz = (vis_h - 10.0 * scale_f).max(16.0) as u32;
            bluetooth.load_icons(&mut icon_cache, &tex_pass, &gpu, bt_sz);
        }
        audio.tick();
        {
            let aud_sz = (vis_h - 10.0 * scale_f).max(16.0) as u32;
            audio.load_icons(&mut icon_cache, &tex_pass, &gpu, aud_sz);
        }
        app_menu.load_icons(&mut icon_cache, &tex_pass, &gpu, scale_f);

        // Temperature (left of clock, right of battery)
        let temp_tex_draws;
        {
            let tw = temperature.measure(vis_h, scale_f);
            let tx = vis_x + vis_w - right_used - widget_gap - tw;
            temp_draw_x = tx;
            temp_draw_w = tw;
            let (_, draws) = temperature.draw(
                &mut painter, &mut text, &mut ix, &icon_cache, &palette,
                tx, vis_y, vis_h, scale_f, phys_w, total_phys_h,
            );
            temp_tex_draws = draws;
            right_used += tw + widget_gap;
        }

        // Battery (left of temperature) — tighter gap
        let mut battery_tex_draws = Vec::new();
        if let Some(bat) = &mut battery {
            let temp_bat_gap = 2.0 * scale_f;
            let bw = bat.measure(vis_h, scale_f);
            let bx = vis_x + vis_w - right_used - temp_bat_gap - bw;
            bat_draw_x = bx;
            bat_draw_w = bw;
            let (_, draws) = bat.draw(
                &mut painter, &mut text, &mut ix, &icon_cache, &palette,
                bx, vis_y, vis_h, scale_f, phys_w, total_phys_h,
            );
            battery_tex_draws = draws;
            right_used += bw + temp_bat_gap;
        }

        // WiFi (left of battery) — extra gap to separate from battery
        let wifi_tex_draws;
        {
            let bat_wifi_gap = widget_gap;
            let ww = wifi.measure(vis_h, scale_f);
            let wx = vis_x + vis_w - right_used - bat_wifi_gap - ww;
            wifi_draw_x = wx;
            wifi_draw_w = ww;
            let (_, draws) = wifi.draw(
                &mut painter, &mut text, &mut ix, &icon_cache, &palette,
                wx, vis_y, vis_h, scale_f, phys_w, total_phys_h,
            );
            wifi_tex_draws = draws;
            right_used += ww + bat_wifi_gap;
        }

        // Bluetooth (left of wifi)
        let bt_tex_draws;
        {
            let bw = bluetooth.measure(vis_h, scale_f);
            let btx = vis_x + vis_w - right_used - bw;
            bt_draw_x = btx;
            bt_draw_w = bw;
            let (_, draws) = bluetooth.draw(
                &mut painter, &mut text, &mut ix, &icon_cache, &palette,
                btx, vis_y, vis_h, scale_f, phys_w, total_phys_h,
            );
            bt_tex_draws = draws;
            right_used += bw;
        }

        // Audio (left of bluetooth)
        let audio_tex_draws;
        {
            let aw = audio.measure(vis_h, scale_f);
            let aud_x = vis_x + vis_w - right_used - aw;
            audio_draw_x = aud_x;
            audio_draw_w = aw;
            let (_, draws) = audio.draw(
                &mut painter, &mut text, &mut ix, &icon_cache, &palette,
                aud_x, vis_y, vis_h, scale_f, phys_w, total_phys_h,
            );
            audio_tex_draws = draws;
            right_used += aw;
        }

        // System tray (left of audio)
        let mut tray_tex_draws = Vec::new();
        if let Some(tray) = &mut system_tray {
            tray.poll(&tex_pass, &gpu);
            // Check for dbusmenu ready events
            if let Some(event) = tray.poll_event() {
                match event {
                    crate::tray::TrayEvent::MenuReady { bus_name, menu_path, items } => {
                        if let Some((mx, my)) = pending_tray_menu_pos.take() {
                            tray_menu_bus = Some(bus_name);
                            tray_menu_path = Some(menu_path);
                            context_menu.open(mx, my, items);
                            if !position_top { context_menu.clamp_bottom_bar(phys_w_f, total_phys_h as f32); }
                            tray_menu_just_opened = true;
                        }
                    }
                }
            }
            let (tw, draws) = tray.draw(
                &mut painter, &mut text, &mut ix, &palette,
                vis_x, vis_y, vis_w, vis_h,
                right_used,
                scale_f, phys_w, total_phys_h,
            );
            tray_tex_draws = draws;
            let _ = right_used + tw;
        }

        // ── App tray ─────────────────────────────────────────────────
        let app_tray_tex_draws;
        {
            let toplevels = state.tracker.toplevels();
            let (_, draws) = app_tray.draw(
                &mut painter, &mut text, &mut ix, &icon_cache, &palette,
                &toplevels,
                vis_x, vis_y, vis_w, vis_h,
                scale_f, phys_w, total_phys_h,
                tray_left_x,
            );
            app_tray_tex_draws = draws;

            // Hover preview — debounced to avoid spamming the compositor
            let hovered = if state.pointer_in_surface {
                app_tray.hovered_app(
                    &ix, phys_cx, phys_cy, &toplevels,
                    vis_x, vis_w, vis_h, scale_f,
                    tray_left_x,
                )
            } else {
                None
            };
            match hovered {
                Some((app_id, lx, lw)) => {
                    let bar_h_logical = vis_h / scale_f;
                    let changed = hover_pending.as_ref().map_or(true, |(id, _, _, _)| id != &app_id);
                    if changed {
                        hover_pending = Some((app_id, lx, lw, bar_h_logical));
                        hover_debounce = Instant::now();
                    } else if hover_debounce.elapsed() >= Duration::from_millis(150) {
                        if let Some((ref id, lx, lw, bh)) = hover_pending {
                            hover_client.hover(id, lx, lw, bh);
                        }
                    }
                }
                None => {
                    hover_pending = None;
                    hover_client.unhover();
                }
            }
        }

        // ── Overlays (layer 1) ────────────────────────────────────
        // Switch to overlay layer so popup shapes render ABOVE base text.
        painter.set_layer(1);
        text.set_layer(1);

        let mut menu_icon_draws = Vec::new();
        let mut menu_modal_icon_draws = Vec::new();
        app_menu.draw(
            &mut painter, &mut text, &mut ix, &icon_cache, &palette,
            vis_x, bar_y_offset, scale_f, phys_w, total_phys_h,
            &mut menu_icon_draws, &mut menu_modal_icon_draws,
        );

        // Calendar popup
        clock.draw_calendar(
            &mut painter, &mut text, &mut ix, &palette,
            phys_w_f, bar_y_offset, bar_h_f, position_top, scale_f,
            phys_w, total_phys_h,
        );

        // Battery popup
        if let Some(bat) = &battery {
            bat.draw_popup(
                &mut painter, &mut text, &mut ix, &palette,
                bat_draw_x, bat_draw_w, bar_y_offset, bar_h_f, position_top, scale_f,
                phys_w, total_phys_h,
            );
        }

        // Temperature popup
        temperature.draw_popup(
            &mut painter, &mut text, &mut ix, &palette,
            temp_draw_x, temp_draw_w, bar_y_offset, bar_h_f, position_top, scale_f,
            phys_w, total_phys_h,
        );

        // WiFi popup
        wifi.draw_popup(
            &mut painter, &mut text, &mut ix, &palette,
            wifi_draw_x, wifi_draw_w, bar_y_offset, bar_h_f, position_top, scale_f,
            phys_w, total_phys_h,
        );

        // Bluetooth popup
        bluetooth.draw_popup(
            &mut painter, &mut text, &mut ix, &palette,
            bt_draw_x, bt_draw_w, bar_y_offset, bar_h_f, position_top, scale_f,
            phys_w, total_phys_h,
        );

        // Audio popup
        audio.draw_popup(
            &mut painter, &mut text, &mut ix, &palette,
            audio_draw_x, audio_draw_w, bar_y_offset, bar_h_f, position_top, scale_f,
            phys_w, total_phys_h,
        );

        // Context menu (layer 2 — always on top of popups)
        painter.set_layer(2);
        text.set_layer(2);
        context_menu.update(dt);
        if let Some(event) = context_menu.draw(&mut painter, &mut text, &mut ix, phys_w, total_phys_h) {
            match event {
                MenuEvent::CheckboxToggled { id: MENU_FLOAT_CHECKBOX, checked } => {
                    user_style = if checked { BarStyle::Floating } else { BarStyle::Docked };
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::CheckboxToggled { id: MENU_AUTOHIDE_CHECKBOX, checked } => {
                    auto_hide = checked;
                    if !checked {
                        hide_t = 0.0;
                        hide_timer = 0.0;
                        apply_layer_config(&layer_surface, state.width, bar_height, anim_t, auto_hide, position_top);
                        surface.commit();
                    }
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::Action(MENU_OPEN_TERMINAL) => {
                    let _ = std::process::Command::new("lntrn-terminal").spawn();
                    context_menu.close();
                }
                MenuEvent::Action(MENU_OPEN_FILES) => {
                    let _ = std::process::Command::new("lntrn-file-manager").spawn();
                    context_menu.close();
                }
                MenuEvent::Action(MENU_APP_PIN) => {
                    if let Some(ref app_id) = ctx_menu_app {
                        if app_tray.is_pinned(app_id) {
                            app_tray.unpin(app_id);
                        } else {
                            app_tray.pin(app_id);
                        }
                    }
                    context_menu.close();
                }
                MenuEvent::Action(MENU_APP_LAUNCH) => {
                    if let Some(ref app_id) = ctx_menu_app {
                        // Look up exec from .desktop files
                        let apps = crate::desktop::scan_apps();
                        if let Some(entry) = apps.iter().find(|e| e.app_id == *app_id) {
                            crate::appmenu::launch_app(&entry.exec);
                        } else {
                            // Fallback: try launching the app_id directly
                            crate::appmenu::launch_app(app_id);
                        }
                    }
                    context_menu.close();
                }
                MenuEvent::Action(MENU_APP_CLOSE) => {
                    if let Some(ref app_id) = ctx_menu_app {
                        state.tracker.close(app_id);
                    }
                    context_menu.close();
                }
                MenuEvent::Action(MENU_PROC_PIN) => {
                    app_menu.sysmon.pin_right_clicked();
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::Action(MENU_PROC_UNPIN) => {
                    if let Some((name, _)) = app_menu.sysmon.right_clicked_proc.clone() {
                        app_menu.sysmon.unpin(&name);
                    }
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::Action(MENU_PROC_KILL) => {
                    app_menu.sysmon.kill_right_clicked();
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::CheckboxToggled { id: MENU_LAVA_LAMP, checked } => {
                    lava.enabled = checked;
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::CheckboxToggled { id: MENU_TRAY_LEFT, checked } => {
                    tray_left = checked;
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::SliderChanged { id: MENU_OPACITY_SLIDER, value } => {
                    bar_opacity = value.clamp(0.0, 1.0);
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                }
                MenuEvent::SliderChanged { id: MENU_HEIGHT_SLIDER, value } => {
                    bar_height = slider_to_height(value);
                    state.height = surface_height(bar_height);
                    apply_layer_config(&layer_surface, state.width, bar_height, anim_t, auto_hide, position_top);
                    gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
                    if let Some(vp) = &viewport {
                        vp.set_destination(state.width as i32, state.height as i32);
                    }
                    surface.commit();
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                }
                MenuEvent::Action(MENU_MOVE_POSITION) => {
                    position_top = !position_top;
                    apply_layer_config(&layer_surface, state.width, bar_height, anim_t, auto_hide, position_top);
                    surface.commit();
                    save_settings(user_style, auto_hide, bar_height, bar_opacity, lava.enabled, position_top, tray_left, &app_menu.sysmon.pinned);
                    context_menu.close();
                }
                MenuEvent::Action(id) if id >= crate::dbusmenu::DBUSMENU_ID_BASE => {
                    let item_id = (id - crate::dbusmenu::DBUSMENU_ID_BASE) as i32;
                    pending_dbusmenu_click = Some(item_id);
                    context_menu.close();
                }
                _ => context_menu.close(),
            }
        }

        // ── Render (layered) ─────────────────────────────────────────
        match gpu.begin_frame("Bar") {
            Ok(mut frame) => {
                let view = frame.view().clone();

                // Layer 0: base shapes + base textures + base text
                painter.render_layer(0, &gpu, frame.encoder_mut(), &view, Some(Color::TRANSPARENT));
                let mut base_tex_draws = tray_tex_draws;
                base_tex_draws.extend(battery_tex_draws);
                base_tex_draws.extend(temp_tex_draws);
                base_tex_draws.extend(wifi_tex_draws);
                base_tex_draws.extend(bt_tex_draws);
                base_tex_draws.extend(audio_tex_draws);
                base_tex_draws.extend(app_tray_tex_draws);
                if !base_tex_draws.is_empty() {
                    tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &base_tex_draws, None);
                }
                text.render_layer(0, &gpu, frame.encoder_mut(), &view);

                // Flush so glyphon's prepare() for layer 1 doesn't overwrite layer 0 vertices
                frame.flush(&gpu);

                // Layer 1: overlay shapes + overlay textures + overlay text
                painter.render_layer(1, &gpu, frame.encoder_mut(), &view, None);
                let mut overlay_tex_draws: Vec<TextureDraw> = Vec::new();
                for (key, x, y, w, h, clip) in &menu_icon_draws {
                    if let Some(tex) = icon_cache.get(key) {
                        overlay_tex_draws.push(TextureDraw {
                            texture: tex, x: *x, y: *y, w: *w, h: *h,
                            opacity: 1.0, uv: [0.0, 0.0, 1.0, 1.0],
                            clip: *clip,
                        });
                    }
                }
                if !overlay_tex_draws.is_empty() {
                    tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &overlay_tex_draws, None);
                }
                text.render_layer(1, &gpu, frame.encoder_mut(), &view);

                // Layer 2: nested overlays (context menu OR power confirmation modal)
                if painter.layer_count() > 2 {
                    frame.flush(&gpu);
                    painter.render_layer(2, &gpu, frame.encoder_mut(), &view, None);
                    // Modal icons render here so they sit ON TOP of the modal card
                    // shapes drawn by layer 2 painter, but still BELOW layer 2 text.
                    let mut modal_tex_draws: Vec<TextureDraw> = Vec::new();
                    for (key, x, y, w, h, clip) in &menu_modal_icon_draws {
                        if let Some(tex) = icon_cache.get(key) {
                            modal_tex_draws.push(TextureDraw {
                                texture: tex, x: *x, y: *y, w: *w, h: *h,
                                opacity: 1.0, uv: [0.0, 0.0, 1.0, 1.0],
                                clip: *clip,
                            });
                        }
                    }
                    if !modal_tex_draws.is_empty() {
                        tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &modal_tex_draws, None);
                    }
                    text.render_layer(2, &gpu, frame.encoder_mut(), &view);
                }

                frame.submit(&gpu.queue);
            }
            Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            }
            Err(SurfaceError::OutOfMemory) => {
                tracing::error!("GPU OOM, exiting");
                break;
            }
            Err(_) => {}
        }

        // Process deferred dbusmenu click (after tray texture borrows are released)
        if let Some(item_id) = pending_dbusmenu_click.take() {
            if let (Some(ref bus), Some(ref path)) = (&tray_menu_bus, &tray_menu_path) {
                if let Some(tray) = &system_tray {
                    tray.send_menu_click(bus, path, item_id);
                }
            }
            tray_menu_bus = None;
            tray_menu_path = None;
        }

        // Determine if we need continuous frames BEFORE committing
        needs_anim = (anim_t - target).abs() > 0.001
            || (hide_t - hide_target).abs() > 0.001
            || (auto_hide && !state.pointer_in_surface && hide_t < 0.99)
            || tray_animating || bat_animating || bt_animating || app_tray.is_dragging()
            || audio.is_dragging();

        // Only request a frame callback when we need continuous rendering.
        // Without this guard, the callback fires → sets frame_done → renders
        // → requests callback again, creating a perpetual render loop at idle.
        if needs_anim {
            surface.frame(&qh, ());
        }
        apply_layer_config(&layer_surface, state.width, bar_height, anim_t, auto_hide, position_top);
        surface.commit();
    }

    Ok(())
}

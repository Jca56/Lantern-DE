use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
        utils::{Logical, Physical, Point, Rectangle, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        cursor_shape::CursorShapeManagerState,
        dmabuf::{DmabufGlobal, DmabufState},
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        output::OutputManagerState,
        pointer_gestures::PointerGesturesState,
        selection::data_device::DataDeviceState,
        selection::wlr_data_control::DataControlState,
        shell::{
            wlr_layer::{WlrLayerShellState, LayerSurface, LayerSurfaceCachedState, Anchor, ExclusiveZone},
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
    },
};

use smithay::backend::renderer::gles::GlesTexture;
use crate::animation::{AnimationState, ClosingWindow};
use crate::canvas::Canvas;
use crate::input::AudioRepeat;
use crate::cursor::CursorState;
use crate::gestures::GestureState;
use crate::ssd::SsdManager;
use crate::handlers::foreign_toplevel::ForeignToplevelManagerState;
use crate::handlers::output_management::OutputManagementState;
use crate::handlers::screencopy::{PendingScreencopy, ScreencopyManagerState};
use crate::handlers::xdg_foreign::XdgForeignState;
use crate::hot_corners::HotCornerState;
use crate::snap::SnappedWindow;
use crate::switcher::AltTabSwitcher;
use crate::tiling::PerOutputTiling;
use crate::tiling_anim::TilingAnimationState;
use crate::udev::UdevData;
use crate::wallpaper::WallpaperState;

const COUNTER_REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone)]
pub struct MinimizedWindow {
    pub surface: WlSurface,
    pub window: Window,
    pub location: Point<i32, Logical>,
}

#[derive(Clone)]
pub struct FullscreenWindow {
    pub surface: WlSurface,
    pub restore: Rectangle<i32, Logical>,
}

#[derive(Clone)]
pub struct MaximizedWindow {
    pub surface: WlSurface,
    pub restore: Rectangle<i32, Logical>,
}

pub struct DebugCounters {
    pub(crate) enabled: bool,
    window_start: std::time::Instant,
    pub(crate) renders: u64,
    pub(crate) frame_callbacks: u64,
    pub(crate) scheduled_renders: u64,
    pub(crate) forced_renders: u64,
    pub(crate) winit_redraw_requests: u64,
}

impl DebugCounters {
    fn from_env() -> Self {
        let flag_path = crate::lantern_home().join("log/compositor-debug.enabled");

        Self {
            enabled: std::env::var("LNTRN_COMPOSITOR_DEBUG_COUNTERS")
                .map(|value| value != "0")
                .unwrap_or(false)
                || flag_path.exists(),
            window_start: std::time::Instant::now(),
            renders: 0,
            frame_callbacks: 0,
            scheduled_renders: 0,
            forced_renders: 0,
            winit_redraw_requests: 0,
        }
    }

    pub(crate) fn maybe_report(&mut self) {
        if !self.enabled {
            return;
        }

        let elapsed = self.window_start.elapsed();
        if elapsed < COUNTER_REPORT_INTERVAL {
            return;
        }

        let secs = elapsed.as_secs_f64();
        tracing::info!(
            target: "performance",
            renders_per_sec = self.renders as f64 / secs,
            frame_callbacks_per_sec = self.frame_callbacks as f64 / secs,
            scheduled_renders_per_sec = self.scheduled_renders as f64 / secs,
            forced_renders_per_sec = self.forced_renders as f64 / secs,
            winit_redraw_requests_per_sec = self.winit_redraw_requests as f64 / secs,
            "lntrn-compositor counters"
        );

        self.window_start = std::time::Instant::now();
        self.renders = 0;
        self.frame_callbacks = 0;
        self.scheduled_renders = 0;
        self.forced_renders = 0;
        self.winit_redraw_requests = 0;
    }
}

pub struct Lantern {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, Lantern>,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Protocol state
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub viewporter_state: ViewporterState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Lantern>,
    pub data_device_state: DataDeviceState,
    pub data_control_state: DataControlState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    pub layer_shell_state: WlrLayerShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub xdg_activation_state: XdgActivationState,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub screencopy_state: ScreencopyManagerState,
    pub pending_screencopy: Vec<PendingScreencopy>,
    pub foreign_toplevel_state: ForeignToplevelManagerState,
    pub output_management_state: OutputManagementState,
    pub pointer_gestures_state: PointerGesturesState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,

    // Cursor
    pub cursor: CursorState,

    // Backend-specific state
    pub udev: Option<UdevData>,
    pub winit_redraw_requested: Arc<AtomicBool>,
    pub pending_client_frame_callbacks: bool,
    pub last_pointer_render_location: Option<(i32, i32)>,
    pub debug_counters: DebugCounters,
    pub focused_surface: Option<WlSurface>,
    pub window_mru: Vec<WlSurface>,
    pub window_spawn_order: Vec<WlSurface>,
    pub minimized_windows: Vec<MinimizedWindow>,
    pub maximized_windows: Vec<MaximizedWindow>,
    pub fullscreen_windows: Vec<FullscreenWindow>,
    pub alt_tab_switcher: AltTabSwitcher,
    pub wallpaper: WallpaperState,
    pub wallpaper_frame_counter: u32,
    pub layer_surfaces: Vec<LayerSurface>,
    pub layer_surface_outputs: HashMap<WlSurface, Output>,
    pub window_opacity: HashMap<WlSurface, f32>,
    pub default_window_opacity: f32,
    /// App IDs that skip blur backdrop and use full opacity.
    pub blur_exclude: Vec<String>,
    pub window_zoom: HashMap<WlSurface, f64>,
    pub focus_glow: bool,
    pub focus_glow_color: [f32; 4],
    pub focus_glow_intensity: f32,
    pub focus_follows_mouse: bool,
    pub super_pressed: bool,
    /// True if Super was pressed and no Super+combo was used (for tap detection)
    pub super_clean_tap: bool,
    pub snapped_windows: Vec<SnappedWindow>,
    pub animations: AnimationState,
    /// Windows that died (client-initiated close) but still have a close animation playing.
    pub closing_windows: Vec<ClosingWindow>,
    /// Per-window snapshot textures captured each render frame for close animations.
    pub window_snapshots: HashMap<WlSurface, (GlesTexture, Size<i32, Physical>)>,
    pub tiling: PerOutputTiling,
    pub tiling_anim: TilingAnimationState,
    pub gesture: GestureState,
    pub canvas: Canvas,

    // Scratchpad (dropdown terminal)
    pub scratchpad_surface: Option<WlSurface>,
    pub scratchpad_pending: bool,

    // Hot corners
    pub hot_corner: HotCornerState,
    pub show_desktop_active: bool,

    // xdg-foreign: cross-client parent-child window relationships
    pub xdg_foreign_state: XdgForeignState,

    // Audio key repeat
    pub audio_repeat: Option<AudioRepeat>,

    // Cached exclusive zone offsets — reconfigure maximized windows when these change
    pub last_exclusive_offsets: (i32, i32, i32, i32),

    // Server-side decorations
    pub ssd: SsdManager,

    // Input settings (read from lantern.toml)
    pub mouse_speed: f64,
    pub cursor_theme_name: String,
    pub input_config_counter: u32,

    // Hover preview (bar → compositor IPC for window thumbnails)
    pub hover_preview: crate::hover_preview::HoverPreview,

    // XWayland support
    pub xwayland_state: crate::xwayland::XWaylandState,
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    pub override_redirect_windows: Vec<Window>,
    /// X11 windows waiting for their Wayland surface to be associated.
    pub pending_x11_windows: Vec<Window>,

    // Window centering: windows waiting for their first real geometry before being centered
    pub pending_center: HashSet<WlSurface>,
    pub center_cascade_counter: i32,
}

impl Lantern {
    pub fn new(event_loop: &mut EventLoop<'static, Self>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let popups = PopupManager::default();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let data_control_state = DataControlState::new::<Self, _>(&dh, None, |_| true);
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let screencopy_state = ScreencopyManagerState::new(&dh);
        let foreign_toplevel_state = ForeignToplevelManagerState::new(&dh);
        let output_management_state = OutputManagementState::new(&dh);
        let xdg_foreign_state = XdgForeignState::new(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<Self>(&dh);
        let xwayland_shell_state = smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "lantern");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();
        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();
        let loop_handle = event_loop.handle();

        Self {
            start_time,
            display_handle: dh,
            loop_handle,
            space,
            loop_signal,
            socket_name,
            compositor_state,
            xdg_shell_state,
            shm_state,
            fractional_scale_manager_state,
            viewporter_state,
            output_manager_state,
            seat_state,
            data_device_state,
            data_control_state,
            cursor_shape_manager_state,
            layer_shell_state,
            xdg_decoration_state,
            xdg_activation_state,
            idle_inhibit_manager_state,
            dmabuf_state,
            dmabuf_global: None,
            screencopy_state,
            pending_screencopy: Vec::new(),
            foreign_toplevel_state,
            output_management_state,
            pointer_gestures_state,
            popups,
            seat,
            cursor: CursorState::new(&crate::input::read_input_setting("cursor_theme", "default")),
            udev: None,
            winit_redraw_requested: Arc::new(AtomicBool::new(false)),
            pending_client_frame_callbacks: false,
            last_pointer_render_location: None,
            debug_counters: DebugCounters::from_env(),
            focused_surface: None,
            window_mru: Vec::new(),
            window_spawn_order: Vec::new(),
            minimized_windows: Vec::new(),
            maximized_windows: Vec::new(),
            fullscreen_windows: Vec::new(),
            alt_tab_switcher: AltTabSwitcher::new(),
            wallpaper: WallpaperState::load_from_config(),
            wallpaper_frame_counter: 0,
            layer_surfaces: Vec::new(),
            layer_surface_outputs: HashMap::new(),
            window_opacity: HashMap::new(),
            default_window_opacity: crate::read_config_f32("window_opacity", 1.0),
            blur_exclude: crate::read_config_list("windows", "blur_exclude"),
            window_zoom: HashMap::new(),
            focus_glow: crate::read_config("window_manager", "focus_glow", "true") == "true",
            focus_glow_color: crate::parse_glow_color(&crate::read_config("window_manager", "focus_glow_color", "#4A9EFF")),
            focus_glow_intensity: crate::read_config("window_manager", "focus_glow_intensity", "0.2")
                .parse::<f32>().unwrap_or(0.2).clamp(0.0, 0.6),
            focus_follows_mouse: crate::read_config("window_manager", "focus_follows_mouse", "false") == "true",
            super_pressed: false,
            super_clean_tap: false,
            snapped_windows: Vec::new(),
            animations: AnimationState::new(),
            closing_windows: Vec::new(),
            window_snapshots: HashMap::new(),
            tiling: PerOutputTiling::new(),
            tiling_anim: TilingAnimationState::new(),
            gesture: GestureState::new(),
            canvas: Canvas::new(),
            scratchpad_surface: None,
            scratchpad_pending: false,
            hot_corner: HotCornerState::new(),
            show_desktop_active: false,
            xdg_foreign_state,
            audio_repeat: None,
            last_exclusive_offsets: (0, 0, 0, 0),
            ssd: SsdManager::new(),
            mouse_speed: crate::input::read_input_setting_f64("mouse_speed", 0.0),
            cursor_theme_name: crate::input::read_input_setting("cursor_theme", "default"),
            input_config_counter: 0,
            hover_preview: crate::hover_preview::HoverPreview::new(),
            xwayland_state: crate::xwayland::XWaylandState::new(),
            xwayland_shell_state,
            override_redirect_windows: Vec::new(),
            pending_x11_windows: Vec::new(),
            pending_center: HashSet::new(),
            center_cascade_counter: 0,
        }
    }

    fn init_wayland_listener(
        display: Display<Lantern>,
        event_loop: &mut EventLoop<'static, Self>,
    ) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    if let Err(e) = unsafe { display.get_mut().dispatch_clients(state) } {
                        tracing::error!("dispatch_clients failed: {:?}", e);
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::wlr_layer::Layer;

        // Check layer surfaces first (Top/Overlay are above windows)
        // Layer surfaces are in screen-space — no canvas transform
        // Use the output the pointer is on for layer surface positioning
        // Skip if a fullscreen window covers this output — fullscreen takes priority
        if let Some(output) = self.output_at_point(pos) {
            let output_has_fullscreen = self.fullscreen_windows.iter().any(|fw| {
                self.find_mapped_window(&fw.surface)
                    .and_then(|w| self.output_for_window(&w))
                    .map_or(false, |o| o == output)
            });
            let output_geo = self.space.output_geometry(&output).unwrap_or_default();
            for ls in &self.layer_surfaces {
                if output_has_fullscreen { break; }
                if !ls.alive() {
                    continue;
                }
                let cached = with_states(ls.wl_surface(), |states| {
                    *states.cached_state.get::<LayerSurfaceCachedState>().current()
                });
                // Only intercept pointer for Top/Overlay layers (above windows)
                if cached.layer != Layer::Top && cached.layer != Layer::Overlay {
                    continue;
                }
                let ls_loc = crate::render::layer_surface_position_logical(&cached, output_geo);
                let size = cached.size;
                let rect = Rectangle::new(ls_loc, size);
                let pos_i = Point::from((pos.x as i32, pos.y as i32));
                if rect.contains(pos_i) {
                    let relative = pos - ls_loc.to_f64();
                    // Check the actual surface tree for subsurfaces
                    if let Some((sub_surface, sub_loc)) = smithay::desktop::utils::under_from_surface_tree(
                        ls.wl_surface(),
                        relative,
                        (0, 0),
                        WindowSurfaceType::ALL,
                    ) {
                        return Some((sub_surface, (sub_loc.to_f64() + ls_loc.to_f64())));
                    }
                }
            }
        }

        // Transform screen-space pointer to canvas-space for window hit-testing
        let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
        let canvas_pos = Point::from((cx, cy));

        // Check windows in the space (which live in canvas-space)
        let window_hit = self.space
            .element_under(canvas_pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(canvas_pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| {
                        let canvas_abs = (p + location).to_f64();
                        let (sx, sy) = self.canvas.canvas_to_screen(canvas_abs.x, canvas_abs.y);
                        (s, Point::from((sx, sy)))
                    })
            });
        if window_hit.is_some() {
            return window_hit;
        }

        // Check Bottom layer surfaces (below windows, above wallpaper)
        if let Some(output) = self.output_at_point(pos) {
            let output_geo = self.space.output_geometry(&output).unwrap_or_default();
            for ls in &self.layer_surfaces {
                if !ls.alive() { continue; }
                let cached = with_states(ls.wl_surface(), |states| {
                    *states.cached_state.get::<LayerSurfaceCachedState>().current()
                });
                if cached.layer != Layer::Bottom { continue; }
                let ls_loc = crate::render::layer_surface_position_logical(&cached, output_geo);
                // cached.size is (0,0) when client requests auto-fill — use output size
                let size: smithay::utils::Size<i32, Logical> = (
                    if cached.size.w > 0 { cached.size.w } else { output_geo.size.w },
                    if cached.size.h > 0 { cached.size.h } else { output_geo.size.h },
                ).into();
                let rect = Rectangle::new(ls_loc, size);
                let pos_i = Point::from((pos.x as i32, pos.y as i32));
                if rect.contains(pos_i) {
                    let relative = pos - ls_loc.to_f64();
                    if let Some((sub_surface, sub_loc)) = smithay::desktop::utils::under_from_surface_tree(
                        ls.wl_surface(),
                        relative,
                        (0, 0),
                        WindowSurfaceType::ALL,
                    ) {
                        return Some((sub_surface, (sub_loc.to_f64() + ls_loc.to_f64())));
                    }
                    // wgpu surfaces may not register in Smithay's surface tree —
                    // fall back to returning the layer surface directly
                    return Some((ls.wl_surface().clone(), ls_loc.to_f64()));
                }
            }
        }

        None
    }

    pub fn request_winit_redraw(&self) {
        self.winit_redraw_requested.store(true, Ordering::Release);
        self.loop_signal.wakeup();
    }

    pub fn take_winit_redraw_request(&self) -> bool {
        self.winit_redraw_requested.swap(false, Ordering::AcqRel)
    }

    pub fn schedule_render(&mut self) {
        if self.debug_counters.enabled {
            self.debug_counters.scheduled_renders += 1;
        }
        if self.udev.is_some() {
            crate::udev::schedule_render_all(self);
        } else {
            self.request_winit_redraw();
        }
        self.debug_counters.maybe_report();
    }

    pub fn schedule_client_render(&mut self) {
        self.pending_client_frame_callbacks = true;
        self.schedule_render();
    }

    pub fn schedule_render_forced(&mut self) {
        if self.debug_counters.enabled {
            self.debug_counters.forced_renders += 1;
        }
        if self.udev.is_some() {
            crate::udev::schedule_render_forced(self);
        } else {
            self.request_winit_redraw();
        }
        self.debug_counters.maybe_report();
    }

    pub fn cycle_desktop_panel(&self) {
        let path = crate::lantern_home().join("config/desktop-panel");
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        let panels = ["home", "terminal", "files"];
        let idx = panels.iter().position(|p| current.trim() == *p).unwrap_or(0);
        let next = (idx + 1) % panels.len();
        let _ = std::fs::write(&path, panels[next]);
        tracing::info!("Desktop panel: {} → {}", panels[idx], panels[next]);
    }

    pub fn record_render(&mut self, frame_callbacks: usize) {
        if self.debug_counters.enabled {
            self.debug_counters.renders += 1;
            self.debug_counters.frame_callbacks += frame_callbacks as u64;
        }
        self.debug_counters.maybe_report();
    }

    pub fn record_winit_redraw_request(&mut self) {
        if self.debug_counters.enabled {
            self.debug_counters.winit_redraw_requests += 1;
        }
        self.debug_counters.maybe_report();
    }

    pub fn should_render_pointer_motion(
        &mut self,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let rounded = (location.x.round() as i32, location.y.round() as i32);
        if self.last_pointer_render_location == Some(rounded) {
            return false;
        }

        self.last_pointer_render_location = Some(rounded);
        true
    }

    /// Find the output whose geometry contains `point`.
    /// Falls back to the closest output if the point is between monitors.
    pub fn output_at_point(&self, point: Point<f64, Logical>) -> Option<Output> {
        // Exact containment check
        for output in self.space.outputs() {
            if let Some(geo) = self.space.output_geometry(output) {
                if geo.to_f64().contains(point) {
                    return Some(output.clone());
                }
            }
        }
        // Fallback: closest output center
        self.space
            .outputs()
            .min_by_key(|o| {
                let geo = self.space.output_geometry(o).unwrap();
                let cx = geo.loc.x + geo.size.w / 2;
                let cy = geo.loc.y + geo.size.h / 2;
                let dx = point.x - cx as f64;
                let dy = point.y - cy as f64;
                (dx * dx + dy * dy) as i64
            })
            .cloned()
    }

    /// Find the output a window lives on by checking which output contains its center.
    pub fn output_for_window(&self, window: &Window) -> Option<Output> {
        let loc = self.space.element_location(window)?;
        let size = window.geometry().size;
        let center = Point::from((
            loc.x as f64 + size.w as f64 / 2.0,
            loc.y as f64 + size.h as f64 / 2.0,
        ));
        self.output_at_point(center)
    }

    /// Combined bounding box of all outputs.
    pub fn total_output_bounds(&self) -> Rectangle<i32, Logical> {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for output in self.space.outputs() {
            if let Some(geo) = self.space.output_geometry(output) {
                min_x = min_x.min(geo.loc.x);
                min_y = min_y.min(geo.loc.y);
                max_x = max_x.max(geo.loc.x + geo.size.w);
                max_y = max_y.max(geo.loc.y + geo.size.h);
            }
        }
        if min_x == i32::MAX {
            return Rectangle::default();
        }
        Rectangle::new(
            (min_x, min_y).into(),
            (max_x - min_x, max_y - min_y).into(),
        )
    }

    /// Compute the total exclusive zone offsets from all layer surfaces.
    /// Compute exclusive zone offsets for a specific output.
    /// Only counts layer surfaces assigned to that output.
    pub fn exclusive_zone_offsets_for_output(&self, output: &Output) -> (i32, i32, i32, i32) {
        use smithay::wayland::compositor::with_states;
        let mut top = 0i32;
        let mut bottom = 0i32;
        let mut left = 0i32;
        let mut right = 0i32;

        let output_name = output.name();
        for ls in &self.layer_surfaces {
            if !ls.alive() {
                continue;
            }
            // Only count layer surfaces assigned to this output
            if let Some(ls_output) = self.layer_surface_outputs.get(ls.wl_surface()) {
                if ls_output.name() != output_name {
                    continue;
                }
            }
            let cached = with_states(ls.wl_surface(), |states| {
                *states.cached_state.get::<LayerSurfaceCachedState>().current()
            });
            let zone = match cached.exclusive_zone {
                ExclusiveZone::Exclusive(v) => v as i32,
                _ => continue,
            };
            let anchor = cached.anchor;
            if anchor.contains(Anchor::TOP) && !anchor.contains(Anchor::BOTTOM) {
                top = top.max(zone + cached.margin.top);
            } else if anchor.contains(Anchor::BOTTOM) && !anchor.contains(Anchor::TOP) {
                bottom = bottom.max(zone + cached.margin.bottom);
            } else if anchor.contains(Anchor::LEFT) && !anchor.contains(Anchor::RIGHT) {
                left = left.max(zone + cached.margin.left);
            } else if anchor.contains(Anchor::RIGHT) && !anchor.contains(Anchor::LEFT) {
                right = right.max(zone + cached.margin.right);
            }
        }

        (top, bottom, left, right)
    }

    /// Global exclusive zone offsets (sum across all outputs). Legacy fallback.
    pub fn exclusive_zone_offsets(&self) -> (i32, i32, i32, i32) {
        use smithay::wayland::compositor::with_states;
        let mut top = 0i32;
        let mut bottom = 0i32;
        let mut left = 0i32;
        let mut right = 0i32;

        for ls in &self.layer_surfaces {
            if !ls.alive() {
                continue;
            }
            let cached = with_states(ls.wl_surface(), |states| {
                *states.cached_state.get::<LayerSurfaceCachedState>().current()
            });
            let zone = match cached.exclusive_zone {
                ExclusiveZone::Exclusive(v) => v as i32,
                _ => continue,
            };
            let anchor = cached.anchor;
            if anchor.contains(Anchor::TOP) && !anchor.contains(Anchor::BOTTOM) {
                top = top.max(zone + cached.margin.top);
            } else if anchor.contains(Anchor::BOTTOM) && !anchor.contains(Anchor::TOP) {
                bottom = bottom.max(zone + cached.margin.bottom);
            } else if anchor.contains(Anchor::LEFT) && !anchor.contains(Anchor::RIGHT) {
                left = left.max(zone + cached.margin.left);
            } else if anchor.contains(Anchor::RIGHT) && !anchor.contains(Anchor::LEFT) {
                right = right.max(zone + cached.margin.right);
            }
        }

        (top, bottom, left, right)
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

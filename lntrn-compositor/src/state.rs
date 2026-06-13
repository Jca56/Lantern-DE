//! Central compositor state.
//!
//! [`Lantern`] is the god-struct every handler and the event loop mutate:
//! it owns the Smithay protocol states, the `Space`, per-output
//! workspaces, input/cursor/animation state, and the live lists of
//! windows in each non-normal layout state. It's intentionally large — a
//! Smithay compositor's central state always is — so the `impl Lantern`
//! block below is grouped into `// ──`-marked sections to stay navigable.
//!
//! Support types live nearby: the per-layout-state records
//! ([`MinimizedWindow`](crate::window_state::MinimizedWindow) et al.) in
//! [`crate::window_state`]; [`PendingWorkspaceMove`] and [`DebugCounters`]
//! here since they're only touched from this module's machinery.

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
        pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        session_lock::SessionLockManagerState,
        selection::data_device::DataDeviceState,
        selection::ext_data_control::DataControlState as ExtDataControlState,
        selection::wlr_data_control::DataControlState as WlrDataControlState,
        shell::{
            wlr_layer::{WlrLayerShellState, LayerSurface, LayerSurfaceCachedState, Anchor, ExclusiveZone},
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
    },
};

use smithay::backend::renderer::gles::GlesTexture;
use crate::animation::{AnimationState, ClosingWindow};
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
use crate::workspace_anim::WorkspaceAnimState;
use crate::workspace_ipc::WorkspaceIpc;
use crate::workspaces::PerOutputWorkspaces;
use crate::minimize_anim::MinimizeAnimState;
use crate::udev::UdevData;
use crate::wallpaper::WallpaperState;
use crate::window_state::{FullscreenWindow, MaximizedWindow, MinimizedWindow, SoloTiledWindow};
use crate::window_state_anim::WindowStateAnimState;

const COUNTER_REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

// Window-state records (MinimizedWindow / FullscreenWindow /
// MaximizedWindow / SoloTiledWindow) live in `crate::window_state`.

/// A window-to-workspace move that's mid slide-off animation. While
/// `complete_at` hasn't elapsed the window remains in the source
/// workspace's Space (sliding off-screen via `window_state_anim`). Once
/// the deadline passes, the deferred unmap-from-source + remap-onto-target
/// fires in `process_pending_workspace_moves`.
pub struct PendingWorkspaceMove {
    pub surface: WlSurface,
    pub target_output: String,
    pub target_workspace_id: u32,
    /// Position to drop the window at on the target workspace's Space.
    pub final_pos: Point<i32, Logical>,
    pub complete_at: std::time::Instant,
}

pub struct DebugCounters {
    pub(crate) enabled: bool,
    window_start: std::time::Instant,
    pub(crate) renders: u64,
    pub(crate) frame_callbacks: u64,
    pub(crate) scheduled_renders: u64,
    pub(crate) forced_renders: u64,
    pub(crate) winit_redraw_requests: u64,
    pub(crate) commits: u64,
    pub(crate) dispatch_iters: u64,
    pub(crate) dispatch_events: u64,
    pub(crate) dispatch_micros: u64,
    pub(crate) commit_micros: u64,
    pub(crate) render_micros: u64,
    pub(crate) loop_iters: u64,
    pub(crate) loop_micros: u64,
    pub(crate) flush_micros: u64,
    pub(crate) libinput_fires: u64,
    pub(crate) drm_fires: u64,
    pub(crate) timer_fires: u64,
    pub(crate) wayland_fires: u64,
    pub(crate) listener_fires: u64,
    pub(crate) udev_fires: u64,
    pub(crate) session_fires: u64,
    pub(crate) xwayland_fires: u64,
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
            commits: 0,
            dispatch_iters: 0,
            dispatch_events: 0,
            dispatch_micros: 0,
            commit_micros: 0,
            render_micros: 0,
            loop_iters: 0,
            loop_micros: 0,
            flush_micros: 0,
            libinput_fires: 0,
            drm_fires: 0,
            timer_fires: 0,
            wayland_fires: 0,
            listener_fires: 0,
            udev_fires: 0,
            session_fires: 0,
            xwayland_fires: 0,
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
            commits_per_sec = self.commits as f64 / secs,
            dispatch_iters_per_sec = self.dispatch_iters as f64 / secs,
            dispatch_events_per_sec = self.dispatch_events as f64 / secs,
            dispatch_pct = (self.dispatch_micros as f64 / 1_000_000.0 / secs) * 100.0,
            commit_pct = (self.commit_micros as f64 / 1_000_000.0 / secs) * 100.0,
            render_pct = (self.render_micros as f64 / 1_000_000.0 / secs) * 100.0,
            loop_iters_per_sec = self.loop_iters as f64 / secs,
            loop_pct = (self.loop_micros as f64 / 1_000_000.0 / secs) * 100.0,
            flush_pct = (self.flush_micros as f64 / 1_000_000.0 / secs) * 100.0,
            libinput_fires_per_sec = self.libinput_fires as f64 / secs,
            drm_fires_per_sec = self.drm_fires as f64 / secs,
            timer_fires_per_sec = self.timer_fires as f64 / secs,
            wayland_fires_per_sec = self.wayland_fires as f64 / secs,
            listener_fires_per_sec = self.listener_fires as f64 / secs,
            udev_fires_per_sec = self.udev_fires as f64 / secs,
            session_fires_per_sec = self.session_fires as f64 / secs,
            xwayland_fires_per_sec = self.xwayland_fires as f64 / secs,
            "lntrn-compositor counters"
        );

        self.window_start = std::time::Instant::now();
        self.renders = 0;
        self.frame_callbacks = 0;
        self.scheduled_renders = 0;
        self.forced_renders = 0;
        self.winit_redraw_requests = 0;
        self.commits = 0;
        self.dispatch_iters = 0;
        self.dispatch_events = 0;
        self.dispatch_micros = 0;
        self.commit_micros = 0;
        self.render_micros = 0;
        self.loop_iters = 0;
        self.loop_micros = 0;
        self.flush_micros = 0;
        self.libinput_fires = 0;
        self.drm_fires = 0;
        self.timer_fires = 0;
        self.wayland_fires = 0;
        self.listener_fires = 0;
        self.udev_fires = 0;
        self.session_fires = 0;
        self.xwayland_fires = 0;
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
    pub data_control_state: WlrDataControlState,
    pub ext_data_control_state: ExtDataControlState,
    pub clipboard_manager: crate::clipboard_manager::ClipboardManager,
    pub clipboard_ipc: crate::clipboard_ipc::ClipboardIpc,
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
    pub pointer_constraints_state: PointerConstraintsState,
    pub relative_pointer_state: RelativePointerManagerState,
    pub text_input_manager_state: TextInputManagerState,
    pub presentation_state: PresentationState,
    pub session_lock_state: SessionLockManagerState,
    /// Present while the session is locked (ext-session-lock-v1). `None` =
    /// unlocked. Holds the per-output lock surfaces + the pending confirmation.
    pub session_lock: Option<crate::handlers::session_lock::SessionLockData>,
    pub popups: PopupManager,

    pub seat: Seat<Self>,

    // Cursor
    pub cursor: CursorState,

    // Backend-specific state
    pub udev: Option<UdevData>,
    pub winit_redraw_requested: Arc<AtomicBool>,
    pub pending_client_frame_callbacks: bool,
    pub last_pointer_render_location: Option<(i32, i32)>,
    /// The (surface, surface_loc) the pointer was over after the last motion
    /// event — i.e. surface_under(prev_loc) for the NEXT event. Caching this
    /// halves the full surface-tree walks done per motion event (the
    /// pointer-constraint check needed its own walk at up to 1000Hz).
    /// Invalidated (None) whenever a motion event ends without computing a
    /// fresh hit (e.g. the Alt-Tab overlay intercepting motion).
    pub last_pointer_under: Option<(WlSurface, Point<f64, Logical>)>,
    /// Last time we sent frame callbacks. Used to keep the vblank stream
    /// running at 60Hz while clients are actively rendering, so wgpu FIFO
    /// presentation doesn't get throttled by sparse vblank events.
    pub last_callback_render: std::time::Instant,
    pub debug_counters: DebugCounters,
    pub focused_surface: Option<WlSurface>,
    pub window_mru: Vec<WlSurface>,
    pub window_spawn_order: Vec<WlSurface>,
    pub minimized_windows: Vec<MinimizedWindow>,
    pub maximized_windows: Vec<MaximizedWindow>,
    /// Windows currently at the "solo tile" size, driven by Super+Up/Down.
    pub solo_tiled_windows: Vec<SoloTiledWindow>,
    /// Posed windows (Shift+Super+Left/Right). Tracks which pose slot a
    /// window currently occupies so subsequent presses cycle through the
    /// Left → Middle → Right sequence instead of jumping straight to a half.
    pub posed_windows: HashMap<WlSurface, crate::window_management::PoseSlot>,
    /// In-flight workspace moves (Super+Shift+N). While the slide-off
    /// animation is running, the window stays on the source workspace's
    /// Space. After `complete_at`, the actual unmap+remap to the target
    /// workspace happens — see `process_pending_workspace_moves`.
    pub pending_workspace_moves: Vec<PendingWorkspaceMove>,
    pub fullscreen_windows: Vec<FullscreenWindow>,
    pub alt_tab_switcher: AltTabSwitcher,
    /// App-icon cache for the Alt+Tab switcher's corner badges.
    pub switcher_icons: crate::switcher::icons::SwitcherIconCache,
    pub wallpaper: WallpaperState,
    pub wallpaper_frame_counter: u32,
    pub layer_surfaces: Vec<LayerSurface>,
    pub layer_surface_outputs: HashMap<WlSurface, Output>,
    /// Layer-shell namespace per surface (the protocol-level LayerSurface
    /// doesn't retain it past `new_layer_surface`).
    pub layer_surface_namespaces: HashMap<WlSurface, String>,
    /// Per-output offscreen target for badge-free screencopy (only
    /// populated while a no-capture overlay — the recording indicator —
    /// is on screen; dropped again on the next plain capture).
    pub screencopy_offscreen: HashMap<String, crate::screencopy_render::OffscreenCapture>,
    /// Per-output PBO slot for asynchronous screencopy readback. The
    /// glReadPixels lands in the PBO without stalling the GPU; the pixels
    /// are mapped and delivered on the output's NEXT frame.
    pub screencopy_pbos: HashMap<String, crate::screencopy_render::ScreencopyPbo>,
    /// System-wide background opacity from `[windows].background_opacity`.
    /// When < 1.0, the compositor pushes a blur backdrop behind every
    /// non-fullscreen, non-excluded window so the apps' translucent
    /// backgrounds reveal a blurred desktop. Refreshed every poll cycle.
    pub system_bg_opacity: f32,
    /// App IDs that skip the blur backdrop.
    pub blur_exclude: Vec<String>,
    /// Blur strength/tint/darken from `[windows]`, cached on the same
    /// 30-frame poll cycle as `system_bg_opacity` — these were read from
    /// config (mutex + string alloc + line walk) every frame at 240Hz.
    pub blur_intensity: f32,
    pub blur_tint: f32,
    pub blur_darken: f32,
    /// Per-output cache of window chrome shader elements (drop shadow,
    /// border ring), keyed by output name → window surface. Rebuilding
    /// these every frame minted a fresh element Id each time, which defeats
    /// damage tracking (a new Id reads as a brand-new damaged element), and
    /// re-allocated the uniform vectors. Entries are pruned in
    /// `forget_window`.
    pub window_chrome_cache:
        HashMap<String, HashMap<WlSurface, crate::render::surface::ChromeCacheEntry>>,
    /// Global default initial window size (logical px). None = let client choose.
    pub default_window_size: Option<(i32, i32)>,
    /// Per-app initial window size overrides from `[[window_rules]]`.
    pub window_rules: Vec<crate::WindowRule>,
    pub window_zoom: HashMap<WlSurface, f64>,
    pub focus_glow: bool,
    pub focus_glow_color: [f32; 4],
    pub focus_glow_intensity: f32,
    pub border_color: [f32; 4],
    /// Blur underlay tint color (premultiplied component values get scaled
    /// by the [windows].blur_tint strength at draw time).
    pub blur_tint_color: [f32; 4],
    pub focus_follows_mouse: bool,
    /// Gaming Mode: when true, the primary output is dropped to scale 1.0 so
    /// fullscreen X11 games render at true native resolution (1:1, no fractional
    /// downscale) with perfectly-aligned input. Toggled with Super+G; restores
    /// the configured desktop scale when turned off. See `gaming_mode.rs`.
    pub gaming_mode: bool,
    pub super_pressed: bool,
    /// True if Super was pressed and no Super+combo was used (for tap detection)
    pub super_clean_tap: bool,
    /// When Super was last pressed — a long hold counts as a hold, not a tap.
    pub super_press_time: Option<std::time::Instant>,
    pub snapped_windows: Vec<SnappedWindow>,
    pub animations: AnimationState,
    /// Windows that died (client-initiated close) but still have a close animation playing.
    pub closing_windows: Vec<ClosingWindow>,
    /// Per-window snapshot textures captured each render frame for close animations.
    pub window_snapshots: HashMap<WlSurface, (GlesTexture, Size<i32, Physical>)>,
    pub workspaces: PerOutputWorkspaces,
    pub window_state_anim: WindowStateAnimState,
    pub minimize_anim: MinimizeAnimState,
    pub workspace_anim: WorkspaceAnimState,
    pub workspace_ipc: WorkspaceIpc,
    pub hdr_ipc: crate::hdr_ipc::HdrIpc,
    pub gaming_ipc: crate::gaming_ipc::GamingIpc,
    pub window_query_ipc: crate::window_query_ipc::WindowQueryIpc,
    /// Output names with HDR currently engaged (connector props committed).
    pub hdr_active_outputs: std::collections::HashSet<String>,
    /// Outputs awaiting "keep HDR" confirmation → their auto-revert deadline.
    pub hdr_pending_confirm: std::collections::HashMap<String, std::time::Instant>,
    /// Monitors the user manually switched off. Their DRM output is torn down
    /// (no dead pointer/window zone) but the connector stays plugged in, so we
    /// stash what we need to rebuild it on re-enable. Keyed by output name.
    pub disabled_outputs: std::collections::HashMap<String, crate::output_toggle::DisabledOutput>,
    /// Set to the output name while it is being deliberately re-enabled, so the
    /// startup "honor persisted-off" reconcile inside `connector_connected`
    /// doesn't immediately tear the freshly-rebuilt output back down.
    pub enabling_output: Option<String>,
    pub gesture: GestureState,

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

    // Lantern layer (hold-to-activate WASD→arrows etc. for 60% keyboards).
    // `layer` is reloaded from config on mtime change; `layer_held` tracks the
    // momentary held state of the trigger key. See `input::layer`.
    pub layer: crate::input::layer::LanternLayer,
    pub layer_held: bool,
    /// Injections to forward to the focused client *after* the filter returns
    /// (we can't re-enter the keyboard handle from inside the filter). Each is
    /// (target_keycode, key_state). Usually one entry, but releasing the
    /// trigger while source keys are still held emits several releases at once.
    pub layer_inject: Vec<(smithay::input::keyboard::Keycode, smithay::backend::input::KeyState)>,
    /// Source-keycode → injected target-keycode for keys currently held down
    /// under the layer. Lets us emit the matching *release* even if the trigger
    /// was let go first — otherwise the client never sees key-up and autorepeats
    /// forever. Keyed by the physical keycode so it's stable across layer drops.
    pub layer_active_keys: std::collections::HashMap<u32, smithay::input::keyboard::Keycode>,
    /// mtime of lantern.toml at last layer reload, so we hot-reload the layer
    /// map when the user edits keybinds in System Settings.
    pub layer_config_mtime: Option<std::time::SystemTime>,

    // Cached exclusive zone offsets — reconfigure maximized windows when these change
    pub last_exclusive_offsets: (i32, i32, i32, i32),

    // Server-side decorations
    pub ssd: SsdManager,

    // Input settings (read from lantern.toml)
    pub mouse_speed: f64,
    pub scroll_speed: f64,
    pub pointer_acceleration: bool,
    pub cursor_theme_name: String,
    pub input_config_counter: u32,
    /// Tracked libinput devices, used to re-apply config on live setting changes.
    pub libinput_devices: Vec<smithay::reexports::input::Device>,

    /// Idle/battery/power-action manager (reads [power] settings).
    pub power: crate::power::PowerState,

    /// WM border width in logical pixels (0 = no border).
    pub border_width: u32,

    // Hover preview (bar → compositor IPC for window thumbnails)
    pub hover_preview: crate::hover_preview::HoverPreview,

    // Command Center thumbnails (CC → compositor IPC for in-tile thumbs)
    pub cc_thumbs: crate::cc_thumbs::CcThumbnails,

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
    // ── Construction & Wayland socket ───────────────────────────────────

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
        let data_control_state = WlrDataControlState::new::<Self, _>(
            &dh,
            None,
            |client| crate::security::is_trusted_client(client),
        );
        let ext_data_control_state = ExtDataControlState::new::<Self, _>(
            &dh,
            None,
            |client| crate::security::is_trusted_client(client),
        );
        let clipboard_manager = crate::clipboard_manager::ClipboardManager::new();
        let clipboard_ipc = crate::clipboard_ipc::ClipboardIpc::new();
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new_with_filter::<Self, _>(
            &dh,
            |client| crate::security::is_trusted_client(client),
        );
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let screencopy_state = ScreencopyManagerState::new(&dh);
        let foreign_toplevel_state = ForeignToplevelManagerState::new(&dh);
        let output_management_state = OutputManagementState::new(&dh);
        let xdg_foreign_state = XdgForeignState::new(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<Self>(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let text_input_manager_state = TextInputManagerState::new::<Self>(&dh);
        // clk_id 1 = CLOCK_MONOTONIC (libc::CLOCK_MONOTONIC)
        let presentation_state = PresentationState::new::<Self>(&dh, libc::CLOCK_MONOTONIC as u32);
        // Only the trusted lockscreen binary (~/.lantern/bin/lntrn-lockscreen)
        // may bind the session-lock manager — same allowlist as layer-shell.
        let session_lock_state = SessionLockManagerState::new::<Self, _>(
            &dh,
            |client| crate::security::is_trusted_client(client),
        );
        let xwayland_shell_state = smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "lantern");
        let keyboard = seat.add_keyboard(Default::default(), 200, 25).unwrap();
        // Default NumLock ON at session start. The Razer Naga side grid (and any
        // real numpad) emit keypad keysyms; with NumLock off those decode to
        // navigation keys (KP1=End, KP5=Begin/dead), so games see VK_END instead
        // of VK_NUMPAD1. Locking NumLock on makes them produce digits. The user
        // can still toggle it off with the physical key.
        let mut mods = keyboard.modifier_state();
        mods.num_lock = true;
        keyboard.set_modifier_state(mods);
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
            ext_data_control_state,
            clipboard_manager,
            clipboard_ipc,
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
            pointer_constraints_state,
            relative_pointer_state,
            text_input_manager_state,
            presentation_state,
            session_lock_state,
            session_lock: None,
            popups,
            seat,
            cursor: CursorState::new(&crate::input::read_input_setting("cursor_theme", "default")),
            udev: None,
            winit_redraw_requested: Arc::new(AtomicBool::new(false)),
            pending_client_frame_callbacks: false,
            last_pointer_render_location: None,
            last_pointer_under: None,
            last_callback_render: std::time::Instant::now() - std::time::Duration::from_secs(60),
            debug_counters: DebugCounters::from_env(),
            focused_surface: None,
            window_mru: Vec::new(),
            window_spawn_order: Vec::new(),
            minimized_windows: Vec::new(),
            maximized_windows: Vec::new(),
            solo_tiled_windows: Vec::new(),
            posed_windows: HashMap::new(),
            pending_workspace_moves: Vec::new(),
            fullscreen_windows: Vec::new(),
            alt_tab_switcher: AltTabSwitcher::new(),
            switcher_icons: crate::switcher::icons::SwitcherIconCache::new(96),
            wallpaper: WallpaperState::load_from_config(),
            wallpaper_frame_counter: 0,
            layer_surfaces: Vec::new(),
            layer_surface_outputs: HashMap::new(),
            layer_surface_namespaces: HashMap::new(),
            screencopy_offscreen: HashMap::new(),
            screencopy_pbos: HashMap::new(),
            system_bg_opacity: crate::read_config_f32("background_opacity", 1.0),
            blur_exclude: crate::read_config_list("windows", "blur_exclude"),
            blur_intensity: crate::read_config_f32("blur_intensity", 0.8),
            blur_tint: crate::read_config_f32("blur_tint", 0.15),
            blur_darken: crate::read_config_f32("blur_darken", 0.0),
            window_chrome_cache: HashMap::new(),
            default_window_size: crate::default_window_size(),
            window_rules: crate::read_window_rules(),
            window_zoom: HashMap::new(),
            focus_glow: crate::read_config("window_manager", "focus_glow", "true") == "true",
            focus_glow_color: crate::parse_glow_color(&crate::read_config("window_manager", "focus_glow_color", "#4A9EFF")),
            border_color: crate::parse_glow_color(&crate::read_config("window_manager", "border_color", "#4A9EFF")),
            blur_tint_color: crate::parse_glow_color(&crate::read_config("windows", "blur_tint_color", "#4A9EFF")),
            focus_glow_intensity: crate::read_config("window_manager", "focus_glow_intensity", "0.2")
                .parse::<f32>().unwrap_or(0.2).clamp(0.0, 0.6),
            focus_follows_mouse: crate::read_config("window_manager", "focus_follows_mouse", "false") == "true",
            gaming_mode: false,
            super_pressed: false,
            super_clean_tap: false,
            super_press_time: None,
            snapped_windows: Vec::new(),
            animations: AnimationState::new(),
            closing_windows: Vec::new(),
            window_snapshots: HashMap::new(),
            workspaces: PerOutputWorkspaces::new(),
            window_state_anim: WindowStateAnimState::new(),
            minimize_anim: MinimizeAnimState::new(),
            workspace_anim: WorkspaceAnimState::new(),
            workspace_ipc: WorkspaceIpc::new(),
            hdr_ipc: crate::hdr_ipc::HdrIpc::new(),
            gaming_ipc: crate::gaming_ipc::GamingIpc::new(),
            window_query_ipc: crate::window_query_ipc::WindowQueryIpc::new(),
            hdr_active_outputs: std::collections::HashSet::new(),
            hdr_pending_confirm: std::collections::HashMap::new(),
            disabled_outputs: std::collections::HashMap::new(),
            enabling_output: None,
            gesture: GestureState::new(),
            scratchpad_surface: None,
            scratchpad_pending: false,
            hot_corner: HotCornerState::new(),
            show_desktop_active: false,
            xdg_foreign_state,
            audio_repeat: None,
            layer: crate::input::layer::LanternLayer::load(),
            layer_held: false,
            layer_inject: Vec::new(),
            layer_active_keys: std::collections::HashMap::new(),
            layer_config_mtime: None,
            last_exclusive_offsets: (0, 0, 0, 0),
            ssd: SsdManager::new(),
            mouse_speed: crate::input::read_input_setting_f64("mouse_speed", 0.0),
            scroll_speed: crate::input::read_input_setting_f64("scroll_speed", 1.0),
            pointer_acceleration: crate::input::read_input_setting("pointer_acceleration", "true") == "true",
            cursor_theme_name: crate::input::read_input_setting("cursor_theme", "default"),
            input_config_counter: 0,
            libinput_devices: Vec::new(),
            power: crate::power::PowerState::new(),
            border_width: crate::read_config("window_manager", "border_width", "0")
                .parse::<u32>().unwrap_or(0).clamp(0, 10),
            hover_preview: crate::hover_preview::HoverPreview::new(),
            cc_thumbs: crate::cc_thumbs::CcThumbnails::new(),
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
                if state.debug_counters.enabled {
                    state.debug_counters.listener_fires += 1;
                }
                // Compute trust BEFORE handing the stream to Wayland — we
                // want the SO_PEERCRED read to capture the connecting
                // process at its connect-time identity, not whatever it
                // execs into later.
                let is_trusted = crate::security::compute_trust_at_connect(&client_stream);
                let client_state = ClientState {
                    is_trusted,
                    ..ClientState::default()
                };
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(client_state))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Drain to completion: dispatch_clients returns the number
                    // of events it dispatched. Loop until 0 so the underlying
                    // epoll fd transitions to "not readable" — otherwise level
                    // mode keeps re-firing this callback and other event
                    // sources (input, render timer) get starved into freeze.
                    // Cap the loop to prevent runaway in case a client spams
                    // events faster than we can dispatch.
                    let mut iters = 0;
                    if state.debug_counters.enabled {
                        state.debug_counters.wayland_fires += 1;
                    }
                    let dispatch_start = if state.debug_counters.enabled {
                        Some(std::time::Instant::now())
                    } else { None };
                    loop {
                        let n = match unsafe { display.get_mut().dispatch_clients(state) } {
                            Ok(n) => n,
                            Err(e) => {
                                tracing::error!("dispatch_clients failed: {:?}", e);
                                break;
                            }
                        };
                        iters += 1;
                        if state.debug_counters.enabled {
                            state.debug_counters.dispatch_iters += 1;
                            state.debug_counters.dispatch_events += n as u64;
                        }
                        if n == 0 || iters >= 64 {
                            break;
                        }
                    }
                    if let Some(t) = dispatch_start {
                        state.debug_counters.dispatch_micros += t.elapsed().as_micros() as u64;
                    }
                    state.debug_counters.maybe_report();
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    // ── Surface hit-testing ─────────────────────────────────────────────

    /// True if `window` is currently visible for input: it's on the active
    /// workspace of its output, or it isn't workspace-tracked at all.
    /// Override-redirect X11 popups (Steam menus, dropdowns) and the
    /// scratchpad live only in the global `self.space` and float above every
    /// workspace — `window_workspace` returns `None` for them, so they stay
    /// hittable. Windows on an inactive workspace stay mapped in the global
    /// Space but must NOT be hit (they're not on screen).
    pub fn window_is_visible(&self, window: &Window) -> bool {
        match crate::window_ext::WindowExt::get_wl_surface(window)
            .and_then(|s| self.workspaces.window_workspace_ref(&s).map(|(o, w)| (o, w)))
        {
            Some((out, ws)) => ws == self.workspaces.active_id(out),
            None => true,
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::wlr_layer::Layer;

        // Check layer surfaces first (Top/Overlay are above windows)
        // Use the output the pointer is on for layer surface positioning
        // Skip if a fullscreen window covers this output — fullscreen takes priority
        if let Some(output) = self.output_at_point(pos) {
            let output_has_fullscreen = self.fullscreen_windows.iter().any(|fw| {
                self.find_mapped_window(&fw.surface)
                    .and_then(|w| self.output_for_window(&w))
                    .map_or(false, |o| o == output)
            });
            let output_geo = self.workspaces.output_geometry(&output).unwrap_or_default();
            // Iterate Top/Overlay layer surfaces newest-first so the most
            // recently created surface (e.g. lntrn-screenshot opening while
            // CC is up) receives pointer events instead of an older surface
            // on the same layer eating the click. Same-layer stacking order
            // is implementation-defined per the layer-shell spec.
            for ls in self.layer_surfaces.iter().rev() {
                if output_has_fullscreen { break; }
                if !ls.alive() {
                    continue;
                }
                if !self.layer_surface_on_output(ls, &output) {
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
                let size = crate::layer_position::layer_surface_effective_size(&cached, output_geo);
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

        // The global `self.space` keeps EVERY window mapped, including those
        // on inactive workspaces (`switch_workspace_on` never unmaps — it just
        // flips which per-workspace Space renders). So we can't hit-test the
        // space naively: a right-click on an empty desktop in workspace 2 would
        // otherwise land on a window still mapped from workspace 1. Walk the
        // global z-order top→bottom and skip anything not currently visible,
        // so an inactive-workspace window never eats the click but the active
        // window beneath it (and floating OR popups / the scratchpad) still do.
        //
        // For XWayland fullscreen games at sub-native resolution, XWayland
        // attaches a wp_viewport (src = game's small buffer, dst = full output)
        // that scales BOTH the rendered image AND the pointer coordinates it
        // forwards to the X11 client. Smithay's surface_under already maps
        // through the viewport dst, so we pass the raw logical position and do
        // NO manual stretch — an extra inverse-transform here would double-scale
        // and desync the game's cursor.
        let window_hit = self.space
            .elements()
            .rev()
            .filter(|w| self.window_is_visible(w))
            .find_map(|window| {
                // Use the RENDER location (buffer top-left), not the mapped
                // location: `surface_under` walks the surface tree from the
                // buffer origin. For CSD windows with shadow margins (Firefox)
                // `geometry().loc` is non-zero, so the two differ — using the
                // raw mapped location shifts the reported pointer up-and-left
                // by the shadow inset. `Space::element_under` subtracts the
                // same offset internally; we replicate it here.
                let location = self.space.element_location(window)? - window.geometry().loc;
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            });
        if window_hit.is_some() {
            return window_hit;
        }

        // Check Bottom layer surfaces (below windows, above wallpaper)
        if let Some(output) = self.output_at_point(pos) {
            let output_geo = self.workspaces.output_geometry(&output).unwrap_or_default();
            for ls in &self.layer_surfaces {
                if !ls.alive() { continue; }
                if !self.layer_surface_on_output(ls, &output) { continue; }
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

    /// True if a Top/Overlay layer surface (command center, screenshot UI,
    /// …) currently accepts pointer input at `pos`. Window-level pointer
    /// grabs — the outer resize zone, SSD decoration clicks, Super+drag —
    /// must defer to it; otherwise a click meant for the overlay falls
    /// through to the window (or its 8px resize border) beneath it.
    ///
    /// Respects the surface's input region (via `under_from_surface_tree`),
    /// so a fullscreen overlay that's currently click-through — empty input
    /// region, e.g. the CC while hidden — correctly reports `false`.
    pub fn pointer_over_top_layer(&self, pos: Point<f64, Logical>) -> bool {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::wlr_layer::Layer;

        let Some(output) = self.output_at_point(pos) else { return false; };
        // A fullscreen window suppresses layer input on its output (mirrors
        // surface_under), so don't block window grabs behind it.
        let output_has_fullscreen = self.fullscreen_windows.iter().any(|fw| {
            self.find_mapped_window(&fw.surface)
                .and_then(|w| self.output_for_window(&w))
                .map_or(false, |o| o == output)
        });
        if output_has_fullscreen {
            return false;
        }
        let output_geo = self.workspaces.output_geometry(&output).unwrap_or_default();
        for ls in self.layer_surfaces.iter().rev() {
            if !ls.alive() {
                continue;
            }
            if !self.layer_surface_on_output(ls, &output) {
                continue;
            }
            let cached = with_states(ls.wl_surface(), |states| {
                *states.cached_state.get::<LayerSurfaceCachedState>().current()
            });
            if cached.layer != Layer::Top && cached.layer != Layer::Overlay {
                continue;
            }
            let ls_loc = crate::render::layer_surface_position_logical(&cached, output_geo);
            let size = crate::layer_position::layer_surface_effective_size(&cached, output_geo);
            let rect = Rectangle::new(ls_loc, size);
            let pos_i = Point::from((pos.x as i32, pos.y as i32));
            if !rect.contains(pos_i) {
                continue;
            }
            let relative = pos - ls_loc.to_f64();
            if smithay::desktop::utils::under_from_surface_tree(
                ls.wl_surface(),
                relative,
                (0, 0),
                WindowSurfaceType::ALL,
            )
            .is_some()
            {
                return true;
            }
        }
        false
    }

    // ── Render scheduling & frame bookkeeping ───────────────────────────

    pub fn request_winit_redraw(&self) {
        self.winit_redraw_requested.store(true, Ordering::Release);
        self.loop_signal.wakeup();
    }

    pub fn take_winit_redraw_request(&self) -> bool {
        self.winit_redraw_requested.swap(false, Ordering::AcqRel)
    }

    /// Whether the session is currently locked (ext-session-lock-v1 active).
    /// True from the moment a lock is requested until `unlock_and_destroy`.
    pub fn is_locked(&self) -> bool {
        self.session_lock.is_some()
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

    /// Commit-path render scheduling: only repaint the output that owns the
    /// committed surface. A 240fps game on the primary shouldn't drag the
    /// secondary monitor through a render pass per commit. Surfaces with no
    /// workspace assignment (OR popups, scratchpad, layer surfaces) fall
    /// back to all outputs.
    pub fn schedule_client_render_for_surface(&mut self, surface: &WlSurface) {
        self.pending_client_frame_callbacks = true;
        if self.udev.is_none() {
            self.request_winit_redraw();
            return;
        }
        let output = self
            .workspaces
            .window_workspace_ref(surface)
            .and_then(|(name, _)| self.workspaces.output_by_name(name))
            .cloned();
        match output {
            Some(o) => crate::udev::schedule_render_output(self, &o),
            None => crate::udev::schedule_render_all(self),
        }
    }

    /// Pointer-path render scheduling: repaint the output under the cursor,
    /// plus the previous output when the cursor just crossed a monitor seam
    /// (it must repaint once to erase the cursor it's still showing).
    pub fn schedule_render_pointer(
        &mut self,
        prev_loc: Point<f64, Logical>,
        pos: Point<f64, Logical>,
    ) {
        if self.udev.is_none() {
            self.request_winit_redraw();
            return;
        }
        let cur = self.output_at_point(pos);
        match &cur {
            Some(o) => {
                let o = o.clone();
                crate::udev::schedule_render_output(self, &o);
            }
            None => crate::udev::schedule_render_all(self),
        }
        if let Some(prev) = self.output_at_point(prev_loc) {
            // Output PartialEq is Arc identity; both handles come from the
            // same workspaces registry, so this is a reliable comparison.
            if cur.as_ref() != Some(&prev) {
                crate::udev::schedule_render_output(self, &prev);
            }
        }
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

    // ── Output & exclusive-zone geometry ────────────────────────────────

    /// Find the output whose geometry contains `point`.
    /// Falls back to the closest output if the point is between monitors.
    pub fn output_at_point(&self, point: Point<f64, Logical>) -> Option<Output> {
        // Exact containment check
        for output in self.workspaces.outputs_iter() {
            if let Some(geo) = self.workspaces.output_geometry(output) {
                if geo.to_f64().contains(point) {
                    return Some(output.clone());
                }
            }
        }
        // Fallback: closest output center. Skip outputs that workspaces
        // doesn't know about (mid-removal, etc.) instead of panicking.
        self.space
            .outputs()
            .filter_map(|o| self.workspaces.output_geometry(o).map(|g| (o, g)))
            .min_by_key(|(_, geo)| {
                let cx = geo.loc.x + geo.size.w / 2;
                let cy = geo.loc.y + geo.size.h / 2;
                let dx = point.x - cx as f64;
                let dy = point.y - cy as f64;
                (dx * dx + dy * dy) as i64
            })
            .map(|(o, _)| o.clone())
    }

    /// Find the output a window lives on by checking which output contains its
    /// top-left. Center-based detection misclassifies oversized windows whose
    /// centers land on the wrong monitor — when this misclassifies, render-path
    /// consumers like per-output frame-callback pacing drop the window from the
    /// rendering output's surface set and the client stops getting frame
    /// callbacks (stutter).
    pub fn output_for_window(&self, window: &Window) -> Option<Output> {
        let loc = self.workspaces.element_location(window)?;
        self.output_at_point(Point::from((loc.x as f64, loc.y as f64)))
    }

    /// Combined bounding box of all outputs.
    pub fn total_output_bounds(&self) -> Rectangle<i32, Logical> {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for output in self.workspaces.outputs_iter() {
            if let Some(geo) = self.workspaces.output_geometry(output) {
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

    /// Whether the given layer surface should render / receive input on
    /// this output. Surfaces with no recorded assignment fall through to
    /// "render everywhere" so a surface that slipped past the handler is
    /// still visible. Match by name — Output's PartialEq is Arc::ptr_eq,
    /// which can mis-fire if the two Outputs originated from different
    /// code paths even when they refer to the same physical connector.
    pub fn layer_surface_on_output(
        &self,
        ls: &smithay::wayland::shell::wlr_layer::LayerSurface,
        output: &Output,
    ) -> bool {
        match self.layer_surface_outputs.get(ls.wl_surface()) {
            Some(assigned) => assigned.name() == output.name(),
            None => true,
        }
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
    /// Whether this client is allowed to bind privileged protocols
    /// (clipboard, screencopy, foreign-toplevel, layer-shell). Computed
    /// once at connect time from `/proc/<pid>/exe` via `SO_PEERCRED`.
    /// See `crate::security::compute_trust_at_connect`.
    pub is_trusted: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        // Wake the main loop so the clipboard manager can recheck whether
        // the disconnecting client owned the active selection.
        if let Some(ping) = crate::clipboard_manager::RECHECK_PING.get() {
            ping.ping();
        }
    }
}

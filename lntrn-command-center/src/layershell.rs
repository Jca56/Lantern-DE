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

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::time::Duration;

use wayland_client::backend::ObjectId;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    backend::WaylandError,
    protocol::{wl_compositor, wl_seat},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::toplevel::ToplevelTracker;

use crate::app::{AppState, PanelRect};
use crate::ipc::{self, Cmd};
use crate::launcher::icons::IconCache;

mod click;
mod dispatch;
mod drag;
mod hover;
mod input;
mod render_tick;
mod right_click;
mod util;
mod view_click;
use click::handle_clicks;
use drag::{handle_drag, handle_terminal_selection};
use hover::track_hovers;
use input::{apply_key_autorepeat, handle_keypress, handle_scroll};
use render_tick::render_frame;
use right_click::handle_right_click;
use util::{commit_transparent, files_strip_rect, set_active_input, sort_menu_items};
#[allow(unused_imports)]
use view_click::handle_control_view_click;

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

/// Physical data for one wl_output. Tracked per-output (keyed by proxy id)
/// so a hotplugged monitor's Mode/Scale events can't clobber the scale of
/// the output we actually render on — that bug shrank the panel to ~0.7×
/// whenever a 1080p secondary was plugged in next to the scaled 4K primary.
#[derive(Default, Clone)]
struct OutputData {
    /// wl_registry global name, so GlobalRemove can evict the right entry.
    registry_name: u32,
    /// Connector name (wl_output::Event::Name, e.g. "DP-1"). This is the
    /// key the compositor's workspace IPC uses, so we need it to look up
    /// the active workspace for the output our surface is actually on.
    name: String,
    /// Physical mode width in pixels (wl_output::Event::Mode).
    phys_width: u32,
    /// Integer scale (wl_output::Event::Scale).
    scale: i32,
}

struct WlState {
    running: bool,
    configured: bool,
    /// The compositor has consumed our last commit (frame callback
    /// fired). Together with `input_dirty` this paces rendering to the
    /// output's refresh rate instead of the raw input-event rate.
    frame_done: bool,
    /// Something happened that needs a redraw: pointer / keyboard
    /// input, a configure, a synthesized key repeat, a freshly loaded
    /// icon. Cleared when a frame is rendered.
    input_dirty: bool,
    width: u32,
    height: u32,
    /// All advertised outputs, keyed by wl_output proxy id.
    outputs: HashMap<ObjectId, OutputData>,
    /// The output our layer surface is on (from wl_surface::Enter).
    current_output: Option<ObjectId>,
    /// Integer-scale fallback used only before the first Enter arrives.
    fallback_scale: i32,
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
    /// Caps Lock toggle. Reported in `mods_locked` (not depressed). When
    /// on, letter keycodes should be treated as if Shift were held too.
    caps_lock: bool,
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
            input_dirty: true,
            width: 0,
            height: 0,
            outputs: HashMap::new(),
            current_output: None,
            fallback_scale: 1,
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
            caps_lock: false,
            held_key: None,
            last_repeat: None,
            pending_key: None,
            scroll_delta_v: 0.0,
            toplevels: ToplevelTracker::new(),
            seat: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        // Scale must come from the output our surface is actually on, never
        // from whichever output spoke most recently. Before the first
        // wl_surface::Enter arrives, a lone output is unambiguous.
        let current = self
            .current_output
            .as_ref()
            .and_then(|id| self.outputs.get(id));
        let single = if self.outputs.len() == 1 {
            self.outputs.values().next()
        } else {
            None
        };
        if let Some(data) = current.or(single) {
            if data.phys_width > 0 && self.width > 0 {
                return data.phys_width as f64 / self.width as f64;
            }
            if data.scale > 0 {
                return data.scale as f64;
            }
        }
        self.fallback_scale.max(1) as f64
    }

    /// Connector name of the output our surface is on, used to look up the
    /// per-output active workspace. Same resolution rule as
    /// `fractional_scale`: prefer the output from wl_surface::Enter, and
    /// before that arrives fall back only when a single output is
    /// unambiguous. Returns `None` (rather than guessing) on multi-monitor
    /// before Enter, so we never show a stale workspace from the wrong
    /// monitor — the tile just waits a frame.
    fn current_output_name(&self) -> Option<&str> {
        let current = self
            .current_output
            .as_ref()
            .and_then(|id| self.outputs.get(id));
        let single = if self.outputs.len() == 1 {
            self.outputs.values().next()
        } else {
            None
        };
        current
            .or(single)
            .map(|d| d.name.as_str())
            .filter(|n| !n.is_empty())
    }

    fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Poll timeout while the panel is visible but nothing is animating —
/// ~20Hz so worker-pushed state (audio / wifi / bluetooth events) and
/// hover tracking stay responsive even with zero input.
const IDLE_TICK: Duration = Duration::from_millis(50);

/// Poll timeout while the panel is hidden. The poll also watches the
/// IPC fd, so a Super-tap wakes us instantly regardless of this value;
/// it only bounds how quickly we notice worker-side wake-ups (an
/// incoming Bluetooth file / pair request) and pump the hidden
/// terminal's PTY. 4 Hz instead of the old 20 Hz sleep loop.
const HIDDEN_TICK: Duration = Duration::from_millis(250);

/// How long we wait for the compositor's frame callback before treating
/// it as lost and rendering anyway. Callbacks normally arrive within
/// one refresh interval (≤ 17 ms at 60 Hz); if the surface is not being
/// painted (covered, unmapped mid-transition) we still want input to
/// produce a frame promptly rather than after the 500 ms fallback.
const CALLBACK_GRACE: Duration = Duration::from_millis(50);

/// Safety-net upper bound on the active-path poll. When something is
/// animating we expect frame callbacks at refresh rate, so this only
/// matters if the compositor stops paining our surface (e.g., another
/// fullscreen surface covers us). Without this cap the daemon would
/// block forever in poll() — with it we wake every second to re-check
/// `is_animating()` and subsystem state.
const ACTIVE_POLL_CAP: Duration = Duration::from_secs(1);

/// Worst-case interval between renders while the panel is visible.
/// Even when nothing is animating and no input has arrived, we force
/// a re-render after this long so subsystem state that updates on a
/// timer (clock minute roll-over, sysmon sparklines sampled at 2 Hz,
/// battery percentage, …) doesn't visibly freeze. 500 ms lines up
/// with sysmon's sample period and is well below the minute boundary.
const FALLBACK_REDRAW_INTERVAL: Duration = Duration::from_millis(500);

/// Drain the wayland queue with a timeout, watching both the wayland
/// socket and our IPC listener so an IPC command (Toggle / Show / Hide)
/// sent while the panel is settled-and-visible still wakes the loop
/// promptly.
///
/// Behaviour mirrors `EventQueue::blocking_dispatch` with two changes:
///
/// 1. We use `prepare_read` + `libc::poll` instead of the built-in
///    blocking read, which lets us specify a timeout AND poll a second
///    fd at the same time.
/// 2. Returning `Ok(())` on timeout / EINTR / IPC-only wake is fine —
///    the outer loop will run its tick / IPC drain logic and try
///    again next iteration.
///
/// Per `ReadEventsGuard` docs: the guard MUST be created before
/// polling the socket, otherwise events arriving between prepare and
/// poll would be lost. We honour that here — guard is held for the
/// duration of the poll and either consumed via `read()` (when wayland
/// has data) or dropped (when only the IPC fd fired or we timed out).
fn dispatch_with_timeout(
    event_queue: &mut EventQueue<WlState>,
    ipc_fd: std::os::fd::RawFd,
    state: &mut WlState,
    timeout: Option<Duration>,
) -> Result<()> {
    // Flush any pending requests so the server can react before we
    // potentially block. Then dispatch anything already in memory —
    // nothing to wait on if the queue is non-empty.
    event_queue.flush()?;
    if event_queue.dispatch_pending(state)? > 0 {
        return Ok(());
    }

    // Stake our claim on the next socket read. If another caller is
    // already mid-read (multi-threaded use), prepare_read returns None;
    // in that case the events will land in our queue shortly, so just
    // dispatch what's there and bail.
    let guard = match event_queue.prepare_read() {
        Some(g) => g,
        None => {
            event_queue.dispatch_pending(state)?;
            return Ok(());
        }
    };

    let timeout_ms = timeout
        .map(|d| d.as_millis().min(i32::MAX as u128 - 1) as i32)
        .unwrap_or(-1);

    let mut fds = [
        libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: ipc_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];

    let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };

    if ret < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err.into());
        }
        // EINTR: drop the guard so the next iteration can re-prepare.
        drop(guard);
        return Ok(());
    }

    if fds[0].revents & libc::POLLIN != 0 {
        // Wayland fd has data — actually read it into our queue.
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(io)) if io.kind() == std::io::ErrorKind::WouldBlock => {
                // Spurious wakeup: poll reported ready but the socket
                // had nothing for us once we attempted the read.
            }
            Err(e) => return Err(e.into()),
        }
        event_queue.dispatch_pending(state)?;
    } else {
        // Wayland fd silent — either timed out or the IPC fd fired.
        // Dropping the guard cancels the prepared read so the next
        // iteration's prepare_read can succeed.
        drop(guard);
    }

    Ok(())
}

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
        layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
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

    tracing::info!(
        w = wl.width,
        h = wl.height,
        "command-center overlay configured"
    );

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

    // Raw fd for the IPC listener — handed to `dispatch_with_timeout`
    // each iteration so a Toggle / Show / Hide sent while the loop is
    // parked in poll() wakes us. The listener outlives the loop, so
    // the fd is valid for the entire run.
    let ipc_fd = sock.as_raw_fd();
    // Wall-clock of the last completed render. Used to guarantee a
    // fallback redraw every FALLBACK_REDRAW_INTERVAL while visible so
    // timer-driven UI (clock minutes, sysmon graphs, battery %) stays
    // current even when the user isn't touching the panel. It doubles
    // as the reference for CALLBACK_GRACE: every render commits with a
    // frame request, so "rendered long ago and still no callback"
    // means the callback is not coming.
    let mut last_render = std::time::Instant::now();

    if initial_visible {
        app.open();
        set_active_input(&surface, &layer_surface, &empty_region, true);
        input_active = true;
    }

    tracing::info!(initial_visible, "command-center daemon ready");

    while wl.running {
        // Drain any queued IPC commands and apply them.
        let ipc_cmd = ipc::drain(&sock);
        if let Some(cmd) = ipc_cmd {
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
            // A frame callback requested just before we hid may never
            // have fired (the surface was unmapped) — don't let that
            // stale "waiting" state gate the first frame of the open.
            wl.frame_done = true;
            wl.input_dirty = true;
            match cmd {
                Cmd::Toggle => app.toggle(),
                Cmd::Show => app.open(),
                Cmd::Hide => app.close(),
            }
        }

        // Let the worker threads know whether anything is on screen so
        // they can drop to their slow hidden cadence (or burst-poll on
        // show). Opening / Closing count as visible.
        crate::panel_visible::set(!app.is_hidden());

        // Refresh the toplevel snapshot for the renderer — only when the
        // tracker saw a change, not on every iteration.
        if wl.toplevels.take_dirty() {
            app.toplevels = wl.toplevels.toplevels();
            wl.input_dirty = true;
        }

        // Swap in a freshly rescanned .desktop set if the background
        // rescan kicked off by the last open has landed (and nothing on
        // screen is holding indices into the old set).
        app.poll_apps_rescan();

        // Dispatch any pending window actions queued by click handlers.
        if !app.window_actions.is_empty() {
            for act in app.window_actions.drain(..) {
                use crate::app::WindowActionKind;
                match act.kind {
                    WindowActionKind::Activate => {
                        if let Some(seat) = wl.seat.as_ref() {
                            wl.toplevels
                                .activate(&act.app_id, &act.title, act.instance, seat);
                        }
                    }
                    WindowActionKind::Close => {
                        wl.toplevels.close(&act.app_id, &act.title, act.instance);
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
        // expect frame callbacks → the poll wakes promptly. When hidden
        // we park in the same poll with a longer timeout.
        if app.is_hidden() {
            // Tell the sysmon worker to stop walking /proc. This is the
            // only place `tick(false)` can run: the active path below is
            // never reached while hidden, and the Closing → Hidden
            // transition `continue`s before it too. (The worker used to
            // keep scanning every process — and spawning `nvidia-smi`
            // every 1.5 s — forever after the first open.)
            app.controls.sysmon.tick(false);

            // Park on the wayland fd + IPC fd. `dispatch_pending` alone
            // never reads the socket, so the old sleep loop left every
            // event the compositor sent while we were hidden sitting
            // unread in the kernel buffer — and, worse, could not notice
            // a dead connection (compositor restart) until the next
            // Super-tap tried to commit, which then killed the daemon
            // and ate that tap. Reading here means a dead compositor
            // ends the daemon within one tick, so the very next tap
            // starts a fresh one that actually opens.
            dispatch_with_timeout(&mut event_queue, ipc_fd, &mut wl, Some(HIDDEN_TICK))?;

            // While hidden, still tick the bluetooth control so an
            // incoming-file request can wake the panel and switch us
            // into the BT view. Other controls don't need the wake-up
            // path so we keep this cheap and BT-specific.
            app.controls.bluetooth.tick();
            if app.controls.bluetooth.incoming_request.is_some()
                || app.controls.bluetooth.pair_request.is_some()
            {
                tracing::info!("incoming BT file/pair → auto-opening panel to BT view");
                app.mode = crate::app::PanelMode::Control(crate::controls::TileId::Bluetooth);
                app.open();
                continue;
            }

            // Keep the terminal grid live while the panel is hidden.
            // The PTY reader thread is always pulling bytes into its
            // channel — pumping them through the VTE here means
            // long-running commands (e.g. `yay -Syu`) stay current and
            // we don't flood the grid on next open.
            app.terminal.pump();
            continue;
        }

        // Active path: poll wayland AND ipc together. We pick the
        // timeout from the current motion state — when something is
        // animating we expect frame callbacks at refresh rate, so a
        // 1s safety cap is fine; when nothing is in motion we cap at
        // IDLE_TICK so subsystem state pushed by worker threads
        // (audio / wifi / bluetooth incoming, etc.) gets a chance to
        // tick at ~20Hz even with zero user input. Without the IPC fd
        // in the poll set the daemon would sit on `blocking_dispatch`
        // and miss any `--toggle` sent during idle-visible.
        let poll_timeout = if ipc_cmd.is_some() {
            // An IPC command just mutated visibility/animation state.
            // A steady panel hasn't committed a frame recently, so no
            // frame callback is pending to wake the poll — blocking
            // here would stall the first frame of whatever animation
            // the command started (Super-toggle collapse-then-close sat
            // frozen for the full ACTIVE_POLL_CAP). Fall straight
            // through and render now; that commit restarts the
            // frame-callback chain.
            Some(Duration::ZERO)
        } else if app.is_animating() {
            Some(ACTIVE_POLL_CAP)
        } else {
            Some(IDLE_TICK)
        };
        dispatch_with_timeout(&mut event_queue, ipc_fd, &mut wl, poll_timeout)?;

        // Tick the animation state machine + control backends (battery
        // sysfs poll, etc.). Both are cheap; rate limiting lives inside
        // each tile's `tick`.
        let was_hidden_before_tick = app.is_hidden();
        if app.tick() {
            wl.input_dirty = true;
        }
        // Drain workspace state pushed by the compositor. Cheap (non-blocking
        // socket); skip while hidden since nothing reads the value then.
        app.workspace_ipc.poll();
        // Drain any pending async export result into flash_text.
        if app.notes.open {
            app.notes.poll_export();
        }
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
        let bt_incoming_before = app.controls.bluetooth.incoming_request.is_some()
            || app.controls.bluetooth.pair_request.is_some();
        // Mirror the cursor position into AppState (in physical px) so
        // the renderer can drive cursor-aware effects (dock magnification
        // wave) without reaching into wayland state.
        {
            let scale_f = wl.fractional_scale() as f32;
            app.cursor_phys = (wl.cursor_x as f32 * scale_f, wl.cursor_y as f32 * scale_f);
            // Sync the split-panel gap once per loop iteration so every
            // hit test + render call below sees the correct offset.
            crate::app::set_split_gap_px(crate::app::effective_split_gap_px(&app, scale_f));
        }
        app.controls.tick();
        app.media.tick();
        // PTY housekeeping for the Terminal view. We spawn lazily on
        // first activation and resize whenever the body geometry
        // changes so the child shell reflows correctly.
        if app.panel_view == crate::app::PanelView::Terminal {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w,
                scale_f,
                app.desired_panel_w_logical(),
                app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            // Single source of truth for cell metrics + grid size so the
            // PTY's wrap column matches what we actually paint.
            let (_, _, _, cols, rows) =
                crate::terminal::body_metrics(panel_rect, top_y, scale_f, app.config.text_size);
            app.terminal.ensure_spawned(cols.max(20), rows.max(5));
        }
        // Drain any pending PTY output into the grid so new bytes
        // appear in the next render (and request one — a scrolling
        // build log shouldn't wait for the 500 ms fallback).
        if app.terminal.pump() {
            wl.input_dirty = true;
        }

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

        // Refresh hover state for every cursor-aware widget in the
        // panel chrome (WiFi rows, power column, view arrows, mini-dock,
        // …) in one pass.
        track_hovers(&mut wl, &mut app);

        let bt_incoming_after = app.controls.bluetooth.incoming_request.is_some()
            || app.controls.bluetooth.pair_request.is_some();
        // Fresh incoming file/pair request → jump straight to the BT view
        // so the inline Accept/Reject isn't hidden behind another view.
        if bt_incoming_after && !bt_incoming_before {
            tracing::info!("incoming BT file/pair while panel visible → switching to BT view");
            app.mode = crate::app::PanelMode::Control(crate::controls::TileId::Bluetooth);
        }

        // Handle Esc → close.
        if wl.esc_pressed {
            wl.esc_pressed = false;
            tracing::debug!(?app.mode, "Esc pressed");
            app.handle_esc();
        }

        // Drain accumulated scroll delta into whichever view is
        // currently scrolling (Wifi list, emoji grid, launcher results,
        // notes editor, terminal scrollback, …).
        handle_scroll(&mut wl, &mut app, &mut text);

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
        apply_key_autorepeat(&mut wl);
        handle_keypress(&mut wl, &mut app, &mut thumbs, &mut text);

        // Terminal body selection (press → drag → release).
        handle_terminal_selection(&mut wl, &mut app);

        // Files-view click: toolbar (controls row) + body (sidebar + list).
        if app.panel_view == crate::app::PanelView::Files
            && wl.left_clicked
            && app.context_menu.is_none()
        {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w,
                scale_f,
                app.desired_panel_w_logical(),
                app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;

            // Toolbar strip in the top-most row takes precedence.
            let strip_hit = files_strip_rect(&app, panel_rect, scale_f)
                .map(|s| crate::files::hit_strip(&app.files, s, scale_f, phys_cx, phys_cy));
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
                            anchor_above: false,
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
                &app.files,
                panel_rect,
                top_y,
                scale_f,
                app.config.text_size,
                phys_cx,
                phys_cy,
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

        // Resolve clicks + pin-drag (left + motion + release).
        handle_clicks(&mut wl, &mut app, &mut text, &mut thumbs);

        // Right-click → open the right context menu for this view.
        handle_right_click(&mut wl, &mut app, &mut text);

        // Drag continuations (sliders + notes editor text drag-select).
        handle_drag(&mut wl, &mut app, &mut text);

        // Upload any icons the background rasterizer finished. A fresh
        // texture is a visual change, so it counts as dirty.
        if icon_cache.pump(&gpu, &tex_pass) > 0 {
            wl.input_dirty = true;
        }

        // Render gate. Two questions:
        //
        //   want: is there anything new to show? Input arrived, an
        //         animation is in flight, an icon finished loading, or
        //         the FALLBACK_REDRAW_INTERVAL timer says timer-driven UI
        //         (clock, sysmon graphs) is due for a refresh.
        //   can:  has the compositor consumed our previous buffer? Every
        //         render commits with a frame request, so `frame_done`
        //         paces us to the output's refresh rate. If the callback
        //         is overdue (surface not being painted) or the fallback
        //         timer fired, render anyway so input never stalls.
        //
        // Both must hold. A steady panel with no input renders nothing
        // even though a callback is sitting satisfied; a 1 kHz mouse
        // sweeping the panel renders once per refresh, not once per
        // motion event.
        let fallback_due = last_render.elapsed() >= FALLBACK_REDRAW_INTERVAL;
        let callback_overdue = !wl.frame_done && last_render.elapsed() >= CALLBACK_GRACE;
        let want = wl.input_dirty || app.is_animating() || fallback_due;
        let can = wl.frame_done || callback_overdue || fallback_due;
        if !(want && can) {
            continue;
        }
        wl.frame_done = false;
        wl.input_dirty = false;
        last_render = std::time::Instant::now();

        let scale_f = wl.fractional_scale() as f32;
        render_frame(
            &mut wl,
            &mut app,
            &mut gpu,
            &surface,
            &viewport,
            &mut painter,
            &mut text,
            &mut mono_text,
            &mut thumbs,
            &mut icon_cache,
            &tex_pass,
            &qh,
            scale_f,
        );
    }

    Ok(())
}

// `handle_control_view_click` → layershell/click.rs
// `files_strip_rect`, `sort_menu_items`, `set_active_input`, `commit_transparent` → layershell/util.rs

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

use std::ffi::c_void;
use std::ptr::NonNull;

use std::os::unix::net::UnixListener;
use std::time::Duration;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
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
use right_click::handle_right_click;
use hover::track_hovers;
use input::{apply_key_autorepeat, handle_keypress, handle_scroll};
use render_tick::render_frame;
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

struct WlState {
    running: bool,
    configured: bool,
    frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
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
            width: 0,
            height: 0,
            scale: 1,
            output_phys_width: 0,
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


// ── Entry point ─────────────────────────────────────────────────────────────

/// Idle tick when the panel is hidden — bound the loop to ~20Hz so we
/// promptly notice IPC commands without burning CPU. When animating
/// or visible we use the wayland frame callback for pacing.
const IDLE_TICK: Duration = Duration::from_millis(50);

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
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::None,
        );
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

    tracing::info!(w = wl.width, h = wl.height, "command-center overlay configured");

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

    if initial_visible {
        app.open();
        set_active_input(&surface, &layer_surface, &empty_region, true);
        input_active = true;
    }

    tracing::info!(initial_visible, "command-center daemon ready");

    while wl.running {
        // Drain any queued IPC commands and apply them.
        if let Some(cmd) = ipc::drain(&sock) {
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
            match cmd {
                Cmd::Toggle => app.toggle(),
                Cmd::Show => app.open(),
                Cmd::Hide => app.close(),
            }
        }

        // Refresh toplevel snapshot for the renderer.
        app.toplevels = wl.toplevels.toplevels();

        // Dispatch any pending window actions queued by click handlers.
        if !app.window_actions.is_empty() {
            for act in app.window_actions.drain(..) {
                use crate::app::WindowActionKind;
                match act.kind {
                    WindowActionKind::Activate => {
                        if let Some(seat) = wl.seat.as_ref() {
                            wl.toplevels.activate(&act.app_id, &act.title, seat);
                        }
                    }
                    WindowActionKind::Close => {
                        wl.toplevels.close(&act.app_id, &act.title);
                    }
                    WindowActionKind::Minimize => {
                        wl.toplevels.set_minimized(&act.app_id, &act.title, true);
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
        // expect frame callbacks → blocking_dispatch wakes promptly. When
        // hidden we use a short idle tick so IPC stays responsive.
        if app.is_hidden() {
            // Non-blocking dispatch: process anything queued, then sleep.
            event_queue.dispatch_pending(&mut wl)?;
            event_queue.flush()?;

            // While hidden, still tick the bluetooth control so an
            // incoming-file request can wake the panel and switch us
            // into the BT view. Other controls don't need the wake-up
            // path so we keep this cheap and BT-specific.
            app.controls.bluetooth.tick();
            if app.controls.bluetooth.incoming_request.is_some() {
                tracing::info!("incoming BT file → auto-opening panel to BT view");
                app.mode = crate::app::PanelMode::Control(
                    crate::controls::TileId::Bluetooth,
                );
                app.open();
                continue;
            }

            // Keep the terminal grid live while the panel is hidden.
            // The PTY reader thread is always pulling bytes into its
            // channel — pumping them through the VTE here means
            // long-running commands (e.g. `yay -Syu`) stay current and
            // we don't flood the grid on next open.
            app.terminal.pump();

            std::thread::sleep(IDLE_TICK);
            continue;
        }

        // Active path: block for an event with a small timeout so we
        // still pick up IPC commands while waiting. Smithay's event_queue
        // doesn't expose a timeout directly, but we can approximate by
        // checking for prepared reads + dispatching pending first.
        event_queue.dispatch_pending(&mut wl)?;
        event_queue.flush()?;
        // Block for the next event; the frame callback usually arrives
        // every ~16ms while visible so this won't stall noticeably.
        event_queue.blocking_dispatch(&mut wl)?;

        // Tick the animation state machine + control backends (battery
        // sysfs poll, etc.). Both are cheap; rate limiting lives inside
        // each tile's `tick`.
        let was_hidden_before_tick = app.is_hidden();
        app.tick();
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
        let bt_incoming_before = app.controls.bluetooth.incoming_request.is_some();
        // Mirror the cursor position into AppState (in physical px) so
        // the renderer can drive cursor-aware effects (dock magnification
        // wave) without reaching into wayland state.
        {
            let scale_f = wl.fractional_scale() as f32;
            app.cursor_phys = (
                wl.cursor_x as f32 * scale_f,
                wl.cursor_y as f32 * scale_f,
            );
        }
        app.controls.tick();
        // PTY housekeeping for the Terminal view. We spawn lazily on
        // first activation and resize whenever the body geometry
        // changes so the child shell reflows correctly.
        if app.panel_view == crate::app::PanelView::Terminal {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            // Single source of truth for cell metrics + grid size so the
            // PTY's wrap column matches what we actually paint.
            let (_, _, _, cols, rows) = crate::terminal::body_metrics(
                panel_rect, top_y, scale_f, app.config.text_size,
            );
            app.terminal.ensure_spawned(cols.max(20), rows.max(5));
        }
        // Drain any pending PTY output into the grid so new bytes
        // appear in the next render.
        app.terminal.pump();

        // Drain any new snapshot the usage worker has produced.
        app.usage.pump();

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

        let bt_incoming_after = app.controls.bluetooth.incoming_request.is_some();
        // Fresh incoming-file request → jump straight to the BT view so
        // the modal isn't hidden behind whatever the user was looking at.
        if bt_incoming_after && !bt_incoming_before {
            tracing::info!("incoming BT file while panel visible → switching to BT view");
            app.mode =
                crate::app::PanelMode::Control(crate::controls::TileId::Bluetooth);
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
        handle_scroll(&mut wl, &mut app);

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
        handle_keypress(&mut wl, &mut app);

        // Terminal body selection (press → drag → release).
        handle_terminal_selection(&mut wl, &mut app);

        // Files-view click: toolbar (controls row) + body (sidebar + list).
        if app.panel_view == crate::app::PanelView::Files && wl.left_clicked
            && app.context_menu.is_none()
        {
            let scale_f = wl.fractional_scale() as f32;
            let phys_w = wl.phys_width().max(1);
            let panel = PanelRect::compute_with_dims(
                phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
            );
            let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
            let top_y = crate::controls::content_top_y(panel_rect, scale_f);
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;

            // Toolbar strip in the top-most row takes precedence.
            let strip_hit = files_strip_rect(&app, panel_rect, scale_f).map(|s| {
                crate::files::hit_strip(&app.files, s, scale_f, phys_cx, phys_cy)
            });
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
                &app.files, panel_rect, top_y, scale_f,
                app.config.text_size, phys_cx, phys_cy,
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

        if !wl.frame_done {
            continue;
        }
        wl.frame_done = false;

        let scale_f = wl.fractional_scale() as f32;
        render_frame(
            &mut wl, &mut app, &mut gpu, &surface, &viewport,
            &mut painter, &mut text, &mut mono_text, &mut thumbs,
            &mut icon_cache, &tex_pass, &qh, scale_f,
        );
    }

    Ok(())
}

// `handle_control_view_click` → layershell/click.rs
// `files_strip_rect`, `sort_menu_items`, `set_active_input`, `commit_transparent` → layershell/util.rs

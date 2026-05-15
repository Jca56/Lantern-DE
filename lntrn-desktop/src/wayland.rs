use std::ffi::c_void;
use std::os::fd::RawFd;
use std::ptr::NonNull;
use std::time::Instant;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, TextRenderer, TexturePass};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_pointer, wl_seat},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};

use crate::assets::IconCache;
use crate::fs_watch::DesktopWatcher;
use crate::input::{self, KeyAction};
use crate::keyboard::{self, KeyboardState};
use crate::layout::{grid_dims, ICON_PX};
use crate::render;
use crate::state::{DesktopState, PendingAction, RenameState};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;

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

pub struct State {
    pub running: bool,
    pub configured: bool,
    pub frame_done: bool,
    pub width: u32,
    pub height: u32,
    pub output_scale: i32,
    pub output_phys_width: u32,
    pub output_phys_height: u32,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub seat: Option<wl_seat::WlSeat>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub cursor_x: f64,
    pub cursor_y: f64,
    pub pointer_in_surface: bool,
    pub pointer_serial: u32,
    pub enter_serial: u32,
    pub left_pressed: bool,
    pub left_released: bool,
    pub right_pressed: bool,
    pub key_pressed: Option<u32>,
    pub keymap_pending: Option<(RawFd, u32)>,
    pub modifiers_pending: Option<(u32, u32, u32, u32)>,
    pub cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: true,
            width: 0,
            height: 0,
            output_scale: 1,
            output_phys_width: 0,
            output_phys_height: 0,
            compositor: None,
            viewporter: None,
            layer_shell: None,
            seat: None,
            pointer: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            pointer_serial: 0,
            enter_serial: 0,
            left_pressed: false,
            left_released: false,
            right_pressed: false,
            key_pressed: None,
            keymap_pending: None,
            modifiers_pending: None,
            cursor_shape_mgr: None,
            cursor_shape_device: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        // Logical layer-surface width vs. output physical width.
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.output_scale.max(1) as f64
        }
    }

    fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let layer_shell = state
        .layer_shell
        .clone()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let surface = compositor.create_surface(&qh, ());
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Bottom,
        "lntrn-desktop".to_string(),
        &qh,
        (),
    );
    use zwlr_layer_surface_v1::Anchor;
    layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
    layer_surface.set_size(0, 0);
    layer_surface.set_exclusive_zone(0);
    layer_surface.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand);
    surface.commit();

    while !state.configured || state.width == 0 || state.height == 0 {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    let viewport = state.viewporter.as_ref().map(|vp| {
        let v = vp.get_viewport(&surface, &qh, ());
        v.set_destination(state.width as i32, state.height as i32);
        v
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
    let tex_pass = TexturePass::new(&gpu);

    let scale = state.fractional_scale() as f32;
    let icon_px = (ICON_PX * scale) as u32;
    let mut icons = IconCache::new(icon_px);

    // App state — scan desktop, assign cells, then prime only the icons we need.
    let desktop_dir = crate::icons::ensure_desktop_dir();
    let mut app = DesktopState::new(desktop_dir.clone());
    let dims = grid_dims(state.width as f32, state.height as f32);
    app.rescan(dims);
    app.save_positions_if_dirty();
    icons.prime_for_items(&gpu, &tex_pass, &app.items);

    // Inotify watcher (we drain in the main loop alongside wayland events).
    let watcher = DesktopWatcher::new(&desktop_dir, &crate::widgets_config::config_path());
    if watcher.is_none() {
        tracing::warn!("[desktop] inotify init failed; live reload disabled");
    }
    // Quick poll: every 500ms re-check inotify, plus a slower 30s tick for the
    // clock so the minute display refreshes on its own.
    let mut last_watch_check = Instant::now();
    let mut last_clock_tick = Instant::now();

    let mut kbd = KeyboardState::new();
    let mut current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape> = None;

    // Audio capture starts unconditionally — the pipewire monitor stream
    // is cheap, and we want bars to react instantly when the user
    // toggles the visualizer on without restarting the daemon.
    let audio = crate::audio::AudioCapture::start();
    let mut viz = crate::visualizer::Visualizer::default();
    let mut last_viz_tick = Instant::now();
    // ~30 Hz cap on FFT work. Broadcast-standard motion; eye can't tell
    // the difference vs 60 Hz on a bar visualizer.
    const VIZ_TICK_MS: u128 = 33;

    // Rainbow widget animation clock. Starts at zero on launch; ~30 Hz
    // repaint cadence while the widget is enabled.
    let rainbow_start = Instant::now();
    let mut last_rainbow_tick = Instant::now();
    const RAINBOW_TICK_MS: u128 = 33;

    surface.frame(&qh, ());
    surface.commit();

    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            tracing::error!("[desktop] dispatch error: {e}");
            break;
        }

        // Periodically drain inotify (do this even if frame_done is unset so we
        // don't sit idle on a stale view).
        if let Some(w) = &watcher {
            if last_watch_check.elapsed().as_millis() > 500 {
                last_watch_check = Instant::now();
                let events = w.drain();
                if events.desktop_changed {
                    let dims = grid_dims(state.width as f32, state.height as f32);
                    app.rescan(dims);
                    state.frame_done = true;
                }
                if events.widgets_changed {
                    app.widgets = crate::widgets_config::WidgetsConfig::load();
                    state.frame_done = true;
                }
            }
        }

        // Clock tick — repaint when the displayed minute might have changed.
        if app.widgets.clock_enabled && last_clock_tick.elapsed().as_secs() >= 15 {
            last_clock_tick = Instant::now();
            state.frame_done = true;
        }

        // Visualizer drives continuous repaint while enabled. We tick
        // the FFT at ~60 Hz and only repaint when the tick produced new
        // bar values — between ticks we'd be redrawing identical pixels.
        if app.widgets.visualizer_enabled {
            if last_viz_tick.elapsed().as_millis() >= VIZ_TICK_MS {
                last_viz_tick = Instant::now();
                viz.tick(&audio);
                if viz.has_motion() {
                    state.frame_done = true;
                }
            } else if viz.has_motion() && !state.frame_done {
                // Bars still animating but FFT is throttled. Keep the
                // wakeup pump alive (re-arm the frame callback) without
                // redoing the wgpu paint — we'd be drawing the same
                // bars again.
                surface.frame(&qh, ());
                surface.commit();
                continue;
            }
        }

        // Rainbow widget — same throttling pattern as the visualizer.
        if app.widgets.rainbow_enabled {
            if last_rainbow_tick.elapsed().as_millis() >= RAINBOW_TICK_MS {
                last_rainbow_tick = Instant::now();
                state.frame_done = true;
            } else if !state.frame_done {
                surface.frame(&qh, ());
                surface.commit();
                continue;
            }
        }

        // Persist widget position after a drag finishes.
        if app.widgets_dirty {
            app.widgets.save();
            app.widgets_dirty = false;
        }

        if !state.frame_done {
            continue;
        }
        state.frame_done = false;

        // Handle resize.
        if state.configured {
            state.configured = false;
            let pw = state.phys_width().max(1);
            let ph = state.phys_height().max(1);
            gpu.resize(pw, ph);
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            // Recompute icon texture size if scale changed.
            let new_icon_px = (ICON_PX * state.fractional_scale() as f32) as u32;
            if new_icon_px != icon_px {
                icons.clear(new_icon_px);
            }
            let dims = grid_dims(state.width as f32, state.height as f32);
            app.rescan(dims);
            icons.prime_for_items(&gpu, &tex_pass, &app.items);
        }

        let s = state.fractional_scale() as f32;
        let cx = state.cursor_x as f32;
        let cy = state.cursor_y as f32;
        let dims = grid_dims(state.width as f32, state.height as f32);

        // Process pending keymap/modifiers.
        if let Some((fd, size)) = state.keymap_pending.take() {
            kbd.update_keymap(fd, size);
        }
        if let Some((dep, lat, lock, grp)) = state.modifiers_pending.take() {
            kbd.update_modifiers(dep, lat, lock, grp);
            app.ctrl_held = keyboard::ctrl_held(dep);
            app.shift_held = keyboard::shift_held(dep);
        }

        // Pointer events.
        let sw_f = state.width as f32;
        let sh_f = state.height as f32;
        if state.pointer_in_surface {
            input::on_cursor_move(&mut app, cx, cy, dims, sw_f, sh_f);
        }
        if state.left_pressed {
            state.left_pressed = false;
            input::on_left_press(&mut app, cx, cy, dims, sw_f, sh_f);
        }
        if state.left_released {
            state.left_released = false;
            input::on_left_release(&mut app, cx, cy, dims);
        }
        if state.right_pressed {
            state.right_pressed = false;
            input::on_right_press(
                &mut app,
                cx,
                cy,
                state.width as f32,
                state.height as f32,
            );
        }

        // Keyboard events.
        if let Some(key) = state.key_pressed.take() {
            let sym = kbd.key_get_sym(key);
            let utf8 = kbd.key_to_utf8(key);

            // Renaming mode consumes input.
            if app.renaming.is_some() {
                let ch = utf8
                    .as_deref()
                    .and_then(|s| s.chars().next())
                    .filter(|c| !c.is_control());
                let action = match sym {
                    keyboard::KEY_RETURN => Some(KeyAction::Enter),
                    keyboard::KEY_ESCAPE => Some(KeyAction::Escape),
                    keyboard::KEY_BACKSPACE => Some(KeyAction::Backspace),
                    keyboard::KEY_LEFT => Some(KeyAction::Left),
                    keyboard::KEY_RIGHT => Some(KeyAction::Right),
                    keyboard::KEY_HOME => Some(KeyAction::Home),
                    keyboard::KEY_END => Some(KeyAction::End),
                    _ if ch.is_some() => Some(KeyAction::Char),
                    _ => None,
                };
                if let Some(a) = action {
                    input::handle_rename_key(&mut app, ch, a);
                }
            } else {
                // Global shortcuts when not renaming.
                match sym {
                    keyboard::KEY_F2 => {
                        if let Some(&idx) = app.selection.iter().next() {
                            app.pending_action = Some(PendingAction::StartRename(idx));
                        }
                    }
                    keyboard::KEY_F5 => {
                        app.pending_action = Some(PendingAction::Refresh);
                    }
                    keyboard::KEY_DELETE => {
                        if !app.selection.is_empty() {
                            let idxs: Vec<usize> = app.selection.iter().copied().collect();
                            app.pending_action = Some(PendingAction::Trash(idxs));
                        }
                    }
                    keyboard::KEY_RETURN => {
                        if let Some(&idx) = app.selection.iter().next() {
                            app.pending_action = Some(PendingAction::Open(idx));
                        }
                    }
                    keyboard::KEY_ESCAPE => {
                        app.selection.clear();
                        app.menu = None;
                    }
                    _ => {}
                }
            }
        }

        // Apply any pending action.
        let mut needs_rescan = false;
        if let Some(action) = app.pending_action.take() {
            apply_action(&mut app, action, &mut needs_rescan);
        }
        if needs_rescan {
            app.rescan(dims);
        }
        app.save_positions_if_dirty();

        // Cursor shape — pointer where there's an icon → pointer; otherwise default.
        if state.pointer_in_surface {
            let desired = if app.hover.is_some() || app.drag.is_some() {
                wp_cursor_shape_device_v1::Shape::Pointer
            } else {
                wp_cursor_shape_device_v1::Shape::Default
            };
            if current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                current_cursor_shape = Some(desired);
            }
        } else {
            current_cursor_shape = None;
        }

        // Render frame.
        painter.clear();

        // Rainbow widget paints under icons so dragging it doesn't hide
        // file labels.
        if app.widgets.rainbow_enabled {
            let (rx, ry) = input::resolve_rainbow_pos(&app, state.width as f32, state.height as f32);
            let elapsed = rainbow_start.elapsed().as_secs_f32();
            crate::rainbow::draw(&mut painter, rx, ry, s, elapsed);
        }

        let tex_draws = render::draw_desktop(
            &mut painter,
            &mut text,
            &mut icons,
            &gpu,
            &tex_pass,
            &app,
            s,
            state.width,
            state.height,
        );

        // Visualizer bars sit at the bottom; drawn into the same
        // painter pass so they composite under icons / context menu
        // (rubber-band still wins because it's queued later inside
        // draw_desktop).
        if app.widgets.visualizer_enabled {
            viz.draw(&mut painter, state.width, state.height, s);
        }

        if let Ok(mut frame) = gpu.begin_frame("desktop") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
            tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
            text.render_queued(&gpu, frame.encoder_mut(), &view);
            frame.submit(&gpu.queue);
        }

        surface.frame(&qh, ());
        surface.commit();
    }

    app.save_positions_if_dirty();
    Ok(())
}

fn apply_action(app: &mut DesktopState, action: PendingAction, needs_rescan: &mut bool) {
    match action {
        PendingAction::Open(idx) => {
            if let Some(item) = app.items.get(idx) {
                crate::icons::open_with_default(&item.path);
            }
        }
        PendingAction::Trash(idxs) => {
            let mut paths: Vec<std::path::PathBuf> = idxs
                .iter()
                .filter_map(|&i| app.items.get(i).map(|it| it.path.clone()))
                .collect();
            paths.sort();
            paths.dedup();
            for p in &paths {
                crate::icons::move_to_trash(p);
            }
            app.selection.clear();
            *needs_rescan = true;
        }
        PendingAction::StartRename(idx) => {
            if let Some(item) = app.items.get(idx) {
                app.renaming = Some(RenameState {
                    idx,
                    buffer: item.name.clone(),
                    cursor: item.name.chars().count(),
                });
                app.selection.clear();
                app.selection.insert(idx);
            }
        }
        PendingAction::SubmitRename => {
            let Some(rn) = app.renaming.take() else {
                return;
            };
            let old = match app.items.get(rn.idx) {
                Some(it) => it.path.clone(),
                None => return,
            };
            let trimmed = rn.buffer.trim();
            if !trimmed.is_empty() && trimmed != old.file_name().unwrap_or_default().to_string_lossy()
            {
                if crate::icons::rename(&old, trimmed) {
                    // Carry over the position to the new name.
                    if let Some(cell) = app.positions.get(&app.items[rn.idx].name) {
                        app.positions.remove(&app.items[rn.idx].name);
                        app.positions.set(trimmed, cell);
                        app.dirty_positions = true;
                    }
                    *needs_rescan = true;
                }
            }
        }
        PendingAction::CancelRename => {
            app.renaming = None;
        }
        PendingAction::NewFolder => {
            if let Some(new_path) = crate::icons::new_folder(&app.desktop_dir) {
                // Place the new folder where the menu was opened (anchor) — if known.
                let anchor_cell = app
                    .menu
                    .as_ref()
                    .map(|m| crate::layout::pixel_to_cell(m.anchor_x, m.anchor_y));
                if let (Some(cell), Some(name)) = (anchor_cell, new_path.file_name()) {
                    app.positions
                        .set(&name.to_string_lossy(), cell);
                    app.dirty_positions = true;
                }
                *needs_rescan = true;
            }
        }
        PendingAction::Refresh => {
            *needs_rescan = true;
        }
        PendingAction::SelectAll => {
            app.selection = (0..app.items.len()).collect();
        }
        PendingAction::OpenTerminal => {
            let dir = app.desktop_dir.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("lntrn-terminal")
                    .current_dir(&dir)
                    .spawn();
            });
        }
        PendingAction::CopyName(idx) => {
            if let Some(item) = app.items.get(idx) {
                let s = item.name.clone();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("wl-copy")
                        .arg(&s)
                        .spawn();
                });
            }
        }
        PendingAction::CopyPath(idx) => {
            if let Some(item) = app.items.get(idx) {
                let s = item.path.display().to_string();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("wl-copy")
                        .arg(&s)
                        .spawn();
                });
            }
        }
    }
}

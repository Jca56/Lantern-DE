use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, Rect, TextRenderer, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_data_device_manager, wl_data_offer, wl_seat, wl_surface},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::app::App;
use crate::canvas::editor::{CanvasEditor, DialogKind, DragMode};
use crate::canvas::input::{self as canvas_input, CanvasAction, CursorHint};
use crate::canvas::persist;
use crate::canvas::sidebar::SidebarState;
use crate::canvas::tex_cache::CanvasTexCache;
use crate::render_launcher::{self, LauncherState};
use crate::{
    AppMode, Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_LAUNCHER_ITEM_BASE, ZONE_LAUNCHER_NEW,
    ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_NAV_NEXT, ZONE_NAV_PREV, ZONE_SHUFFLE,
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
// Fields are pub(crate): the Dispatch impls that fill them live in
// wayland_dispatch.rs (protocol plumbing) and dnd.rs (drag-and-drop).

pub(crate) struct State {
    pub(crate) running: bool,
    /// Compositor asked us to close (xdg_toplevel.close) — the main loop turns
    /// this into a confirm dialog when the canvas has unsaved changes.
    pub(crate) close_requested: bool,
    pub(crate) configured: bool,
    pub(crate) frame_done: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: i32,
    pub(crate) output_phys_width: u32,
    pub(crate) output_phys_height: u32,
    pub(crate) maximized: bool,
    // Wayland objects
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) surface: Option<wl_surface::WlSurface>,
    pub(crate) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(crate) toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    pub(crate) cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub(crate) cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub(crate) current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    // Input
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) middle_pressed: bool,
    pub(crate) middle_released: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) enter_serial: u32,
    // Keyboard
    pub(crate) ctrl: bool,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
    pub(crate) key_pressed: Option<u32>,
    // Drag-and-drop receive (Dispatch impls live in dnd.rs)
    pub(crate) data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub(crate) dnd_mimes: Vec<String>,
    pub(crate) dnd_offer: Option<wl_data_offer::WlDataOffer>,
    /// Drop cursor position in logical surface coords (× scale before use).
    pub(crate) dnd_x: f64,
    pub(crate) dnd_y: f64,
    /// True while a drop's pipe is being read on a worker thread — keeps the
    /// event loop polling so the result lands promptly.
    pub(crate) dnd_reading: bool,
    pub(crate) dnd_tx: mpsc::Sender<Vec<PathBuf>>,
}

impl State {
    fn new(dnd_tx: mpsc::Sender<Vec<PathBuf>>) -> Self {
        Self {
            running: true,
            close_requested: false,
            configured: false,
            frame_done: true,
            width: 0,
            height: 0,
            scale: 1,
            output_phys_width: 0,
            output_phys_height: 0,
            maximized: false,
            compositor: None,
            wm_base: None,
            viewporter: None,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            seat: None,
            cursor_shape_mgr: None,
            cursor_shape_device: None,
            current_cursor_shape: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            left_pressed: false,
            left_released: false,
            middle_pressed: false,
            middle_released: false,
            scroll_delta: 0.0,
            pointer_serial: 0,
            enter_serial: 0,
            ctrl: false,
            shift: false,
            alt: false,
            key_pressed: None,
            data_device_manager: None,
            dnd_mimes: Vec::new(),
            dnd_offer: None,
            dnd_x: 0.0,
            dnd_y: 0.0,
            dnd_reading: false,
            dnd_tx,
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

pub fn run(initial_path: Option<String>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let (dnd_tx, dnd_rx) = mpsc::channel::<Vec<PathBuf>>();
    let mut state = State::new(dnd_tx);

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;
    // Second roundtrip so wl_output.Mode/Scale events arrive before we size the window.
    event_queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state
        .wm_base
        .as_ref()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    // Compute initial window size from the image's native dimensions, capped to
    // ~85% of the output's logical size while preserving aspect ratio. Falls
    // back to 960×640 if no image arg was given or its header is unreadable.
    let scale_i = state.scale.max(1) as u32;
    let screen_logical_w = if state.output_phys_width > 0 {
        state.output_phys_width / scale_i
    } else {
        1920
    };
    let screen_logical_h = if state.output_phys_height > 0 {
        state.output_phys_height / scale_i
    } else {
        1080
    };

    let (init_w, init_h) = initial_path
        .as_deref()
        .and_then(|p| crate::app::peek_image_dimensions(Path::new(p)))
        .map(|(w, h)| fit_to_screen(w, h, screen_logical_w, screen_logical_h))
        .unwrap_or((960, 640));

    if state.width == 0 {
        state.width = init_w;
    }
    if state.height == 0 {
        state.height = init_h;
    }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Image Viewer".into());
    toplevel.set_app_id("lntrn-image-viewer".into());
    // The compositor reads a min == max declaration before the first configure
    // as an exact startup-size request (a bare min would only clamp its default
    // suggestion). Relaxed right after the first configure so the user can
    // still resize the window freely.
    toplevel.set_min_size(init_w as i32, init_h as i32);
    toplevel.set_max_size(init_w as i32, init_h as i32);
    surface.commit();

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    // DnD target: needs a data device on our seat (events handled in dnd.rs).
    let _data_device = match (&state.data_device_manager, &state.seat) {
        (Some(mgr), Some(seat)) => Some(mgr.get_data_device(seat, &qh, ())),
        _ => None,
    };

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;
    // Relax the exact-size declaration so the user can resize the window
    // freely (max 0,0 = unlimited per the xdg-shell protocol).
    toplevel.set_min_size(400, 300);
    toplevel.set_max_size(0, 0);

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

    let mut app = App::new();
    let mut input = InteractionContext::new();

    // ── Mode selection ──────────────────────────────────────────────
    let mut editor = CanvasEditor::new_empty();
    let mut sidebar = SidebarState::new();
    let mut tex_cache = CanvasTexCache::new();
    let mut launcher = LauncherState::new();
    let mut mode = match &initial_path {
        Some(p) if p.to_ascii_lowercase().ends_with(".lcanvas") => {
            match persist::load_canvas(Path::new(p)) {
                Ok(doc) => {
                    editor = CanvasEditor::from_doc(doc, Some(PathBuf::from(p)));
                    AppMode::Canvas
                }
                Err(e) => {
                    eprintln!("[image-viewer] cannot open canvas {p}: {e}");
                    launcher.error = Some(format!("Couldn't open canvas: {e}"));
                    AppMode::Launcher
                }
            }
        }
        Some(p) => {
            app.open_image(&gpu.ctx, &gpu.tex_pass, p);
            AppMode::Viewer
        }
        None => AppMode::Launcher,
    };

    let mut last_frame = Instant::now();
    let mut last_title = String::new();

    while state.running {
        // Non-blocking dispatch while anything animates or a DnD read is in
        // flight (those complete off the wayland socket, so blocking_dispatch
        // would never wake for them); blocking otherwise.
        let canvas_busy =
            mode == AppMode::Canvas && (sidebar.scroll.is_animating() || sidebar.has_pending());
        if app.gif.is_some() || canvas_busy || state.dnd_reading {
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                eprintln!("[image-viewer] dispatch error: {e}");
                break;
            }
            event_queue.flush()?;
            // Tick GIF animation
            let gif_changed = app.tick_gif(&gpu.ctx, &gpu.tex_pass);
            if gif_changed {
                state.frame_done = true;
            }
            if !state.frame_done {
                if canvas_busy || state.dnd_reading {
                    // Steady redraw cadence for scroll animation / thumbnail
                    // arrival / drop-read completion.
                    std::thread::sleep(std::time::Duration::from_millis(12));
                    state.frame_done = true;
                } else {
                    // Sleep until next GIF frame is due (or a short poll interval)
                    let sleep = app
                        .gif
                        .as_ref()
                        .map(|g| {
                            let remaining = g.current_delay().saturating_sub(g.last_swap.elapsed());
                            remaining.min(std::time::Duration::from_millis(16))
                        })
                        .unwrap_or(std::time::Duration::from_millis(16));
                    std::thread::sleep(sleep);
                    continue;
                }
            }
        } else {
            if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                eprintln!("[image-viewer] dispatch error: {e}");
                break;
            }
            if !state.frame_done {
                continue;
            }
        }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.ctx
                .resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let s = scale_f;

        // ── Compositor close request ────────────────────────────────────
        if state.close_requested {
            state.close_requested = false;
            if mode == AppMode::Canvas && editor.dirty && editor.dialog.is_none() {
                editor.dialog = Some(DialogKind::ConfirmQuit);
            } else {
                state.running = false;
            }
        }

        // ── Cursor ──────────────────────────────────────────────────────
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        // ── DnD drops (read off-thread, results polled here) ────────────
        while let Ok(paths) = dnd_rx.try_recv() {
            state.dnd_reading = false;
            if paths.is_empty() {
                continue;
            }
            match mode {
                AppMode::Canvas => {
                    let dx = (state.dnd_x as f32) * s;
                    let dy = (state.dnd_y as f32) * s;
                    canvas_input::add_dropped(&mut editor, &sidebar, &paths, dx, dy, wf, hf, s);
                }
                AppMode::Viewer => {
                    if let Some(p) = paths.iter().find(|p| crate::app::is_supported(p)) {
                        app.open_image(&gpu.ctx, &gpu.tex_pass, &p.to_string_lossy());
                    }
                }
                AppMode::Launcher => {}
            }
        }

        // ── Scroll ──────────────────────────────────────────────────────
        if state.scroll_delta.abs() > 0.01 {
            let delta = state.scroll_delta;
            state.scroll_delta = 0.0;
            match mode {
                AppMode::Viewer => {
                    let title_h = crate::TITLE_H * s;
                    let status_h = crate::STATUS_H * s;
                    let canvas = Rect::new(0.0, title_h, wf, hf - title_h - status_h);
                    if canvas.contains(cx, cy) {
                        let factor = if delta < 0.0 { 1.03 } else { 1.0 / 1.03 };
                        app.zoom_at(
                            factor,
                            cx,
                            cy,
                            canvas.x + canvas.w * 0.5,
                            canvas.y + canvas.h * 0.5,
                        );
                    }
                }
                AppMode::Canvas => {
                    canvas_input::on_scroll(
                        &mut editor,
                        &mut sidebar,
                        delta,
                        state.ctrl,
                        cx,
                        cy,
                        wf,
                        hf,
                        s,
                    );
                }
                AppMode::Launcher => {
                    render_launcher::apply_scroll(&mut launcher, delta * s * 4.0, wf, hf, s);
                }
            }
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            match mode {
                AppMode::Viewer => handle_key(&mut app, &mut gpu, key, state.ctrl),
                AppMode::Canvas => {
                    let action = canvas_input::on_key(
                        &mut editor,
                        &mut sidebar,
                        key,
                        state.ctrl,
                        state.shift,
                        wf,
                        hf,
                        s,
                    );
                    if matches!(action, CanvasAction::Quit) {
                        state.running = false;
                    }
                }
                AppMode::Launcher => {
                    const KEY_Q: u32 = 16;
                    if state.ctrl && key == KEY_Q {
                        state.running = false;
                    }
                }
            }
        }

        // ── Left press ──────────────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            // Edge resize
            let border = crate::RESIZE_BORDER * s;
            if let Some(edge) = edge_resize(cx, cy, wf, hf, border) {
                if let Some(seat) = &state.seat {
                    toplevel.resize(seat, state.pointer_serial, edge);
                }
            } else if let Some(zone_id) = input.on_left_pressed() {
                match zone_id {
                    ZONE_CLOSE => {
                        if mode == AppMode::Canvas && editor.dirty {
                            editor.dialog = Some(DialogKind::ConfirmQuit);
                        } else {
                            state.running = false;
                        }
                    }
                    ZONE_MINIMIZE => {
                        toplevel.set_minimized();
                    }
                    ZONE_MAXIMIZE => {
                        if state.maximized {
                            toplevel.unset_maximized();
                        } else {
                            toplevel.set_maximized();
                        }
                    }
                    _ => match mode {
                        AppMode::Viewer => match zone_id {
                            ZONE_CANVAS => {
                                // Start panning
                                app.is_panning = true;
                                app.last_pan_x = cx;
                                app.last_pan_y = cy;
                            }
                            ZONE_NAV_PREV => {
                                app.prev_image(&gpu.ctx, &gpu.tex_pass);
                            }
                            ZONE_NAV_NEXT => {
                                app.next_image(&gpu.ctx, &gpu.tex_pass);
                            }
                            ZONE_SHUFFLE => {
                                app.toggle_shuffle();
                            }
                            _ => {}
                        },
                        AppMode::Canvas => {
                            let action = canvas_input::on_zone_pressed(
                                &mut editor,
                                &mut sidebar,
                                zone_id,
                                cx,
                                cy,
                                wf,
                                hf,
                                s,
                            );
                            if matches!(action, CanvasAction::Quit) {
                                state.running = false;
                            }
                        }
                        AppMode::Launcher => match zone_id {
                            ZONE_LAUNCHER_NEW => {
                                editor = CanvasEditor::new_empty();
                                tex_cache.clear();
                                mode = AppMode::Canvas;
                            }
                            z if z >= ZONE_LAUNCHER_ITEM_BASE => {
                                let i = (z - ZONE_LAUNCHER_ITEM_BASE) as usize;
                                if let Some(entry) = launcher.canvases.get(i) {
                                    match persist::load_canvas(&entry.path) {
                                        Ok(doc) => {
                                            editor = CanvasEditor::from_doc(
                                                doc,
                                                Some(entry.path.clone()),
                                            );
                                            tex_cache.clear();
                                            mode = AppMode::Canvas;
                                        }
                                        Err(e) => {
                                            launcher.error =
                                                Some(format!("Couldn't open {}: {e}", entry.name,));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        },
                    },
                }
            } else {
                // Title bar drag
                let title_h = crate::TITLE_H * s;
                if cy < title_h {
                    if let Some(seat) = &state.seat {
                        toplevel._move(seat, state.pointer_serial);
                    }
                }
            }
        }

        // ── Middle press → pan ──────────────────────────────────────────
        if state.middle_pressed {
            state.middle_pressed = false;
            match mode {
                AppMode::Viewer => {
                    app.is_panning = true;
                    app.last_pan_x = cx;
                    app.last_pan_y = cy;
                }
                AppMode::Canvas => {
                    if editor.dialog.is_none() {
                        editor.drag = DragMode::PanningCanvas {
                            last_x: cx,
                            last_y: cy,
                        };
                    }
                }
                AppMode::Launcher => {}
            }
        }

        // ── Pointer-driven updates while moving ─────────────────────────
        if mode == AppMode::Viewer && app.is_panning && state.pointer_in_surface {
            let dx = cx - app.last_pan_x;
            let dy = cy - app.last_pan_y;
            app.pan_x += dx;
            app.pan_y += dy;
            app.last_pan_x = cx;
            app.last_pan_y = cy;
        }
        if mode == AppMode::Canvas && state.pointer_in_surface {
            canvas_input::on_motion(
                &mut editor,
                &mut sidebar,
                &input,
                cx,
                cy,
                wf,
                hf,
                s,
                !state.alt,
            );
        }

        // ── Left/middle release ─────────────────────────────────────────
        if state.left_released {
            state.left_released = false;
            if mode == AppMode::Canvas {
                canvas_input::on_release(&mut editor, &mut sidebar, cx, cy, wf, hf, s);
            }
            app.is_panning = false;
            input.on_left_released();
        }
        if state.middle_released {
            state.middle_released = false;
            app.is_panning = false;
            if mode == AppMode::Canvas && matches!(editor.drag, DragMode::PanningCanvas { .. }) {
                editor.drag = DragMode::Idle;
            }
        }

        // ── Keep SVGs crisp ──────────────────────────────────────────────
        // Re-rasterize the vector image to the size it's about to be drawn at,
        // mirroring the fit+zoom math in render_frame so it never looks blurry.
        if let Some(img) = &app.image {
            let canvas_w = wf;
            let canvas_h = hf - (crate::TITLE_H + crate::STATUS_H) * s;
            let fit_zoom = (canvas_w / img.width as f32).min(canvas_h / img.height as f32);
            let display_zoom = fit_zoom * app.zoom;
            let disp_w = img.width as f32 * display_zoom;
            let disp_h = img.height as f32 * display_zoom;
            app.maybe_rerender_svg(&gpu.ctx, &gpu.tex_pass, disp_w, disp_h);
        }

        // ── Resize cursor on hover ───────────────────────────────────────
        // Show a directional resize cursor whenever the pointer is over an
        // edge band — not just while an interactive resize is in progress.
        if state.pointer_in_surface {
            let border = crate::RESIZE_BORDER * s;
            let in_canvas = mode == AppMode::Canvas;
            let desired = if in_canvas && sidebar.resizing {
                wp_cursor_shape_device_v1::Shape::ColResize
            } else {
                match edge_resize(cx, cy, wf, hf, border) {
                    Some(edge) => resize_edge_to_cursor_shape(edge),
                    None if in_canvas => canvas_cursor_shape(canvas_input::cursor_hint(
                        &editor, &sidebar, cx, cy, wf, hf, s,
                    )),
                    None => wp_cursor_shape_device_v1::Shape::Default,
                }
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Window title tracks the canvas name + dirty state ───────────
        let desired_title = match mode {
            AppMode::Canvas => editor.window_title(),
            _ => "Lantern Image Viewer".to_string(),
        };
        if desired_title != last_title {
            toplevel.set_title(desired_title.clone());
            last_title = desired_title;
        }

        // ── Render ──────────────────────────────────────────────────────
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f32().min(0.05);
        last_frame = now;
        // Re-read each frame so theme/accent changes apply on next draw.
        let palette = FoxPalette::current();
        match mode {
            AppMode::Viewer => {
                crate::render::render_frame(&mut gpu, &app, &mut input, &palette, s);
            }
            AppMode::Launcher => {
                render_launcher::render_launcher_frame(
                    &mut gpu,
                    &mut launcher,
                    &mut input,
                    &palette,
                    s,
                );
            }
            AppMode::Canvas => {
                crate::render_canvas::render_canvas_frame(
                    &mut gpu,
                    &mut editor,
                    &mut sidebar,
                    &mut tex_cache,
                    &mut input,
                    &palette,
                    s,
                    dt,
                );
            }
        }

        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

// ── Sizing helpers ──────────────────────────────────────────────────────────

/// Cap (img_w, img_h) to 85% of the screen while preserving aspect ratio.
/// Returns native size for images that already fit.
fn fit_to_screen(img_w: u32, img_h: u32, screen_w: u32, screen_h: u32) -> (u32, u32) {
    let max_w = (screen_w as f32 * 0.85).max(320.0);
    let max_h = (screen_h as f32 * 0.85).max(240.0);
    let iw = img_w.max(1) as f32;
    let ih = img_h.max(1) as f32;
    if iw <= max_w && ih <= max_h {
        return (img_w.max(1), img_h.max(1));
    }
    let s = (max_w / iw).min(max_h / ih);
    ((iw * s).round() as u32, (ih * s).round() as u32)
}

// ── Input helpers ───────────────────────────────────────────────────────────

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

/// Map a resize edge to the matching directional cursor shape.
/// Canvas-mode pointer shapes: sidebar grip, item grab, and handle resizes.
fn canvas_cursor_shape(hint: CursorHint) -> wp_cursor_shape_device_v1::Shape {
    use crate::canvas::editor::ResizeHandle as H;
    use wp_cursor_shape_device_v1::Shape;
    match hint {
        CursorHint::Default => Shape::Default,
        CursorHint::ColResize => Shape::ColResize,
        CursorHint::Grab => Shape::Grab,
        CursorHint::Grabbing => Shape::Grabbing,
        CursorHint::Resize(h) => match h {
            H::TopLeft | H::BottomRight => Shape::NwseResize,
            H::TopRight | H::BottomLeft => Shape::NeswResize,
            H::Top | H::Bottom => Shape::NsResize,
            H::Left | H::Right => Shape::EwResize,
        },
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

// Linux keycodes
const KEY_Q: u32 = 16;
const KEY_0: u32 = 11;
const KEY_EQUAL: u32 = 13; // =/+ key
const KEY_MINUS: u32 = 12;
const KEY_S: u32 = 31;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;

fn handle_key(app: &mut App, gpu: &mut Gpu, key: u32, ctrl: bool) {
    match key {
        KEY_LEFT => {
            app.prev_image(&gpu.ctx, &gpu.tex_pass);
        }
        KEY_RIGHT => {
            app.next_image(&gpu.ctx, &gpu.tex_pass);
        }
        KEY_S if !ctrl => {
            app.toggle_shuffle();
        }
        _ if ctrl => match key {
            KEY_Q => std::process::exit(0),
            KEY_EQUAL => {
                app.zoom = (app.zoom * 1.05).min(50.0);
            }
            KEY_MINUS => {
                app.zoom = (app.zoom / 1.05).max(0.05);
            }
            KEY_0 => {
                app.fit_to_view();
            }
            _ => {}
        },
        _ => {}
    }
}

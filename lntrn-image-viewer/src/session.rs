//! What the app is doing between Wayland events: which mode it's in, the
//! state of each, and the work of one wake-up — input, timers, drops, close
//! requests — plus drawing a frame when one is owed. wayland.rs owns the
//! connection and the loop; this owns everything the loop drives.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use lntrn_ui::gpu::{FoxPalette, InteractionContext};
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::QueueHandle;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;

use crate::app::App;
use crate::canvas::editor::{CanvasEditor, DialogKind, DragMode};
use crate::canvas::input::{self as canvas_input, CanvasAction};
use crate::canvas::persist;
use crate::canvas::sidebar::SidebarState;
use crate::canvas::tex_cache::CanvasTexCache;
use crate::cursor;
use crate::render_launcher::{self, LauncherState};
use crate::viewer_input::{self, ViewerAction};
use crate::wayland::State;
use crate::{AppMode, Gpu, ZONE_CLOSE, ZONE_LAUNCHER_ITEM_BASE, ZONE_LAUNCHER_NEW, ZONE_MAXIMIZE, ZONE_MINIMIZE};

/// How often an idle window looks for a theme change in lantern.toml.
const THEME_POLL: Duration = Duration::from_secs(1);

/// The Wayland objects input handling acts through.
pub struct Window<'a> {
    pub toplevel: &'a XdgToplevel,
    pub data_device: Option<&'a WlDataDevice>,
    pub qh: &'a QueueHandle<State>,
}

pub struct Session {
    pub mode: AppMode,
    app: App,
    editor: CanvasEditor,
    sidebar: SidebarState,
    tex_cache: CanvasTexCache,
    launcher: LauncherState,
    input: InteractionContext,
    last_title: String,
    last_frame: Instant,
    theme_checked: Instant,
    theme_stamp: Option<SystemTime>,
}

impl Session {
    pub fn new(initial_path: Option<&Path>, gpu: &Gpu) -> Self {
        let mut s = Self {
            mode: AppMode::Launcher,
            app: App::new(),
            editor: CanvasEditor::new_empty(),
            sidebar: SidebarState::new(),
            tex_cache: CanvasTexCache::new(),
            launcher: LauncherState::new(),
            input: InteractionContext::new(),
            last_title: String::new(),
            last_frame: Instant::now(),
            theme_checked: Instant::now(),
            theme_stamp: theme_stamp(),
        };
        s.mode = match initial_path {
            Some(p) if crate::wayland::is_canvas_file(p) => match persist::load_canvas(p) {
                Ok(doc) => {
                    s.editor = CanvasEditor::from_doc(doc, Some(p.to_path_buf()));
                    AppMode::Canvas
                }
                Err(e) => {
                    eprintln!("[image-viewer] cannot open canvas {}: {e}", p.display());
                    s.launcher.error = Some(format!("Couldn't open canvas: {e}"));
                    AppMode::Launcher
                }
            },
            Some(p) => {
                s.app.open_image(&gpu.ctx, &gpu.tex_pass, p);
                // The viewer starts with the browser folded to a strip (B or
                // click opens it); canvas mode wants it open since it's the
                // source of images.
                s.sidebar.collapsed = true;
                AppMode::Viewer
            }
            None => AppMode::Launcher,
        };
        s
    }

    // ── Loop pacing ─────────────────────────────────────────────────────

    /// The next moment something is due without any event to announce it.
    pub fn next_deadline(&self, state: &State, now: Instant) -> Option<Instant> {
        let app = (self.mode == AppMode::Viewer)
            .then(|| self.app.next_deadline(now))
            .flatten();
        let theme = Some(self.theme_checked + THEME_POLL);
        [app, state.keyboard.repeat_at(), theme]
            .into_iter()
            .flatten()
            .min()
    }

    /// Background work finishing off the socket (thumbnails, a drop being
    /// read, a GIF frame being decoded) — the loop polls until it lands.
    pub fn is_busy(&self, state: &State, now: Instant) -> bool {
        state.dnd_reading
            || (self.mode != AppMode::Launcher && self.sidebar.has_pending())
            || (self.mode == AppMode::Viewer && self.app.is_waiting(now))
    }

    /// Something moves every frame (the browser's smooth scroll), so the
    /// next frame is owed as soon as the compositor takes this one.
    pub fn is_animating(&self) -> bool {
        self.mode != AppMode::Launcher && self.sidebar.scroll.is_animating()
    }

    // ── One wake-up ─────────────────────────────────────────────────────

    pub fn on_wake(&mut self, state: &mut State, win: &Window, gpu: &Gpu, dnd_rx: &Receiver<Vec<PathBuf>>) {
        let now = Instant::now();
        let s = state.fractional_scale() as f32;
        let (wf, hf) = (gpu.ctx.width() as f32, gpu.ctx.height() as f32);

        self.handle_close(state);
        self.handle_drops(state, gpu, dnd_rx, wf, hf, s);

        // Timers and background results.
        if self.mode == AppMode::Viewer && self.app.tick(&gpu.ctx, &gpu.tex_pass, now) {
            state.needs_redraw = true;
        }
        if self.mode != AppMode::Launcher && self.sidebar.poll_thumbs(&gpu.ctx, &gpu.tex_pass) {
            state.needs_redraw = true;
        }
        if now >= self.theme_checked + THEME_POLL {
            self.theme_checked = now;
            let stamp = theme_stamp();
            if stamp != self.theme_stamp {
                self.theme_stamp = stamp;
                state.needs_redraw = true;
            }
        }
        state.keyboard.tick_repeat(now);

        // Input.
        let cx = state.cursor_x as f32 * s;
        let cy = state.cursor_y as f32 * s;
        if state.pointer_in_surface {
            self.input.on_cursor_moved(cx, cy);
        } else {
            self.input.on_cursor_left();
        }
        self.handle_scroll(state, cx, cy, wf, hf, s);
        self.handle_keys(state, win, gpu, wf, hf, s);
        self.handle_left_press(state, win, gpu, cx, cy, wf, hf, s);
        self.handle_middle_press(state, cx, cy);
        self.handle_motion(state, cx, cy, wf, hf, s);
        self.handle_releases(state, gpu, cx, cy, wf, hf, s);
        self.update_cursor_shape(state, cx, cy, wf, hf, s);
    }

    fn handle_close(&mut self, state: &mut State) {
        if !state.close_requested {
            return;
        }
        state.close_requested = false;
        if self.mode == AppMode::Canvas && self.editor.dirty && self.editor.dialog.is_none() {
            self.editor.dialog = Some(DialogKind::ConfirmQuit);
            state.needs_redraw = true;
        } else {
            state.running = false;
        }
    }

    /// Drops are read off-thread; their paths arrive here.
    fn handle_drops(&mut self, state: &mut State, gpu: &Gpu, dnd_rx: &Receiver<Vec<PathBuf>>, wf: f32, hf: f32, s: f32) {
        while let Ok(paths) = dnd_rx.try_recv() {
            state.dnd_reading = false;
            state.needs_redraw = true;
            if paths.is_empty() {
                continue;
            }
            match self.mode {
                AppMode::Canvas => {
                    let dx = state.dnd_x as f32 * s;
                    let dy = state.dnd_y as f32 * s;
                    canvas_input::add_dropped(&mut self.editor, &self.sidebar, &paths, dx, dy, wf, hf, s);
                }
                AppMode::Viewer => {
                    if let Some(p) = paths.iter().find(|p| crate::app::is_supported(p)) {
                        self.app.open_image(&gpu.ctx, &gpu.tex_pass, p);
                    }
                }
                AppMode::Launcher => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_scroll(&mut self, state: &mut State, cx: f32, cy: f32, wf: f32, hf: f32, s: f32) {
        if state.scroll_delta.abs() <= 0.01 {
            return;
        }
        let delta = std::mem::take(&mut state.scroll_delta);
        let ctrl = state.keyboard.ctrl;
        match self.mode {
            AppMode::Viewer => {
                viewer_input::on_scroll(&mut self.app, &mut self.sidebar, delta, ctrl, cx, cy, wf, hf, s)
            }
            AppMode::Canvas => {
                canvas_input::on_scroll(&mut self.editor, &mut self.sidebar, delta, ctrl, cx, cy, wf, hf, s)
            }
            AppMode::Launcher => {
                render_launcher::apply_scroll(&mut self.launcher, delta * s * 4.0, wf, hf, s)
            }
        }
    }

    fn handle_keys(&mut self, state: &mut State, win: &Window, gpu: &Gpu, wf: f32, hf: f32, s: f32) {
        const KEY_Q: u32 = 16;
        while let Some(press) = state.keyboard.pop() {
            state.needs_redraw = true;
            match self.mode {
                AppMode::Viewer => match viewer_input::on_key(&mut self.app, &mut self.sidebar, gpu, press) {
                    ViewerAction::Quit => state.running = false,
                    ViewerAction::Copy => {
                        crate::clipboard::copy_current(state, win.data_device, win.qh, &mut self.app)
                    }
                    ViewerAction::None => {}
                },
                AppMode::Canvas => {
                    let action = canvas_input::on_key(&mut self.editor, &mut self.sidebar, press, wf, hf, s);
                    if matches!(action, CanvasAction::Quit) {
                        state.running = false;
                    }
                }
                AppMode::Launcher => {
                    if press.ctrl && press.key == KEY_Q && !press.repeat {
                        state.running = false;
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_left_press(&mut self, state: &mut State, win: &Window, gpu: &Gpu, cx: f32, cy: f32, wf: f32, hf: f32, s: f32) {
        if !std::mem::take(&mut state.left_pressed) {
            return;
        }
        let border = crate::RESIZE_BORDER * s;
        // Edge resize (a maximized window has no edges to drag).
        if let Some(edge) = cursor::edge_resize(cx, cy, wf, hf, border).filter(|_| !state.maximized) {
            if let Some(seat) = &state.seat {
                win.toplevel.resize(seat, state.pointer_serial, edge);
            }
            return;
        }
        let Some(zone_id) = self.input.on_left_pressed() else {
            // Title bar drag (no bar to grab while the viewer chrome is hidden)
            let title_h = if self.mode == AppMode::Viewer && self.app.chrome_hidden {
                0.0
            } else {
                crate::TITLE_H * s
            };
            if cy < title_h {
                if let Some(seat) = &state.seat {
                    win.toplevel._move(seat, state.pointer_serial);
                }
            }
            return;
        };
        match zone_id {
            ZONE_CLOSE => {
                if self.mode == AppMode::Canvas && self.editor.dirty {
                    self.editor.dialog = Some(DialogKind::ConfirmQuit);
                } else {
                    state.running = false;
                }
            }
            ZONE_MINIMIZE => win.toplevel.set_minimized(),
            ZONE_MAXIMIZE => {
                if state.maximized {
                    win.toplevel.unset_maximized();
                } else {
                    win.toplevel.set_maximized();
                }
            }
            _ => match self.mode {
                AppMode::Viewer => {
                    viewer_input::on_zone_pressed(&mut self.app, &mut self.sidebar, gpu, zone_id, cx, cy)
                }
                AppMode::Canvas => {
                    let action = canvas_input::on_zone_pressed(
                        &mut self.editor,
                        &mut self.sidebar,
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
                AppMode::Launcher => self.launcher_pressed(zone_id),
            },
        }
    }

    fn launcher_pressed(&mut self, zone_id: u32) {
        match zone_id {
            ZONE_LAUNCHER_NEW => {
                self.editor = CanvasEditor::new_empty();
                self.tex_cache.clear();
                self.mode = AppMode::Canvas;
            }
            z if z >= ZONE_LAUNCHER_ITEM_BASE => {
                let i = (z - ZONE_LAUNCHER_ITEM_BASE) as usize;
                let Some(entry) = self.launcher.canvases.get(i) else {
                    return;
                };
                match persist::load_canvas(&entry.path) {
                    Ok(doc) => {
                        self.editor = CanvasEditor::from_doc(doc, Some(entry.path.clone()));
                        self.tex_cache.clear();
                        self.mode = AppMode::Canvas;
                    }
                    Err(e) => {
                        self.launcher.error = Some(format!("Couldn't open {}: {e}", entry.name));
                    }
                }
            }
            _ => {}
        }
    }

    /// Middle button pans.
    fn handle_middle_press(&mut self, state: &mut State, cx: f32, cy: f32) {
        if !std::mem::take(&mut state.middle_pressed) {
            return;
        }
        match self.mode {
            AppMode::Viewer => {
                self.app.is_panning = true;
                self.app.last_pan_x = cx;
                self.app.last_pan_y = cy;
            }
            AppMode::Canvas => {
                if self.editor.dialog.is_none() {
                    self.editor.drag = DragMode::PanningCanvas {
                        last_x: cx,
                        last_y: cy,
                    };
                }
            }
            AppMode::Launcher => {}
        }
    }

    /// Pointer-driven updates while moving: pans, drags, resizes.
    fn handle_motion(&mut self, state: &State, cx: f32, cy: f32, wf: f32, hf: f32, s: f32) {
        if !state.pointer_in_surface {
            return;
        }
        match self.mode {
            AppMode::Viewer => {
                viewer_input::on_motion(&mut self.app, &mut self.sidebar, &self.input, cx, cy, wf, hf, s)
            }
            AppMode::Canvas => canvas_input::on_motion(
                &mut self.editor,
                &mut self.sidebar,
                &self.input,
                cx,
                cy,
                wf,
                hf,
                s,
                !state.keyboard.alt,
            ),
            AppMode::Launcher => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_releases(&mut self, state: &mut State, gpu: &Gpu, cx: f32, cy: f32, wf: f32, hf: f32, s: f32) {
        if std::mem::take(&mut state.left_released) {
            match self.mode {
                AppMode::Canvas => {
                    canvas_input::on_release(&mut self.editor, &mut self.sidebar, cx, cy, wf, hf, s)
                }
                AppMode::Viewer => {
                    viewer_input::on_release(&mut self.app, &mut self.sidebar, gpu, cx, cy, wf, hf, s)
                }
                AppMode::Launcher => {}
            }
            self.app.is_panning = false;
            self.input.on_left_released();
        }
        if std::mem::take(&mut state.middle_released) {
            self.app.is_panning = false;
            if self.mode == AppMode::Canvas && matches!(self.editor.drag, DragMode::PanningCanvas { .. }) {
                self.editor.drag = DragMode::Idle;
            }
        }
    }

    /// Directional resize cursor over the window edges, plus each mode's
    /// own shapes elsewhere.
    fn update_cursor_shape(&mut self, state: &mut State, cx: f32, cy: f32, wf: f32, hf: f32, s: f32) {
        if !state.pointer_in_surface {
            return;
        }
        let border = crate::RESIZE_BORDER * s;
        let in_canvas = self.mode == AppMode::Canvas;
        let in_viewer = self.mode == AppMode::Viewer;
        let edge = cursor::edge_resize(cx, cy, wf, hf, border).filter(|_| !state.maximized);
        let desired = if (in_canvas || in_viewer) && self.sidebar.resizing {
            Shape::ColResize
        } else if let Some(edge) = edge {
            cursor::resize_edge_shape(edge)
        } else if in_canvas {
            cursor::canvas_shape(canvas_input::cursor_hint(&self.editor, &self.sidebar, cx, cy, wf, hf, s))
        } else if in_viewer && viewer_input::over_grip(&self.app, &self.sidebar, cx, cy, wf, hf, s) {
            Shape::ColResize
        } else {
            Shape::Default
        };
        if state.current_cursor_shape != Some(desired) {
            if let Some(dev) = &state.cursor_shape_device {
                dev.set_shape(state.enter_serial, desired);
            }
            state.current_cursor_shape = Some(desired);
        }
    }

    // ── Drawing ─────────────────────────────────────────────────────────

    pub fn render(&mut self, state: &State, gpu: &mut Gpu, toplevel: &XdgToplevel) {
        let s = state.fractional_scale() as f32;
        let (wf, hf) = (gpu.ctx.width() as f32, gpu.ctx.height() as f32);

        // Window title tracks the canvas name + dirty state.
        let title = match self.mode {
            AppMode::Canvas => self.editor.window_title(),
            _ => "Lantern Image Viewer".to_string(),
        };
        if title != self.last_title {
            toplevel.set_title(title.clone());
            self.last_title = title;
        }

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        // Read per frame so theme/accent changes apply on the next draw.
        let palette = FoxPalette::current();
        match self.mode {
            AppMode::Viewer => {
                viewer_input::sync_sidebar(&self.app, &mut self.sidebar, wf, hf, s);
                // Keep SVGs crisp at the size they're about to be drawn at.
                let canvas = crate::render::viewer_canvas(&self.app, &self.sidebar, wf, hf, s);
                self.app.maybe_rerender_svg(&gpu.ctx, &gpu.tex_pass, canvas);
                crate::render::render_frame(gpu, &self.app, &mut self.sidebar, &mut self.input, &palette, s, dt);
            }
            AppMode::Launcher => {
                render_launcher::render_launcher_frame(gpu, &mut self.launcher, &mut self.input, &palette, s)
            }
            AppMode::Canvas => crate::render_canvas::render_canvas_frame(
                gpu,
                &mut self.editor,
                &mut self.sidebar,
                &mut self.tex_cache,
                &mut self.input,
                &palette,
                s,
                dt,
            ),
        }
    }
}

/// When lantern.toml (theme, accent, background) last changed.
fn theme_stamp() -> Option<SystemTime> {
    let path = lntrn_theme::config::lantern_config_path()?;
    std::fs::metadata(path).ok()?.modified().ok()
}

//! winit application handler for Afterglow.
//!
//! Owns the window, GPU context, daemon client and the staged tuning state.
//! A timer polls telemetry on an interval; interaction is split between the
//! zone system (buttons/toggles/tabs) and direct geometry hit-testing for the
//! tuning sliders, which need drag.

mod draw;
mod monitor;
mod pending;
mod render;
mod tune;

use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{CursorIcon, ResizeDirection, Window, WindowAttributes, WindowId};

use lntrn_render::{GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use lntrn_afterglow::ipc::{Client, CpuTuning, Info, Snapshot};
use lntrn_afterglow::profile::{save as save_profile, Profile};
use lntrn_afterglow::config_path;

use pending::{
    Knob, Pending, ZONE_APPLY, ZONE_EPP, ZONE_GOV, ZONE_RESET, ZONE_SAVE, ZONE_TAB_MONITOR,
    ZONE_TAB_TUNE, ZONE_TGL_FAN, ZONE_TGL_LOCK, ZONE_TGL_TURBO,
};

pub const ZONE_CLOSE: u32 = 1;
pub const ZONE_MAXIMIZE: u32 = 2;
pub const ZONE_MINIMIZE: u32 = 3;

const POLL_INTERVAL: Duration = Duration::from_millis(750);
const INIT_W: f64 = 860.0;
const INIT_H: f64 = 760.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Monitor,
    Tune,
}

pub fn run() {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("event loop");
}

struct Gpu {
    ctx: GpuContext,
    painter: Painter,
    text: TextRenderer,
}

struct App {
    window: Option<Window>,
    gpu: Option<Gpu>,
    input: InteractionContext,
    palette: FoxPalette,
    bg_opacity: f32,
    scale: f32,
    needs_redraw: bool,

    client: Client,
    info: Option<Info>,
    snap: Option<Snapshot>,
    tuning: Option<CpuTuning>,
    status: String,

    tab: Tab,
    pending: Pending,
    dragging: Option<Knob>,
    tune_status: String,
}

impl App {
    fn new() -> App {
        App {
            window: None,
            gpu: None,
            input: InteractionContext::new(),
            palette: FoxPalette::current(),
            bg_opacity: lntrn_theme::background_opacity(),
            scale: 1.0,
            needs_redraw: true,
            client: Client::new(),
            info: None,
            snap: None,
            tuning: None,
            status: "connecting…".to_string(),
            tab: Tab::Monitor,
            pending: Pending::default(),
            dragging: None,
            tune_status: String::new(),
        }
    }

    fn poll(&mut self) {
        if self.info.is_none() {
            match self.client.info() {
                Ok(i) => self.info = Some(i),
                Err(e) => {
                    self.status = e;
                    self.snap = None;
                    self.tuning = None;
                    return;
                }
            }
        }
        match self.client.snapshot() {
            Ok(s) => self.snap = Some(s),
            Err(e) => {
                self.status = e;
                self.info = None;
                self.snap = None;
                return;
            }
        }
        if let Ok(t) = self.client.cpu_tuning() {
            self.tuning = Some(t);
        }

        // Seed staged values once we have everything (never clobbers edits).
        if !self.pending.initialized {
            if let (Some(i), Some(s)) = (&self.info, &self.snap) {
                self.pending.init_from(i, s, self.tuning.as_ref());
            }
        }

        let root = self.info.as_ref().map(|i| i.root).unwrap_or(false);
        self.status = if root {
            "connected".to_string()
        } else {
            "connected (read-only — daemon not root)".to_string()
        };
    }

    // ── Tuning actions ──────────────────────────────────────────────

    /// Push every staged knob to the daemon. Returns collected errors.
    fn apply_core(&mut self) -> Vec<String> {
        let Some(info) = self.info.clone() else {
            return vec!["not connected".to_string()];
        };
        let mut errs = Vec::new();
        for cmd in info_commands(&self.pending, &info) {
            if let Err(e) = self.client.command(&cmd) {
                errs.push(e);
            }
        }
        errs
    }

    fn action_apply(&mut self) {
        let errs = self.apply_core();
        self.tune_status = status_line("Applied", &errs);
    }

    fn action_save(&mut self) {
        let errs = self.apply_core();
        // Mark stable=1: the system is demonstrably up. The daemon disarms +
        // re-arms across boots, so a profile that wedges a boot falls back.
        let profile = self.pending.to_profile(true);
        let mut all = errs;
        if let Err(e) = save_profile(&config_path(), &profile) {
            all.push(format!("save: {e}"));
        }
        self.tune_status = status_line("Saved boot profile", &all);
    }

    fn action_reset(&mut self) {
        let mut errs = Vec::new();
        if let Err(e) = self.client.command("reset") {
            errs.push(e);
        }
        let _ = self.client.command("set max_freq_pct 100");
        let _ = self.client.command("set turbo 1");
        if let Some(info) = &self.info {
            let info = info.clone();
            self.pending.reset_to_stock(&info);
        }
        // Clear the boot profile so the next boot is stock.
        let _ = save_profile(&config_path(), &Profile::default());
        self.tune_status = status_line("Reset to stock", &errs);
    }

    fn cycle_governor(&mut self) {
        if let Some(t) = &self.tuning {
            self.pending.governor = next_in(&t.governors, &self.pending.governor);
        }
    }

    fn cycle_epp(&mut self) {
        if let Some(t) = &self.tuning {
            self.pending.epp = next_in(&t.epps, &self.pending.epp);
        }
    }

    // ── Geometry helpers ────────────────────────────────────────────

    fn window_size_f(&self) -> (f32, f32) {
        self.gpu
            .as_ref()
            .map_or((INIT_W as f32, INIT_H as f32), |g| {
                (g.ctx.width() as f32, g.ctx.height() as f32)
            })
    }

    /// The tuning slider whose track is under the cursor (and editable).
    fn slider_at(&self, cx: f32, cy: f32) -> Option<Knob> {
        let info = self.info.as_ref()?;
        let (wf, hf) = self.window_size_f();
        let lay = tune::layout(info, wf, hf, self.scale);
        lay.sliders
            .iter()
            .find(|(k, r)| pending::slider_editable(*k, &self.pending) && hit(*r, cx, cy))
            .map(|(k, _)| *k)
    }

    fn drag_slider(&mut self, cx: f32) {
        let Some(knob) = self.dragging else { return };
        let Some(info) = self.info.clone() else { return };
        let (wf, hf) = self.window_size_f();
        let lay = tune::layout(&info, wf, hf, self.scale);
        if let Some((_, track)) = lay.sliders.iter().find(|(k, _)| *k == knob) {
            let frac = ((cx - track.x) / track.w).clamp(0.0, 1.0);
            self.pending.set_slider(knob, frac, &info);
        }
    }

    fn edge_resize_direction(&self) -> Option<ResizeDirection> {
        let (cx, cy) = self.input.cursor()?;
        let (wf, hf) = self.window_size_f();
        let border = 9.0 * self.scale;
        let (left, right) = (cx < border, cx > wf - border);
        let (top, bottom) = (cy < border, cy > hf - border);
        match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        }
    }

    fn is_on_title_bar(&self) -> bool {
        self.input
            .cursor()
            .map_or(false, |(_, cy)| cy < draw::TITLE_BAR_H * self.scale)
    }

    fn handle_zone(&mut self, zone: u32, event_loop: &ActiveEventLoop) {
        match zone {
            ZONE_CLOSE => self.shutdown(event_loop),
            ZONE_MINIMIZE => {
                if let Some(w) = &self.window {
                    w.set_minimized(true);
                }
            }
            ZONE_MAXIMIZE => {
                if let Some(w) = &self.window {
                    w.set_maximized(!w.is_maximized());
                }
            }
            ZONE_TAB_MONITOR => self.tab = Tab::Monitor,
            ZONE_TAB_TUNE => self.tab = Tab::Tune,
            ZONE_TGL_FAN => self.pending.fan_manual = !self.pending.fan_manual,
            ZONE_TGL_LOCK => self.pending.lock_enabled = !self.pending.lock_enabled,
            ZONE_TGL_TURBO => self.pending.turbo = !self.pending.turbo,
            ZONE_GOV => self.cycle_governor(),
            ZONE_EPP => self.cycle_epp(),
            ZONE_APPLY => self.action_apply(),
            ZONE_SAVE => self.action_save(),
            ZONE_RESET => self.action_reset(),
            _ => {
                if self.is_on_title_bar() {
                    if let Some(w) = &self.window {
                        let _ = w.drag_window();
                    }
                }
            }
        }
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.gpu = None;
        self.window = None;
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let size = winit::dpi::LogicalSize::new(INIT_W, INIT_H);
        let attrs = WindowAttributes::default()
            .with_name("lntrn-afterglow", "lntrn-afterglow")
            .with_title("Afterglow")
            .with_inner_size(size)
            .with_decorations(false)
            .with_transparent(true);
        let window = event_loop.create_window(attrs).expect("create window");
        let _ = window.request_inner_size(size);
        self.scale = window.scale_factor() as f32;

        let phys = window.inner_size();
        let ctx = GpuContext::from_window(&window, phys.width, phys.height)
            .expect("create GPU context");
        self.gpu = Some(Gpu {
            painter: Painter::new(&ctx),
            text: TextRenderer::new(&ctx),
            ctx,
        });
        self.window = Some(window);

        self.poll();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL));
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            self.poll();
            self.needs_redraw = true;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.shutdown(event_loop),

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.needs_redraw = true;
            }

            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.ctx.resize(size.width, size.height);
                }
                self.needs_redraw = true;
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.input.on_cursor_moved(position.x as f32, position.y as f32);
                if self.dragging.is_some() {
                    self.drag_slider(position.x as f32);
                } else if let Some(w) = &self.window {
                    let cursor = self
                        .edge_resize_direction()
                        .map_or(CursorIcon::Default, CursorIcon::from);
                    w.set_cursor(cursor);
                }
                self.needs_redraw = true;
            }

            WindowEvent::CursorLeft { .. } => {
                self.input.on_cursor_left();
                self.needs_redraw = true;
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if let (MouseButton::Left, ElementState::Pressed) = (button, state) {
                    if let Some(dir) = self.edge_resize_direction() {
                        if let Some(w) = &self.window {
                            let _ = w.drag_resize_window(dir);
                        }
                        return;
                    }
                    // Tuning sliders are geometry-hit (need drag), checked first.
                    if self.tab == Tab::Tune {
                        if let Some((cx, cy)) = self.input.cursor() {
                            if let Some(knob) = self.slider_at(cx, cy) {
                                self.dragging = Some(knob);
                                self.drag_slider(cx);
                                self.needs_redraw = true;
                                return;
                            }
                        }
                    }
                    if let Some(zone) = self.input.on_left_pressed() {
                        self.handle_zone(zone, event_loop);
                    } else if self.is_on_title_bar() {
                        if let Some(w) = &self.window {
                            let _ = w.drag_window();
                        }
                        return;
                    }
                    self.needs_redraw = true;
                } else if let (MouseButton::Left, ElementState::Released) = (button, state) {
                    self.input.on_left_released();
                    self.dragging = None;
                    self.needs_redraw = true;
                }
            }

            WindowEvent::RedrawRequested => {
                self.palette = FoxPalette::current();
                self.bg_opacity = lntrn_theme::background_opacity();
                let cursor = self.input.cursor();
                let dragging = self.dragging;
                let tab = self.tab;
                let connected = self.client.is_connected();
                if let Some(gpu) = &mut self.gpu {
                    render::render_frame(
                        gpu,
                        &mut self.input,
                        &self.palette,
                        self.bg_opacity,
                        self.scale,
                        tab,
                        self.info.as_ref(),
                        self.snap.as_ref(),
                        self.tuning.as_ref(),
                        &self.pending,
                        cursor,
                        dragging,
                        &self.status,
                        &self.tune_status,
                        connected,
                    );
                }
                self.needs_redraw = false;
            }

            _ => {}
        }

        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

fn info_commands(p: &Pending, info: &Info) -> Vec<String> {
    p.set_commands(info)
}

fn next_in(list: &[String], current: &str) -> String {
    if list.is_empty() {
        return current.to_string();
    }
    let idx = list.iter().position(|v| v == current).unwrap_or(0);
    list[(idx + 1) % list.len()].clone()
}

fn hit(r: lntrn_render::Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

fn status_line(action: &str, errs: &[String]) -> String {
    if errs.is_empty() {
        format!("{action} ✓")
    } else {
        format!("error: {}", errs.join("; "))
    }
}

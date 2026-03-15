mod render;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowAttributes, WindowId};

use lntrn_render::{GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea};

// ── Hit zone IDs ────────────────────────────────────────────────────

pub const ZONE_CLOSE: u32 = 1;
pub const ZONE_MAXIMIZE: u32 = 2;
pub const ZONE_MINIMIZE: u32 = 3;
pub const ZONE_BTN_CREATE: u32 = 10;
pub const ZONE_BTN_PRUNE: u32 = 11;
pub const ZONE_BTN_ROLLBACK: u32 = 12;
pub const ZONE_BTN_DELETE: u32 = 13;
pub const ZONE_BTN_RENAME: u32 = 14;
pub const ZONE_SCROLLBAR: u32 = 20;
pub const ZONE_ROW_BASE: u32 = 100;

// ── Data ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    pub name: String,
    pub kind: String,
    pub date: String,
}

// ── GPU ─────────────────────────────────────────────────────────────

pub struct Gpu {
    pub ctx: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
}

// ── Main ────────────────────────────────────────────────────────────

fn main() {
    // Must be root for btrfs ioctls
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("error: lntrn-snapshot-gui requires root privileges");
        eprintln!("  try: sudo lntrn-snapshot-gui");
        std::process::exit(1);
    }

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut handler = SnapHandler::new();
    event_loop.run_app(&mut handler).expect("Event loop failed");
}

// ── Handler ─────────────────────────────────────────────────────────

struct SnapHandler {
    window: Option<Window>,
    gpu: Option<Gpu>,
    input: InteractionContext,
    palette: FoxPalette,
    scale: f32,
    needs_redraw: bool,
    snapshots: Vec<SnapshotEntry>,
    selected: Option<usize>,
    scroll_offset: f32,
    status_msg: String,
}

impl SnapHandler {
    fn new() -> Self {
        Self {
            window: None,
            gpu: None,
            input: InteractionContext::new(),
            palette: FoxPalette::dark(),
            scale: 1.0,
            needs_redraw: true,
            snapshots: Vec::new(),
            selected: None,
            scroll_offset: 0.0,
            status_msg: String::new(),
        }
    }

    fn refresh_list(&mut self) {
        self.snapshots = list_snapshots_cli();
        // Clamp selection
        if let Some(sel) = self.selected {
            if sel >= self.snapshots.len() {
                self.selected = if self.snapshots.is_empty() {
                    None
                } else {
                    Some(self.snapshots.len() - 1)
                };
            }
        }
        self.needs_redraw = true;
    }

    fn action_create(&mut self) {
        let output = run_snapshot_cmd(&["create"]);
        self.status_msg = output;
        self.refresh_list();
    }

    fn action_prune(&mut self) {
        let output = run_snapshot_cmd(&["prune"]);
        self.status_msg = output;
        self.refresh_list();
    }

    fn action_delete(&mut self) {
        if let Some(idx) = self.selected {
            if let Some(snap) = self.snapshots.get(idx) {
                let name = snap.name.clone();
                let output = run_snapshot_cmd(&["delete", &name]);
                self.status_msg = output;
                self.refresh_list();
            }
        }
    }

    fn action_rollback(&mut self) {
        if let Some(idx) = self.selected {
            if let Some(snap) = self.snapshots.get(idx) {
                let name = snap.name.clone();
                let output = run_snapshot_cmd(&["rollback", &name]);
                self.status_msg = output;
                self.refresh_list();
            }
        }
    }

    fn action_rename(&mut self) {
        if let Some(idx) = self.selected {
            if let Some(snap) = self.snapshots.get(idx) {
                let old_name = snap.name.clone();
                // Use zenity for input dialog
                let result = std::process::Command::new("zenity")
                    .args([
                        "--entry",
                        "--title=Rename Snapshot",
                        "--text=New name:",
                        &format!("--entry-text={}", old_name),
                    ])
                    .output();
                if let Ok(out) = result {
                    if out.status.success() {
                        let new_name = String::from_utf8_lossy(&out.stdout)
                            .trim()
                            .to_string();
                        if !new_name.is_empty() && new_name != old_name {
                            let output =
                                run_snapshot_cmd(&["rename", &old_name, &new_name]);
                            self.status_msg = output;
                            self.refresh_list();
                        }
                    }
                }
            }
        }
    }

    fn edge_resize_direction(&self) -> Option<ResizeDirection> {
        let (cx, cy) = self.input.cursor()?;
        let gpu = self.gpu.as_ref()?;
        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let border = 10.0 * self.scale;
        let left = cx < border;
        let right = cx > wf - border;
        let top = cy < border;
        let bottom = cy > hf - border;
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
            .map_or(false, |(_, cy)| cy < render::TITLE_BAR_H * self.scale)
    }

    fn window_size(&self) -> (f32, f32) {
        self.gpu
            .as_ref()
            .map_or((700.0, 500.0), |g| {
                (g.ctx.width() as f32, g.ctx.height() as f32)
            })
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.gpu = None;
        self.window = None;
        event_loop.exit();
    }
}

// ── Application handler ─────────────────────────────────────────────

impl ApplicationHandler for SnapHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("lntrn-snapshot")
            .with_inner_size(winit::dpi::LogicalSize::new(700.0, 500.0))
            .with_decorations(false)
            .with_transparent(true);

        let window = event_loop
            .create_window(attrs)
            .expect("Failed to create window");
        self.scale = window.scale_factor() as f32;

        let size = window.inner_size();
        let gpu_ctx = GpuContext::from_window(&window, size.width, size.height)
            .expect("Failed to create GPU context");

        self.gpu = Some(Gpu {
            painter: Painter::new(&gpu_ctx),
            text: TextRenderer::new(&gpu_ctx),
            ctx: gpu_ctx,
        });
        self.window = Some(window);

        // Load initial snapshot list
        self.refresh_list();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
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
                self.input
                    .on_cursor_moved(position.x as f32, position.y as f32);
                if let Some(dir) = self.edge_resize_direction() {
                    if let Some(w) = &self.window {
                        w.set_cursor(CursorIcon::from(dir));
                    }
                } else if let Some(w) = &self.window {
                    w.set_cursor(CursorIcon::Default);
                }
                self.needs_redraw = true;
            }

            WindowEvent::CursorLeft { .. } => {
                self.input.on_cursor_left();
                self.needs_redraw = true;
            }

            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    if let Some(dir) = self.edge_resize_direction() {
                        if let Some(w) = &self.window {
                            let _ = w.drag_resize_window(dir);
                        }
                        return;
                    }

                    if let Some(zone_id) = self.input.on_left_pressed() {
                        match zone_id {
                            ZONE_CLOSE => {
                                self.shutdown(event_loop);
                                return;
                            }
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
                            ZONE_BTN_CREATE => self.action_create(),
                            ZONE_BTN_PRUNE => self.action_prune(),
                            ZONE_BTN_ROLLBACK => self.action_rollback(),
                            ZONE_BTN_DELETE => self.action_delete(),
                            ZONE_BTN_RENAME => self.action_rename(),
                            id if id >= ZONE_ROW_BASE => {
                                let idx = (id - ZONE_ROW_BASE) as usize;
                                if idx < self.snapshots.len() {
                                    self.selected = Some(idx);
                                }
                            }
                            _ => {}
                        }
                    } else if self.is_on_title_bar() {
                        if let Some(w) = &self.window {
                            let _ = w.drag_window();
                        }
                        return;
                    }
                    self.needs_redraw = true;
                }
                (MouseButton::Left, ElementState::Released) => {
                    self.input.on_left_released();
                    self.needs_redraw = true;
                }
                _ => {}
            },

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0 * self.scale,
                    MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
                };
                let s = self.scale;
                let (wf, hf) = self.window_size();
                let viewport = render::list_viewport(wf, hf, s);
                let total_h = render::content_height(self.snapshots.len(), s);
                ScrollArea::apply_scroll(
                    &mut self.scroll_offset,
                    scroll,
                    total_h,
                    viewport.h,
                );
                self.needs_redraw = true;
            }

            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    render::render_frame(
                        gpu,
                        &mut self.input,
                        &self.palette,
                        self.scale,
                        &self.snapshots,
                        self.selected,
                        &mut self.scroll_offset,
                        &self.status_msg,
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

// ── CLI helpers ─────────────────────────────────────────────────────

fn find_cli_exe() -> std::path::PathBuf {
    // 1. Next to this binary
    if let Ok(self_exe) = std::env::current_exe() {
        if let Some(dir) = self_exe.parent() {
            let candidate = dir.join("lntrn-snapshot");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    // 2. ~/.local/bin (where we deploy)
    if let Ok(home) = std::env::var("HOME") {
        let candidate = std::path::PathBuf::from(home)
            .join(".local/bin/lntrn-snapshot");
        if candidate.exists() {
            return candidate;
        }
    }
    // 3. Hope it's in PATH
    std::path::PathBuf::from("lntrn-snapshot")
}

fn run_snapshot_cmd(args: &[&str]) -> String {
    let exe = find_cli_exe();

    match std::process::Command::new(&exe).args(args).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() {
                stdout.trim().to_string()
            } else {
                format!("error: {}", stderr.trim())
            }
        }
        Err(e) => format!("failed to run lntrn-snapshot: {}", e),
    }
}

fn list_snapshots_cli() -> Vec<SnapshotEntry> {
    let exe = find_cli_exe();

    let output = match std::process::Command::new(&exe).args(["list"]).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(_) => return Vec::new(),
    };

    // Parse the list output:
    //   manual-2026-03-11_143022  Manual   2026-03-11 14:30:22
    let mut entries = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("snapshots for")
            || line.starts_with("(none)")
        {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let name = parts[0].to_string();
            let kind = parts[1].to_string();
            // Remaining parts are the date
            let date = parts[2..].join(" ");
            entries.push(SnapshotEntry { name, kind, date });
        }
    }

    entries
}

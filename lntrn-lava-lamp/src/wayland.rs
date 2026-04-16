use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::Instant;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, Rect};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_pointer, wl_seat, wl_surface},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::simulation::LavaSimulation;
use crate::theme::Theme;

pub const BTN_LEFT: u32 = 0x110;
const KEY_ESC: u32 = 1;

// ── WaylandHandle for wgpu ─────────────────────────────────────────────

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

// ── Wayland state ──────────────────────────────────────────────────────

pub(crate) struct State {
    pub running: bool,
    pub configured: bool,
    pub frame_done: bool,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub output_phys_width: u32,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub surface: Option<wl_surface::WlSurface>,
    pub xdg_surface: Option<xdg_surface::XdgSurface>,
    pub toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub seat: Option<wl_seat::WlSeat>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    pub left_pressed: bool,
    pub pointer_serial: u32,
    pub enter_serial: u32,
    pub pointer_in_surface: bool,
    pub key_pressed: Option<u32>,
    pub decoration_mgr: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
}

impl State {
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
            wm_base: None,
            viewporter: None,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            seat: None,
            pointer: None,
            cursor_shape_mgr: None,
            cursor_shape_device: None,
            current_cursor_shape: None,
            left_pressed: false,
            pointer_serial: 0,
            enter_serial: 0,
            pointer_in_surface: false,
            key_pressed: None,
            decoration_mgr: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        // output_phys_width / window_width only works for full-width windows.
        // For a small widget, cap to a sane range; otherwise fall back to
        // the integer wl_output scale.
        if self.output_phys_width > 0 && self.width > 0 {
            let s = self.output_phys_width as f64 / self.width as f64;
            if s >= 1.0 && s <= 3.0 { return s; }
        }
        self.scale.max(1) as f64
    }

    fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Lamp geometry ──────────────────────────────────────────────────────

fn lamp_body(w: f32, h: f32) -> Rect {
    Rect::new(w * 0.10, h * 0.08, w * 0.80, h * 0.76)
}

fn lamp_base(w: f32, h: f32) -> Rect {
    Rect::new(w * 0.05, h * 0.83, w * 0.90, h * 0.13)
}

fn lamp_cap(w: f32, h: f32) -> Rect {
    Rect::new(w * 0.25, h * 0.02, w * 0.50, h * 0.07)
}

fn draw_lamp(painter: &mut Painter, sim: &LavaSimulation, theme: &Theme, w: f32, h: f32) {
    let body = lamp_body(w, h);
    let base = lamp_base(w, h);
    let cap = lamp_cap(w, h);
    let body_r = body.w * 0.20;
    let base_r = base.h * 0.3;
    let cap_r = cap.h * 0.4;

    // Base (behind everything)
    painter.rect_filled(base, base_r, theme.base_color);
    painter.rect_stroke_sdf(base, base_r, 1.0, theme.glass_border);

    // Glass body background
    painter.rect_filled(body, body_r, theme.glass_tint);

    // Subtle heat glow at the very bottom of the glass
    painter.push_clip(body);
    let glow_h = body.h * 0.18;
    let glow_rect = Rect::new(body.x, body.y + body.h - glow_h, body.w, glow_h);
    painter.rect_gradient_radial(glow_rect, body_r, theme.heat_glow, Color::TRANSPARENT);

    // Draw merge bridges between nearby blobs (behind the blobs)
    let blob_count = sim.blobs.len();
    for i in 0..blob_count {
        for j in (i + 1)..blob_count {
            let a = &sim.blobs[i];
            let b = &sim.blobs[j];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let dist = (dx * dx + dy * dy).sqrt();
            let touch = (a.radius + b.radius) * 1.3;
            if dist < touch && dist > 0.1 {
                let ca = theme.blob_colors[a.color_index % theme.blob_colors.len()];
                let cb = theme.blob_colors[b.color_index % theme.blob_colors.len()];
                let blend = ca.lerp(cb, 0.5);
                let strength = 1.0 - dist / touch;
                let br = a.radius.min(b.radius) * strength * 0.6;
                let mx = (a.x + b.x) * 0.5;
                let my = (a.y + b.y) * 0.5;
                painter.rect_filled(
                    Rect::new(mx - br, my - br, br * 2.0, br * 2.0),
                    br,
                    blend,
                );
            }
        }
    }

    // Draw blobs as organic deforming shapes
    for blob in &sim.blobs {
        let color = theme.blob_colors[blob.color_index % theme.blob_colors.len()];
        let r = blob.radius;

        // Soft glow behind the blob
        let glow_r = r * 1.6;
        let gr = Rect::new(blob.x - glow_r, blob.y - glow_r, glow_r * 2.0, glow_r * 2.0);
        painter.rect_gradient_radial(gr, 0.0, color.with_alpha(0.25), Color::TRANSPARENT);

        // Velocity-based stretch: blobs elongate in direction of motion
        let speed = (blob.vx * blob.vx + blob.vy * blob.vy).sqrt();
        let stretch = (speed * 0.08).min(0.35);

        // Build a deformed polygon outline — organic, blobby shape
        let n = 24;
        let phase = blob.wobble_phase;
        let tau = std::f32::consts::TAU;
        let points: Vec<(f32, f32)> = (0..n)
            .map(|k| {
                let angle = k as f32 * tau / n as f32;
                // Multi-frequency deformation for organic wobble
                let deform = 1.0
                    + 0.12 * (angle * 2.0 + phase * 1.3).sin()
                    + 0.08 * (angle * 3.0 - phase * 0.9).sin()
                    + 0.05 * (angle * 5.0 + phase * 1.7).sin();
                // Stretch in direction of motion
                let sx = 1.0 - stretch * 0.4;
                let sy = 1.0 + stretch * 0.6;
                let x = blob.x + angle.cos() * r * deform * sx;
                let y = blob.y + angle.sin() * r * deform * sy;
                (x, y)
            })
            .collect();
        painter.polygon(&points, color);

        // Inner highlight — smaller deformed polygon, offset up-left
        let hl_points: Vec<(f32, f32)> = (0..12)
            .map(|k| {
                let angle = k as f32 * tau / 12.0;
                let hr = r * 0.25;
                let hx = blob.x - r * 0.15;
                let hy = blob.y - r * 0.2;
                (hx + angle.cos() * hr, hy + angle.sin() * hr)
            })
            .collect();
        painter.polygon(&hl_points, Color::rgba(1.0, 1.0, 1.0, 0.12));
    }
    painter.pop_clip();

    // Subtle glass reflection on the left
    let refl = Rect::new(
        body.x + body.w * 0.08,
        body.y + body.h * 0.05,
        body.w * 0.12,
        body.h * 0.90,
    );
    painter.rect_filled(refl, refl.w * 0.5, Color::rgba(1.0, 1.0, 1.0, 0.03));

    // Glass border
    painter.rect_stroke_sdf(body, body_r, 1.0, theme.glass_border);

    // Cap on top
    painter.rect_filled(cap, cap_r, theme.cap_color);
    painter.rect_stroke_sdf(cap, cap_r, 1.0, theme.glass_border);
}

// ── Entry point ────────────────────────────────────────────────────────

pub fn run(blob_count: usize, theme_name: &str) -> Result<()> {
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
    let wm_base = state
        .wm_base
        .clone()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    if state.width == 0 { state.width = 300; }
    if state.height == 0 { state.height = 700; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lava Lamp".into());
    toplevel.set_app_id("lntrn-lava-lamp".into());

    // Client-side decorations (we draw none → borderless)
    if let Some(mgr) = &state.decoration_mgr {
        let deco = mgr.get_toplevel_decoration(&toplevel, &qh, ());
        deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
    }

    surface.commit();
    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

    // GPU setup
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

    let theme = Theme::by_name(theme_name);
    let mut sim = LavaSimulation::new(blob_count);
    let mut last_frame = Instant::now();

    // ── Render loop ────────────────────────────────────────────────────
    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[lava-lamp] dispatch error: {e}");
            break;
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        // Frame timing
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f32().min(0.1);
        last_frame = now;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.width() as f32;
        let hf = gpu.height() as f32;

        // Left click anywhere → drag to move the widget
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(seat) = &state.seat {
                toplevel._move(seat, state.pointer_serial);
            }
        }

        // Esc → quit
        if let Some(key) = state.key_pressed.take() {
            if key == KEY_ESC {
                state.running = false;
            }
        }

        // Physics step
        let body = lamp_body(wf, hf);
        sim.update(dt, body);

        // Draw
        painter.clear();
        draw_lamp(&mut painter, &sim, &theme, wf, hf);

        if let Ok(mut frame) = gpu.begin_frame("lava-lamp") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
            frame.submit(&gpu.queue);
        }

        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

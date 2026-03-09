use std::collections::HashMap;
use std::ffi::c_void;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::ptr::NonNull;
use std::time::Instant;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, GpuTexture, Painter, Rect, SurfaceError, TextRenderer, TexturePass, TextureDraw};
use lntrn_ui::gpu::FoxPalette;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_callback, wl_compositor, wl_output, wl_region, wl_registry, wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::svg_icon;

const ICON_DIR: &str = "/home/alva/.config/lntrn-bar/icons";

const OSD_W: u32 = 340;
const OSD_H: u32 = 64;
const CORNER_RADIUS: f32 = 16.0;
const DISPLAY_SECS: f32 = 2.0;
const FADE_SECS: f32 = 0.3;

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

struct State {
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
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: OSD_W, height: OSD_H,
            scale: 1, output_phys_width: 0,
            compositor: None, layer_shell: None, viewporter: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / (self.output_phys_width as f64 / self.scale.max(1) as f64)
        } else {
            self.scale.max(1) as f64
        }
    }

    fn phys_w(&self) -> u32 { (self.width as f64 * self.fractional_scale()).round() as u32 }
    fn phys_h(&self) -> u32 { (self.height as f64 * self.fractional_scale()).round() as u32 }
}

// ── Dispatch impls ──────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self, registry: &wl_registry::WlRegistry,
        event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for State {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(_: &mut Self, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => state.scale = factor,
            wl_output::Event::Mode { width, .. } => state.output_phys_width = width as u32,
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(state: &mut Self, _: &wl_callback::WlCallback, _: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        state.frame_done = true;
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self, layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer_surface.ack_configure(serial);
                if width > 0 { state.width = width; }
                if height > 0 { state.height = height; }
                state.configured = true;
                state.frame_done = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.running = false,
            _ => {}
        }
    }
}

// ── Icon helpers ────────────────────────────────────────────────────────────

fn icon_key(volume: u32, muted: bool) -> &'static str {
    if muted || volume == 0 { "snd-muted" }
    else if volume <= 33 { "snd-low" }
    else if volume <= 89 { "snd-medium" }
    else { "snd-high" }
}

const ICON_FILES: &[(&str, &str)] = &[
    ("snd-muted",  "spark-sound-muted.svg"),
    ("snd-low",    "spark-sound-low.svg"),
    ("snd-medium", "spark-sound-medium.svg"),
    ("snd-high",   "spark-sound-high.svg"),
];

fn bar_color(volume: u32, muted: bool) -> Color {
    if muted { return Color::rgba(0.4, 0.4, 0.4, 0.6); }
    let t = (volume as f32 / 100.0).clamp(0.0, 1.0);
    let r = 0.31 + t * 0.40;
    let g = 0.20 + t * 0.27;
    let b = 0.02 + t * 0.01;
    Color::rgba(r, g, b, 1.0)
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run(mut volume: u32, mut muted: bool, sock: UnixDatagram) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let layer_shell = state.layer_shell.as_ref()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let surface = compositor.create_surface(&qh, ());

    let layer_surface = layer_shell.get_layer_surface(
        &surface, None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "lntrn-osd".to_string(),
        &qh, (),
    );

    layer_surface.set_size(OSD_W, OSD_H);
    layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::empty());
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(
        zwlr_layer_surface_v1::KeyboardInteractivity::None,
    );

    let empty_region = compositor.create_region(&qh, ());
    surface.set_input_region(Some(&empty_region));

    surface.commit();
    event_queue.roundtrip(&mut state)?;

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }

    let scale_f = state.fractional_scale() as f32;

    let viewport = state.viewporter.as_ref().map(|vp| {
        let v = vp.get_viewport(&surface, &qh, ());
        v.set_destination(state.width as i32, state.height as i32);
        v
    });

    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let phys_w = state.phys_w().max(1);
    let phys_h = state.phys_h().max(1);
    let mut gpu = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let tex_pass = TexturePass::new(&gpu);
    let palette = FoxPalette::dark();

    // Pre-load all icon variants
    let icon_sz = (phys_h as f32 * 0.6) as u32;
    let mut icons: HashMap<&str, GpuTexture> = HashMap::new();
    let dir = Path::new(ICON_DIR);
    for &(key, file) in ICON_FILES {
        if let Some(tex) = svg_icon::load_svg(&tex_pass, &gpu, &dir.join(file), icon_sz, icon_sz) {
            icons.insert(key, tex);
        }
    }

    let mut last_update = Instant::now();
    let mut recv_buf = [0u8; 64];

    while state.running {
        event_queue.blocking_dispatch(&mut state)?;
        if !state.frame_done { continue; }
        state.frame_done = false;

        // Poll socket for updates (non-blocking)
        while let Ok(n) = sock.recv(&mut recv_buf) {
            if let Ok(msg) = std::str::from_utf8(&recv_buf[..n]) {
                let (v, m) = crate::parse_message(msg);
                volume = v;
                muted = m;
                last_update = Instant::now();
            }
        }

        let elapsed = last_update.elapsed().as_secs_f32();
        let total_time = DISPLAY_SECS + FADE_SECS;

        if elapsed > total_time {
            break;
        }

        let alpha = if elapsed > DISPLAY_SECS {
            1.0 - (elapsed - DISPLAY_SECS) / FADE_SECS
        } else {
            1.0
        };

        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_w().max(1), state.phys_h().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let pw = state.phys_w() as f32;
        let ph = state.phys_h() as f32;

        painter.clear();

        // Background pill
        let bg = Color::rgba(
            palette.surface.r, palette.surface.g, palette.surface.b,
            0.92 * alpha,
        );
        let radius = CORNER_RADIUS * scale_f;
        painter.rect_filled(Rect::new(0.0, 0.0, pw, ph), radius, bg);

        // Layout: [pad] [icon] [gap] [bar] [gap] [text] [pad]
        let pad = 16.0 * scale_f;
        let gap = 12.0 * scale_f;
        let icon_f = icon_sz as f32;
        let icon_x = pad;
        let icon_y = (ph - icon_f) / 2.0;

        let bar_x = icon_x + icon_f + gap;
        let bar_h = 8.0 * scale_f;
        let bar_y = (ph - bar_h) / 2.0;
        let text_w = 60.0 * scale_f;
        let bar_w = pw - bar_x - gap - text_w - pad;

        // Bar background
        let bar_bg = Color::rgba(1.0, 1.0, 1.0, 0.12 * alpha);
        painter.rect_filled(Rect::new(bar_x, bar_y, bar_w, bar_h), bar_h / 2.0, bar_bg);

        // Bar fill
        let fill_frac = if muted { 0.0 } else { (volume as f32 / 100.0).clamp(0.0, 1.0) };
        if fill_frac > 0.0 {
            let fill_w = bar_w * fill_frac;
            let fill_color = bar_color(volume, muted);
            let fc = Color::rgba(fill_color.r, fill_color.g, fill_color.b, alpha);
            painter.rect_filled(Rect::new(bar_x, bar_y, fill_w, bar_h), bar_h / 2.0, fc);
        }

        // Percentage text
        let font_size = 22.0 * scale_f;
        let label = if muted { "MUTE".to_string() } else { format!("{}%", volume) };
        let text_x = bar_x + bar_w + gap;
        let text_y = (ph - font_size) / 2.0;
        let text_color = Color::rgba(palette.text.r, palette.text.g, palette.text.b, alpha);
        text.queue(&label, font_size, text_x, text_y, text_color, pw, pw as u32, ph as u32);

        // Icon texture
        let mut tex_draws = Vec::new();
        let key = icon_key(volume, muted);
        if let Some(tex) = icons.get(key) {
            tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, icon_f, icon_f));
        }

        // Render
        match gpu.begin_frame("OSD") {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
                if !tex_draws.is_empty() {
                    tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
                }
                text.render_queued(&gpu, frame.encoder_mut(), &view);
                frame.submit(&gpu.queue);
            }
            Err(SurfaceError::Lost | SurfaceError::Outdated) => {
                gpu.resize(state.phys_w().max(1), state.phys_h().max(1));
            }
            Err(_) => {}
        }

        surface.frame(&qh, ());
        surface.commit();
    }

    // Clean up socket
    let _ = std::fs::remove_file(crate::SOCK_PATH);
    Ok(())
}

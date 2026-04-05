use std::ffi::c_void;
use std::os::fd::{AsFd, AsRawFd};
use std::ptr::NonNull;
use std::time::Instant;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, SurfaceError, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, ToastAnchor, ToastItem, ToastStack, ToastVariant};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use tokio::sync::mpsc;
use wayland_client::{
    protocol::{wl_callback, wl_compositor, wl_output, wl_region, wl_registry, wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::notifications::{NotifyEvent, Notification, Urgency};

const DISPLAY_SECS: f32 = 5.0;
const FADE_SECS: f32 = 0.4;
const SLIDE_IN_SECS: f32 = 0.5;
const SLIDE_OUT_SECS: f32 = 0.4;
const CRITICAL_DISPLAY_SECS: f32 = 10.0;

fn notification_sound_path() -> Option<std::path::PathBuf> {
    let xdg_data = std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".local/share")
        });
    let path = xdg_data.join("lantern/notification.mp3");
    if path.exists() { Some(path) } else { None }
}

fn play_notification_sound() {
    if let Some(path) = notification_sound_path() {
        std::thread::spawn(move || {
            let _ = std::process::Command::new("pw-play")
                .arg(&path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }
}

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
            width: 0, height: 0,
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

// ── Active notification tracking ────────────────────────────────────────────

struct ActiveNotification {
    toast: ToastItem,
    spawned: Instant,
    display_secs: f32,
    id: u32,
}

impl ActiveNotification {
    fn from_notification(notif: &Notification) -> Self {
        let variant = match notif.urgency {
            Urgency::Low => ToastVariant::Info,
            Urgency::Normal => ToastVariant::Info,
            Urgency::Critical => ToastVariant::Error,
        };

        let display = if notif.urgency == Urgency::Critical {
            CRITICAL_DISPLAY_SECS
        } else if notif.timeout_ms > 0 {
            (notif.timeout_ms as f32 / 1000.0).clamp(2.0, 30.0)
        } else {
            DISPLAY_SECS
        };

        Self {
            toast: ToastItem::new(notif.summary.clone(), notif.body.clone(), variant),
            spawned: Instant::now(),
            display_secs: display,
            id: notif.id,
        }
    }

    fn elapsed(&self) -> f32 {
        self.spawned.elapsed().as_secs_f32()
    }

    fn progress(&self) -> f32 {
        let elapsed = self.elapsed();
        (1.0 - elapsed / self.display_secs).clamp(0.0, 1.0)
    }

    /// 0.0 = off-screen, 1.0 = fully visible
    fn slide(&self) -> f32 {
        let elapsed = self.elapsed();
        if elapsed < SLIDE_IN_SECS {
            // Sliding in
            elapsed / SLIDE_IN_SECS
        } else if elapsed >= self.display_secs {
            // Sliding out
            let out_elapsed = elapsed - self.display_secs;
            (1.0 - out_elapsed / SLIDE_OUT_SECS).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    fn is_expired(&self) -> bool {
        self.elapsed() >= self.display_secs + SLIDE_OUT_SECS
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run(mut rx: mpsc::UnboundedReceiver<NotifyEvent>) -> Result<()> {
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
        "lntrn-notifyd".to_string(),
        &qh, (),
    );

    // Full-screen transparent overlay — 0 means fill
    layer_surface.set_size(0, 0);
    layer_surface.set_anchor(
        zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(
        zwlr_layer_surface_v1::KeyboardInteractivity::None,
    );

    // Pass-through input
    let empty_region = compositor.create_region(&qh, ());
    surface.set_input_region(Some(&empty_region));

    surface.commit();
    event_queue.roundtrip(&mut state)?;

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }

    // Fix the zero-width layer shell bug
    layer_surface.set_size(state.width, state.height);
    surface.commit();

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
    let palette = FoxPalette::dark();

    let mut active: Vec<ActiveNotification> = Vec::new();
    let mut needs_clear = true; // Force an initial transparent frame on first loop iteration

    // Set wayland fd to non-blocking for poll-based dispatch
    let wl_fd = conn.as_fd().as_raw_fd();
    {
        let flags = nix::fcntl::fcntl(wl_fd, nix::fcntl::FcntlArg::F_GETFL)
            .map_err(|e| anyhow!("fcntl get: {e}"))?;
        let mut oflags = nix::fcntl::OFlag::from_bits_truncate(flags);
        oflags |= nix::fcntl::OFlag::O_NONBLOCK;
        nix::fcntl::fcntl(wl_fd, nix::fcntl::FcntlArg::F_SETFL(oflags))
            .map_err(|e| anyhow!("fcntl set: {e}"))?;
    }

    while state.running {
        // Drain D-Bus events
        while let Ok(event) = rx.try_recv() {
            match event {
                NotifyEvent::Show(notif) => {
                    let is_new = !active.iter().any(|a| a.id == notif.id);
                    if let Some(existing) = active.iter_mut().find(|a| a.id == notif.id) {
                        *existing = ActiveNotification::from_notification(&notif);
                    } else {
                        active.push(ActiveNotification::from_notification(&notif));
                    }
                    if is_new {
                        play_notification_sound();
                    }
                }
                NotifyEvent::Close(id) => {
                    active.retain(|a| a.id != id);
                }
            }
        }

        // Remove expired
        let had_toasts = !active.is_empty();
        active.retain(|a| !a.is_expired());
        if had_toasts && active.is_empty() {
            needs_clear = true;
        }

        // Read + dispatch wayland events (non-blocking via prepare_read)
        if let Some(guard) = event_queue.prepare_read() {
            let _ = guard.read();
        }
        event_queue.dispatch_pending(&mut state)?;
        event_queue.flush()?;

        if active.is_empty() && !needs_clear {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        // ~30fps while toasts are visible
        std::thread::sleep(std::time::Duration::from_millis(33));

        if state.configured {
            state.configured = false;
            let pw = state.phys_w().max(1);
            let ph = state.phys_h().max(1);
            gpu.resize(pw, ph);
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let pw = state.phys_w();
        let ph = state.phys_h();

        // Build toast items with updated progress
        let toasts: Vec<ToastItem> = active.iter().map(|a| {
            let mut t = a.toast.clone();
            t.progress = a.progress();
            t.slide = a.slide();
            t
        }).collect();

        painter.clear();

        ToastStack::new(&toasts)
            .anchor(ToastAnchor::TopRight)
            .scale(scale_f)
            .margin(60.0)
            .draw(&mut painter, &mut text, &palette, pw, ph);

        match gpu.begin_frame("Notifications") {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::rgba(0.0, 0.0, 0.0, 0.0));
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
        needs_clear = false;
    }

    Ok(())
}

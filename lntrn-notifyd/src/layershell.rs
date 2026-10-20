mod active;
mod dispatch;

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::fd::{AsFd, AsRawFd};
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, SurfaceError, TextRenderer};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use tokio::sync::mpsc;
use wayland_client::{backend::ObjectId, protocol::wl_compositor, Connection, EventQueue, Proxy};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::notifications::{NotifyEvent, Outcome};
use crate::render::{Toast, ToastStack};
use active::{ActiveNotification, CLOSED_BY_CALL, DISMISSED, EXPIRED};

fn notification_sound_path() -> Option<std::path::PathBuf> {
    let sounds = lntrn_theme::lantern_home()?.join("sounds");
    // Prefer the lossless wav (cleaner, no encode clipping); fall back to the
    // legacy mp3 so machines that only have the old asset still chime.
    for name in ["notification.wav", "notification.mp3"] {
        let path = sounds.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn play_notification_sound(volume: f32) {
    if volume <= 0.0 {
        return;
    }
    if let Some(path) = notification_sound_path() {
        let vol = volume.clamp(0.0, 1.0);
        // One-shot jingle on its own rodio stream (same backend as the music
        // player). `OutputStream` isn't `Send`, so build it inside the thread
        // and hold it alive until playback finishes via `sleep_until_end`.
        std::thread::spawn(move || {
            let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
                return;
            };
            let Ok(sink) = rodio::Sink::try_new(&handle) else {
                return;
            };
            sink.set_volume(vol);
            let Ok(file) = std::fs::File::open(&path) else {
                return;
            };
            let Ok(source) = rodio::Decoder::new(std::io::BufReader::new(file)) else {
                return;
            };
            sink.append(source);
            sink.sleep_until_end();
        });
    }
}

/// Snapshot of notification settings, re-read from `lantern.toml` on each event.
struct NotifSettings {
    do_not_disturb: bool,
    show_toasts: bool,
    play_sound: bool,
    volume: f32,
    default_duration_secs: f32,
    position: crate::render::ToastPosition,
}

fn read_notif_settings() -> NotifSettings {
    let pos_str = lntrn_theme::read_config_string("notifications", "position", "top-right");
    NotifSettings {
        do_not_disturb: lntrn_theme::read_config_bool("notifications", "do_not_disturb", false),
        show_toasts: lntrn_theme::read_config_bool("notifications", "show_toasts", true),
        play_sound: lntrn_theme::read_config_bool("notifications", "play_sound", true),
        volume: lntrn_theme::read_config_f32("notifications", "volume", 0.8),
        default_duration_secs: lntrn_theme::read_config_f32(
            "notifications",
            "default_duration_secs",
            5.0,
        )
        .clamp(1.0, 30.0),
        position: crate::render::ToastPosition::from_str(&pos_str),
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

/// Physical data for one wl_output. Tracked per-output (keyed by proxy id)
/// so another monitor's Mode/Scale events can't clobber the scale of the
/// output our surface is actually on.
#[derive(Default)]
struct OutputData {
    /// wl_registry global name, so GlobalRemove can evict the right entry.
    registry_name: u32,
    /// Physical mode width in pixels (wl_output::Event::Mode).
    phys_width: u32,
    /// Integer scale (wl_output::Event::Scale).
    scale: i32,
}

struct State {
    running: bool,
    configured: bool,
    frame_done: bool,
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
    // Pointer state — logical surface coords; click queued for main-loop hit-test.
    pointer_x: f64,
    pointer_y: f64,
    /// The pointer is over our surface (a toast's click region).
    pointer_in: bool,
    pending_click: Option<(f64, f64)>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: true,
            width: 0,
            height: 0,
            outputs: HashMap::new(),
            current_output: None,
            fallback_scale: 1,
            compositor: None,
            layer_shell: None,
            viewporter: None,
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_in: false,
            pending_click: None,
        }
    }

    /// The live scale of the output our surface is on. The overlay spans the
    /// whole output, so physical mode width over configured logical width is
    /// the exact fractional scale — and it follows every configure, so a
    /// scale change mid-session (Settings slider, Gaming Mode) is picked up
    /// without a restart.
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

    fn phys_w(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    fn phys_h(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Show the notifications `rx` brings, telling `out` what became of each.
pub fn run(mut rx: mpsc::UnboundedReceiver<NotifyEvent>, out: mpsc::UnboundedSender<Outcome>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let layer_shell = state
        .layer_shell
        .as_ref()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let surface = compositor.create_surface(&qh, ());

    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "lntrn-notifyd".to_string(),
        &qh,
        (),
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
    layer_surface.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);

    // Pass-through input
    let empty_region = compositor.create_region(&qh, ());
    surface.set_input_region(Some(&empty_region));

    surface.commit();
    event_queue.roundtrip(&mut state)?;

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }

    // The size stays (0, 0) = "fill the output": the compositor re-sizes us
    // whenever the output's logical size changes. Pinning the first configured
    // size here froze the overlay at the startup scale's dimensions.

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
    // Logical + physical size the swapchain and viewport were last set up
    // for. Compared every frame, because the physical size also moves when
    // only the output's scale or mode changes.
    let mut applied_size = (state.width, state.height, phys_w, phys_h);
    state.configured = false;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);

    let mut active: Vec<ActiveNotification> = Vec::new();
    // The frame on screen may show toasts: true until one with none has
    // been presented, so the overlay starts transparent, and a toast that
    // goes (however it goes) never stays drawn after it.
    let mut toasts_shown = true;

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
        // Settings snapshot — re-read each loop iteration so live config
        // changes (DND, duration, position) apply immediately.
        let settings = read_notif_settings();

        // Drain D-Bus events
        while let Ok(event) = rx.try_recv() {
            match event {
                NotifyEvent::Show(notif) => {
                    if settings.do_not_disturb || !settings.show_toasts {
                        // DND is on (or toasts are off): dropped. Apps still
                        // get their id, and hear it's gone at once, so one
                        // waiting on an action isn't left waiting.
                        if !active.iter().any(|a| a.id == notif.id) {
                            let _ = out.send(Outcome::Closed(notif.id, EXPIRED));
                        }
                        continue;
                    }
                    let is_new = !active.iter().any(|a| a.id == notif.id);
                    if settings.show_toasts {
                        if let Some(existing) = active.iter_mut().find(|a| a.id == notif.id) {
                            *existing = ActiveNotification::from_notification(
                                &notif,
                                settings.default_duration_secs,
                            );
                        } else {
                            active.push(ActiveNotification::from_notification(
                                &notif,
                                settings.default_duration_secs,
                            ));
                        }
                    }
                    if is_new && settings.play_sound {
                        play_notification_sound(settings.volume);
                    }
                }
                NotifyEvent::Close(id) => {
                    // Slides out like a dismissed one; the app hears it
                    // closed once it's gone.
                    if let Some(a) = active.iter_mut().find(|a| a.id == id) {
                        a.dismiss(CLOSED_BY_CALL);
                    }
                }
            }
        }

        // Read + dispatch wayland events (non-blocking via prepare_read)
        if let Some(guard) = event_queue.prepare_read() {
            let _ = guard.read();
        }
        event_queue.dispatch_pending(&mut state)?;

        // What the pointer is over, and a queued click on it: hit-tested
        // in logical surface coords.
        {
            let toasts_hit: Vec<Toast> = active.iter().map(|a| a.to_toast()).collect();
            let stack_hit = ToastStack::new(&toasts_hit)
                .scale(1.0)
                .margin(60.0)
                .position(settings.position)
                .screen_h(state.height as f32);
            let sw = state.width as f32;
            let pointer = (state.pointer_x as f32, state.pointer_y as f32);
            for (i, a) in active.iter_mut().enumerate() {
                a.hover = if state.pointer_in { stack_hit.hit(i, &toasts_hit[i], sw, pointer, &mut text) } else { None };
            }
            if let Some((px, py)) = state.pending_click.take() {
                for (i, a) in active.iter_mut().enumerate() {
                    let Some(hit) = stack_hit.hit(i, &toasts_hit[i], sw, (px as f32, py as f32), &mut text) else { continue };
                    if let Some(key) = a.action_of(hit) {
                        let _ = out.send(Outcome::Action(a.id, key));
                    }
                    a.dismiss(DISMISSED);
                    break;
                }
            }
        }

        // Remove expired (after click handling so dismissed toasts still draw their slide-out)
        for a in active.iter().filter(|a| a.is_expired()) {
            let _ = out.send(Outcome::Closed(a.id, a.reason.unwrap_or(EXPIRED)));
        }
        active.retain(|a| !a.is_expired());

        event_queue.flush()?;

        if active.is_empty() && !toasts_shown {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        // ~30fps while toasts are visible
        std::thread::sleep(std::time::Duration::from_millis(33));

        let scale_f = state.fractional_scale() as f32;
        let pw = state.phys_w().max(1);
        let ph = state.phys_h().max(1);
        let size = (state.width, state.height, pw, ph);
        if state.configured || size != applied_size {
            state.configured = false;
            applied_size = size;
            gpu.resize(pw, ph);
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        // Build toast snapshots with current progress + slide.
        let toasts: Vec<Toast> = active.iter().map(|a| a.to_toast()).collect();

        painter.clear();

        ToastStack::new(&toasts)
            .scale(scale_f)
            .margin(60.0)
            .position(settings.position)
            .screen_h(ph as f32)
            .draw(&mut painter, &mut text, pw, ph);

        // Update input region to where toasts take clicks (logical coords):
        // close X's and buttons, all of one with a default action. The rest
        // of the layer is pass-through.
        let region = state.compositor.as_ref().unwrap().create_region(&qh, ());
        let logical_stack = ToastStack::new(&toasts)
            .scale(1.0)
            .margin(60.0)
            .position(settings.position)
            .screen_h(state.height as f32);
        let sw = state.width as f32;
        for (i, t) in toasts.iter().enumerate() {
            for r in logical_stack.hit_rects(i, t, sw, &mut text) {
                region.add(r.x as i32, r.y as i32, r.w.ceil() as i32, r.h.ceil() as i32);
            }
        }
        surface.set_input_region(Some(&region));
        region.destroy();

        match gpu.begin_frame("Notifications") {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_pass(
                    &gpu,
                    frame.encoder_mut(),
                    &view,
                    Color::rgba(0.0, 0.0, 0.0, 0.0),
                );
                text.render_queued(&gpu, frame.encoder_mut(), &view);
                frame.submit(&gpu.queue);
                toasts_shown = !toasts.is_empty();
            }
            Err(SurfaceError::Lost | SurfaceError::Outdated) => {
                gpu.resize(pw, ph);
            }
            Err(_) => {}
        }

        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

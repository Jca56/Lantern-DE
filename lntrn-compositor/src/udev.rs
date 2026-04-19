use std::{collections::HashMap, time::{Duration, Instant}};

use smithay::{
    backend::{
        allocator::{
            gbm::GbmAllocator,
            Fourcc,
        },
        drm::{
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager},
            DrmDeviceFd, DrmNode, NodeType,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            gles::{GlesPixelProgram, GlesRenderer, GlesTexProgram},
            ImportMemWl,
        },
        session::{
            libseat::LibSeatSession,
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, RegistrationToken,
        },
        drm::control::crtc,
        input::Libinput,
        wayland_server::DisplayHandle,
    },
};
use smithay::utils::IsAlive;
use smithay_drm_extras::drm_scanner::DrmScanner;
use tracing::{error, info, trace, warn};

use crate::render::render_surface;
use crate::udev_device::{device_added, device_changed, device_removed, render_device};
use crate::Lantern;

pub const BG_COLOR: [f32; 4] = [0.094, 0.094, 0.094, 1.0];
pub(crate) const SUPPORTED_FORMATS: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Abgr8888];
pub const RENDER_INTERVAL: Duration = Duration::from_millis(16);
const POINTER_RENDER_INTERVAL: Duration = Duration::from_millis(16);
/// Compositor output scale (reads [display] scale from lantern.toml, defaults 1.0).
pub(crate) fn lantern_output_scale() -> f64 { crate::output_scale() }

pub fn frame_callback_interval(output: &smithay::output::Output) -> Duration {
    let refresh = output.current_mode().map(|mode| mode.refresh).unwrap_or(60_000);
    let refresh = u64::try_from(refresh.max(1)).unwrap_or(60_000);
    Duration::from_nanos(1_000_000_000u64 / refresh)
}

pub(crate) struct OutputSurface {
    #[allow(dead_code)] // stored for future multi-GPU identification
    pub device_id: DrmNode,
    pub global: smithay::reexports::wayland_server::backend::GlobalId,
    pub drm_output: DrmOutput<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        (),
        DrmDeviceFd,
    >,
    pub frame_pending: bool,
    pub pending_render: bool,
    pub pending_interval: Duration,
    pub cooldown_until: Instant,
}

pub(crate) struct GpuBackend {
    pub(crate) drm_output_manager: DrmOutputManager<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        (),
        DrmDeviceFd,
    >,
    pub(crate) drm_scanner: DrmScanner,
    pub surfaces: HashMap<crtc::Handle, OutputSurface>,
    #[allow(dead_code)] // must stay alive to keep DRM event source registered
    pub(crate) drm_registration: RegistrationToken,
}

pub struct UdevData {
    pub session: LibSeatSession,
    pub(crate) primary_gpu: DrmNode,
    pub renderer: Option<GlesRenderer>,
    pub(crate) backends: HashMap<DrmNode, GpuBackend>,
    pub(crate) display_handle: DisplayHandle,
    pub corner_shader: Option<GlesPixelProgram>,
    pub shadow_shader: Option<GlesPixelProgram>,
    pub hot_corner_glow_shader: Option<GlesPixelProgram>,
    pub ssd_icon_shader: Option<GlesPixelProgram>,
    pub ssd_header_shader: Option<GlesPixelProgram>,
    pub rounded_tex_shader: Option<GlesTexProgram>,
    pub blur_down_shader: Option<GlesTexProgram>,
    pub blur_up_shader: Option<GlesTexProgram>,
    pub backdrop_shader: Option<GlesTexProgram>,
    pub blur_state: Option<crate::blur::BlurState>,
    /// One-shot timer token for demand-driven rendering.
    /// When a render is scheduled, we insert a timer to flush it;
    /// `None` means no timer is pending (idle — zero CPU).
    pub(crate) render_timer: Option<smithay::reexports::calloop::RegistrationToken>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct UdevOutputId {
    pub device_id: DrmNode,
    pub crtc: crtc::Handle,
}

/// All DRM modes available for a given output (stored as Output user data).
pub(crate) struct UdevOutputModes {
    pub drm_modes: Vec<smithay::reexports::drm::control::Mode>,
    pub connector_handle: smithay::reexports::drm::control::connector::Handle,
}

pub fn init_udev(
    event_loop: &mut EventLoop<'static, Lantern>,
    state: &mut Lantern,
) -> Result<(), Box<dyn std::error::Error>> {
    let (session, notifier) = LibSeatSession::new()
        .map_err(|e| format!("Failed to initialize session: {}", e))?;

    let seat_name = session.seat();
    info!("Session on seat: {}", seat_name);

    let primary_gpu_path = primary_gpu(&seat_name)?
        .or_else(|| all_gpus(&seat_name).ok()?.into_iter().next())
        .ok_or("No GPU found")?;
    let primary_gpu = DrmNode::from_path(&primary_gpu_path)?
        .node_with_type(NodeType::Render)
        .and_then(|r| r.ok())
        .unwrap_or_else(|| {
            DrmNode::from_path(&primary_gpu_path).expect("Failed to get DRM node")
        });
    info!("Using {} as primary GPU", primary_gpu);

    let udev_data = UdevData {
        session: session.clone(),
        primary_gpu,
        renderer: None,
        backends: HashMap::new(),
        display_handle: state.display_handle.clone(),
        corner_shader: None,
        shadow_shader: None,
        hot_corner_glow_shader: None,
        ssd_icon_shader: None,
        ssd_header_shader: None,
        rounded_tex_shader: None,
        blur_down_shader: None,
        blur_up_shader: None,
        backdrop_shader: None,
        blur_state: None,
        render_timer: None,
    };
    state.udev = Some(udev_data);

    let udev_backend = UdevBackend::new(&seat_name)?;
    info!("Enumerating DRM devices...");

    for (device_id, path) in udev_backend.device_list() {
        info!("Found device: {:?} at {:?}", device_id, path);
        if let Ok(node) = DrmNode::from_dev_id(device_id) {
            if let Err(e) = device_added(state, node, &path) {
                error!("Failed to add device {}: {}", node, e);
            }
        }
    }

    // Update SHM formats now that the renderer exists
    if let Some(udev) = state.udev.as_ref() {
        if let Some(renderer) = udev.renderer.as_ref() {
            state.shm_state.update_formats(renderer.shm_formats());
        }
    }

    // Initialize linux-dmabuf global now that we have a renderer
    if let Some(udev) = state.udev.as_ref() {
        if let Some(renderer) = udev.renderer.as_ref() {
            use smithay::backend::renderer::ImportDma;
            use smithay::wayland::dmabuf::DmabufFeedbackBuilder;

            let render_node = udev.primary_gpu
                .node_with_type(NodeType::Render)
                .and_then(|r| r.ok())
                .unwrap_or(udev.primary_gpu);

            let dmabuf_formats = renderer.dmabuf_formats();
            let default_feedback = DmabufFeedbackBuilder::new(
                render_node.dev_id(),
                dmabuf_formats,
            )
            .build()
            .expect("Failed to build dmabuf feedback");

            let dmabuf_global = state.dmabuf_state
                .create_global_with_default_feedback::<Lantern>(
                    &state.display_handle,
                    &default_feedback,
                );
            state.dmabuf_global = Some(dmabuf_global);
            info!("linux-dmabuf global initialized");
        }
    }

    // Verify we have at least one working output
    let has_outputs = state
        .udev
        .as_ref()
        .map(|u| u.backends.values().any(|b| !b.surfaces.is_empty()))
        .unwrap_or(false);
    if !has_outputs {
        error!("No DRM outputs could be initialized! Cannot continue.");
        return Err("No DRM outputs initialized. Check GPU permissions and DRM device access.".into());
    }
    info!("DRM outputs initialized successfully");

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
            session.clone().into(),
        );
    libinput_context.udev_assign_seat(&seat_name).unwrap();
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    event_loop
        .handle()
        .insert_source(libinput_backend, move |event, _, state| {
            state.process_input_event(event);
        })?;

    event_loop
        .handle()
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                info!("Session paused");
                libinput_context.suspend();
                if let Some(udev) = state.udev.as_mut() {
                    for backend in udev.backends.values_mut() {
                        backend.drm_output_manager.pause();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                info!("Session resumed");
                if let Err(e) = libinput_context.resume() {
                    error!("Failed to resume libinput: {:?}", e);
                }
                if let Some(udev) = state.udev.as_mut() {
                    for (_node, backend) in udev.backends.iter_mut() {
                        backend
                            .drm_output_manager
                            .activate(false)
                            .expect("Failed to activate DRM backend");
                    }
                    let nodes: Vec<_> = udev.backends.keys().copied().collect();
                    for node in nodes {
                        render_device(state, node, None);
                    }
                }
            }
        })?;

    event_loop
        .handle()
        .insert_source(udev_backend, move |event, _, state| match event {
            UdevEvent::Added { device_id, path } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    if let Err(e) = device_added(state, node, &path) {
                        error!("Failed to add device {}: {}", node, e);
                    }
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_changed(state, node);
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_removed(state, node);
                }
            }
        })?;

    event_loop.run(None, state, |state| {
        // Handle dead windows: animate client-initiated closes, clean up compositor-initiated ones.
        let dead_windows: Vec<_> = state.space.elements()
            .filter(|w| !w.alive())
            .filter_map(|w| {
                let surface = crate::window_ext::WindowExt::get_wl_surface(w)?;
                let location = state.space.element_location(w)?;
                let size = w.geometry().size;
                let had_ssd = state.ssd.has_ssd(&surface);
                Some(crate::animation::ClosingWindow { surface, location, size, had_ssd })
            })
            .collect();
        for cw in dead_windows {
            if state.animations.take_close_done(&cw.surface) {
                // Compositor-initiated close (Super+Q) already animated — just clean up
                state.forget_window(&cw.surface);
            } else {
                // Client-initiated close — start zombie close animation
                let surface = cw.surface.clone();
                state.animations.start_close(&surface);
                state.closing_windows.push(cw);
                state.schedule_render();
            }
        }
        state.space.refresh();
        state.popups.cleanup();
        state.layer_surfaces.retain(|ls| ls.alive());
        state.check_exclusive_zone_change();
        state.tick_audio_repeat();
        crate::reap_zombies();
        let _ = state.display_handle.flush_clients();
    })?;

    Ok(())
}

pub(crate) fn frame_finish(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {

    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    let surface = match backend.surfaces.get_mut(&crtc) {
        Some(s) => s,
        None => return,
    };

    surface.frame_pending = false;

    trace!("vblank: frame_submitted starting");
    let submit_result = surface.drm_output.frame_submitted();
    trace!("vblank: frame_submitted done");

    match submit_result {
        Ok(_) => {}
        Err(e) => {
            warn!("Frame submit error: {:?}", e);
        }
    };

    if surface.pending_render && Instant::now() >= surface.cooldown_until {
        render_surface(state, node, crtc);
    }
}

/// Trigger a re-render on all outputs (e.g. after cursor movement)
pub fn schedule_render_all(state: &mut Lantern) {
    schedule_render(state, false);
}

/// Force a re-render even if a frame is pending (for cursor motion)
pub fn schedule_render_forced(state: &mut Lantern) {
    schedule_render(state, true);
}

fn schedule_render(state: &mut Lantern, force: bool) {
    let interval = if state.pending_client_frame_callbacks {
        RENDER_INTERVAL
    } else {
        POINTER_RENDER_INTERVAL
    };

    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    if force {
        // Forced renders (e.g., Super+Shift+R) render immediately.
        let mut targets = Vec::new();
        for (node, backend) in &mut udev.backends {
            for (crtc, surface) in &mut backend.surfaces {
                surface.pending_render = true;
                surface.pending_interval = surface.pending_interval.min(interval);
                targets.push((*node, *crtc));
            }
        }

        for (node, crtc) in targets {
            render_surface(state, node, crtc);
        }
    } else {
        // Normal path: set flags and ensure a one-shot timer will flush them.
        // This prevents mouse motion events from blocking on GPU rendering
        // while also avoiding a permanent polling timer that wastes CPU at idle.
        let needs_timer = udev.render_timer.is_none();
        for (_, backend) in &mut udev.backends {
            for (_, surface) in &mut backend.surfaces {
                surface.pending_render = true;
                surface.pending_interval = surface.pending_interval.min(interval);
            }
        }

        if needs_timer {
            let token = state.loop_handle.insert_source(
                Timer::from_duration(interval),
                |_, _, state| {
                    // Clear the token so the next schedule_render can insert a new one
                    if let Some(udev) = state.udev.as_mut() {
                        udev.render_timer = None;
                    }
                    flush_pending_renders(state, false);
                    TimeoutAction::Drop
                },
            );
            if let Ok(token) = token {
                if let Some(udev) = state.udev.as_mut() {
                    udev.render_timer = Some(token);
                }
            }
        }
    }
}

fn flush_pending_renders(state: &mut Lantern, force: bool) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let mut targets = Vec::new();
    let now = Instant::now();
    let mut earliest_retry: Option<Instant> = None;
    for (node, backend) in &mut udev.backends {
        for (crtc, surface) in &mut backend.surfaces {
            if surface.pending_render {
                if force || (!surface.frame_pending && now >= surface.cooldown_until) {
                    targets.push((*node, *crtc));
                } else if !surface.frame_pending {
                    // Still in cooldown — remember the earliest retry time
                    earliest_retry = Some(match earliest_retry {
                        Some(t) => t.min(surface.cooldown_until),
                        None => surface.cooldown_until,
                    });
                }
                // If frame_pending, VBlank handler will pick it up — no timer needed
            }
        }
    }

    for (node, crtc) in targets {
        render_surface(state, node, crtc);
    }

    // If surfaces remain pending (in cooldown), schedule another one-shot timer
    if let Some(retry_at) = earliest_retry {
        if let Some(udev) = state.udev.as_mut() {
            if udev.render_timer.is_none() {
                let delay = retry_at.saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1));
                let token = state.loop_handle.insert_source(
                    Timer::from_duration(delay),
                    |_, _, state| {
                        if let Some(udev) = state.udev.as_mut() {
                            udev.render_timer = None;
                        }
                        flush_pending_renders(state, false);
                        TimeoutAction::Drop
                    },
                );
                if let Ok(token) = token {
                    if let Some(udev) = state.udev.as_mut() {
                        udev.render_timer = Some(token);
                    }
                }
            }
        }
    }
}


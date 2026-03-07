use std::{collections::HashMap, path::Path, time::Duration};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
        },
        egl::{context::ContextPriority, EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                memory::MemoryRenderBufferRenderElement,
                render_elements,
                surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
            ImportMemWl,
        },
        session::{
            libseat::LibSeatSession,
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    desktop::space::SpaceRenderElements,
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::{EventLoop, RegistrationToken},
        drm::control::{connector, crtc, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::DisplayHandle,
    },
    utils::{DeviceFd, Physical, Point},
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use tracing::{error, info, warn};

use crate::Lantern;

const BG_COLOR: [f32; 4] = [0.094, 0.094, 0.094, 1.0];
const SUPPORTED_FORMATS: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Abgr8888];

// Combined render element enum: space windows + cursor overlay
render_elements! {
    CustomRenderElements<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
}

struct OutputSurface {
    device_id: DrmNode,
    global: smithay::reexports::wayland_server::backend::GlobalId,
    drm_output: DrmOutput<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        (),
        DrmDeviceFd,
    >,
    frame_pending: bool,
}

struct GpuBackend {
    drm_output_manager: DrmOutputManager<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        (),
        DrmDeviceFd,
    >,
    drm_scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, OutputSurface>,
    drm_registration: RegistrationToken,
}

pub struct UdevData {
    pub session: LibSeatSession,
    primary_gpu: DrmNode,
    pub renderer: Option<GlesRenderer>,
    backends: HashMap<DrmNode, GpuBackend>,
    display_handle: DisplayHandle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct UdevOutputId {
    device_id: DrmNode,
    crtc: crtc::Handle,
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
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.display_handle.flush_clients();
    })?;

    Ok(())
}

fn device_added(
    state: &mut Lantern,
    node: DrmNode,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Adding DRM device: {} at {:?}", node, path);

    let fd = {
        let udev = state.udev.as_mut().ok_or("No udev data")?;
        udev.session
            .open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY)
            .map_err(|e| format!("Failed to open device {:?}: {}", path, e))?
    };
    info!("Opened DRM device fd");

    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)
        .map_err(|e| format!("DrmDevice::new failed: {}", e))?;
    info!("DRM device created");

    let gbm = GbmDevice::new(fd.clone())
        .map_err(|e| format!("GbmDevice::new failed: {}", e))?;
    info!("GBM device created");

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    let framebuffer_exporter = GbmFramebufferExporter::new(gbm.clone(), None);

    // Create EGL display and renderer from the SAME GBM device used for DRM output
    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }
        .map_err(|e| format!("EGLDisplay::new failed: {}", e))?;
    info!("EGL display created");

    let context = EGLContext::new_with_priority(&egl_display, ContextPriority::High)
        .map_err(|e| format!("EGLContext::new failed: {}", e))?;

    let render_formats = context
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect::<Vec<_>>();

    let renderer = unsafe { GlesRenderer::new(context) }
        .map_err(|e| format!("GlesRenderer::new failed: {}", e))?;
    info!("GLES renderer created from DRM device");

    let drm_output_manager = DrmOutputManager::new(
        drm,
        allocator,
        framebuffer_exporter,
        Some(gbm),
        SUPPORTED_FORMATS.iter().copied(),
        render_formats,
    );
    info!("DRM output manager created");

    let drm_registration = state
        .loop_handle
        .insert_source(drm_notifier, move |event, _metadata, state| match event {
            DrmEvent::VBlank(crtc) => {
                frame_finish(state, node, crtc);
            }
            DrmEvent::Error(err) => {
                error!("DRM error: {:?}", err);
            }
        })?;

    let udev = state.udev.as_mut().ok_or("No udev data")?;
    // Store the renderer (replaces any previous one for multi-GPU, MVP uses one)
    udev.renderer = Some(renderer);
    // Keep EGL display alive by leaking it (it must outlive the renderer)
    std::mem::forget(egl_display);
    udev.backends.insert(
        node,
        GpuBackend {
            drm_output_manager,
            drm_scanner: DrmScanner::new(),
            surfaces: HashMap::new(),
            drm_registration,
        },
    );
    info!("DRM backend registered for {}", node);

    device_changed(state, node);

    Ok(())
}

fn device_changed(state: &mut Lantern, node: DrmNode) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    let scan_result = match backend
        .drm_scanner
        .scan_connectors(backend.drm_output_manager.device())
    {
        Ok(res) => res,
        Err(e) => {
            warn!("Failed to scan connectors: {:?}", e);
            return;
        }
    };

    for event in scan_result {
        match event {
            DrmScanEvent::Connected {
                connector,
                crtc: Some(crtc),
            } => {
                connector_connected(state, node, connector, crtc);
            }
            DrmScanEvent::Disconnected {
                connector: _,
                crtc: Some(crtc),
            } => {
                connector_disconnected(state, node, crtc);
            }
            _ => {}
        }
    }
}

fn connector_connected(
    state: &mut Lantern,
    node: DrmNode,
    connector: connector::Info,
    crtc: crtc::Handle,
) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    let output_name = format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id()
    );
    info!("Connector connected: {} (modes: {})", output_name, connector.modes().len());

    let mode_id = connector
        .modes()
        .iter()
        .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .unwrap_or(0);
    let drm_mode = connector.modes()[mode_id];
    let wl_mode = WlMode::from(drm_mode);

    let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        output_name,
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: connector.subpixel().into(),
            make: "Unknown".into(),
            model: "Unknown".into(),
        },
    );

    let global = output.create_global::<Lantern>(&udev.display_handle);

    let x = state
        .space
        .outputs()
        .fold(0, |acc, o| {
            acc + state.space.output_geometry(o).unwrap().size.w
        });

    output.set_preferred(wl_mode);
    output.change_current_state(Some(wl_mode), None, None, Some((x, 0).into()));
    state.space.map_output(&output, (x, 0));

    output
        .user_data()
        .insert_if_missing(|| UdevOutputId {
            crtc,
            device_id: node,
        });

    let drm_output = match backend
        .drm_output_manager
        .initialize_output::<_, CustomRenderElements>(
            crtc,
            drm_mode,
            &[connector.handle()],
            &output,
            None,
            udev.renderer.as_mut().expect("Renderer not initialized"),
            &DrmOutputRenderElements::default(),
        ) {
        Ok(output) => output,
        Err(e) => {
            warn!("Failed to initialize DRM output: {:?}", e);
            return;
        }
    };

    backend.surfaces.insert(
        crtc,
        OutputSurface {
            device_id: node,
            global,
            drm_output,
            frame_pending: false,
        },
    );

    render_device(state, node, Some(crtc));
}

fn connector_disconnected(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    if let Some(surface) = backend.surfaces.remove(&crtc) {
        udev.display_handle
            .remove_global::<Lantern>(surface.global);
    }

    let output = state
        .space
        .outputs()
        .find(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .map(|id| id.device_id == node && id.crtc == crtc)
                .unwrap_or(false)
        })
        .cloned();

    if let Some(output) = output {
        state.space.unmap_output(&output);
    }
}

fn device_removed(state: &mut Lantern, node: DrmNode) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.remove(&node) {
        Some(b) => b,
        None => return,
    };

    for (_, surface) in backend.surfaces {
        udev.display_handle
            .remove_global::<Lantern>(surface.global);
    }

    let outputs: Vec<_> = state
        .space
        .outputs()
        .filter(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .map(|id| id.device_id == node)
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    for output in outputs {
        state.space.unmap_output(&output);
    }
}

fn frame_finish(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
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

    let submit_result = surface.drm_output.frame_submitted();

    match submit_result {
        Ok(_) => {}
        Err(e) => {
            warn!("Frame submit error: {:?}", e);
        }
    };

    render_surface(state, node, crtc);
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
    let udev = match state.udev.as_ref() {
        Some(u) => u,
        None => return,
    };

    // Collect (node, crtc) pairs that need rendering
    let targets: Vec<_> = udev
        .backends
        .iter()
        .flat_map(|(node, backend)| {
            backend.surfaces.iter().filter_map(move |(crtc, surface)| {
                if force || !surface.frame_pending {
                    Some((*node, *crtc))
                } else {
                    None
                }
            })
        })
        .collect();

    for (node, crtc) in targets {
        render_surface(state, node, crtc);
    }
}

fn render_device(state: &mut Lantern, node: DrmNode, crtc: Option<crtc::Handle>) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    let crtcs: Vec<_> = if let Some(crtc) = crtc {
        vec![crtc]
    } else {
        backend.surfaces.keys().copied().collect()
    };

    for crtc in crtcs {
        render_surface(state, node, crtc);
    }
}

fn render_surface(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
    let output = match state.space.outputs().find(|o| {
        o.user_data()
            .get::<UdevOutputId>()
            .map(|id| id.device_id == node && id.crtc == crtc)
            .unwrap_or(false)
    }) {
        Some(o) => o.clone(),
        None => return,
    };

    // Get cursor position relative to this output
    let pointer_location = state
        .seat
        .get_pointer()
        .map(|ptr| ptr.current_location())
        .unwrap_or_default();
    let output_pos = state
        .space
        .output_geometry(&output)
        .map(|geo| geo.loc)
        .unwrap_or_default();
    let cursor_pos: Point<f64, Physical> = (
        pointer_location.x - output_pos.x as f64,
        pointer_location.y - output_pos.y as f64,
    )
        .into();

    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let renderer = match udev.renderer.as_mut() {
        Some(r) => r,
        None => return,
    };

    let space_elements = match state.space.render_elements_for_output(renderer, &output, 1.0) {
        Ok(e) => e,
        Err(_) => return,
    };

    // Build combined elements: cursor first (highest z-order, eligible for HW cursor plane)
    let mut elements: Vec<CustomRenderElements> =
        Vec::with_capacity(space_elements.len() + 1);

    if let Some(cursor_elem) = state.cursor.render_element(renderer, cursor_pos) {
        elements.push(CustomRenderElements::Cursor(cursor_elem));
    }

    elements.extend(space_elements.into_iter().map(CustomRenderElements::Space));

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    let surface = match backend.surfaces.get_mut(&crtc) {
        Some(s) => s,
        None => return,
    };

    let result = surface.drm_output.render_frame(
        renderer,
        &elements,
        BG_COLOR,
        FrameFlags::DEFAULT,
    );

    let rendered = match result {
        Ok(result) => !result.is_empty,
        Err(err) => {
            warn!("Render error: {:?}", err);
            return;
        }
    };

    if rendered {
        state.space.elements().for_each(|window| {
            window.send_frame(
                &output,
                state.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        });

        surface.frame_pending = true;
        if let Err(e) = surface.drm_output.queue_frame(()) {
            surface.frame_pending = false;
            warn!("Failed to queue frame: {:?}", e);
        }
    }
}

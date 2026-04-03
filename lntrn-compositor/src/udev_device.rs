/// DRM device hot-plug handling: adding, changing, and removing DRM
/// devices and their connectors / CRTCs.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode,
        },
        egl::{context::ContextPriority, EGLContext, EGLDisplay},
        renderer::gles::{GlesRenderer, UniformName, UniformType},
        session::Session,
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Scale},
    reexports::{
        drm::control::{connector, crtc, ModeTypeFlags},
        rustix::fs::OFlags,
    },
    utils::DeviceFd,
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use tracing::{error, info, warn};

use crate::render::{render_surface, CustomRenderElements};
use crate::shaders::{
    CORNER_SHADER_SRC, HOT_CORNER_GLOW_SHADER_SRC, ROUNDED_TEX_SHADER_SRC, SHADOW_SHADER_SRC,
};
use crate::udev::{
    GpuBackend, OutputSurface, UdevOutputId, LANTERN_OUTPUT_SCALE, RENDER_INTERVAL,
    SUPPORTED_FORMATS,
};
use crate::Lantern;

pub fn device_added(
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
                crate::udev::frame_finish(state, node, crtc);
            }
            DrmEvent::Error(err) => {
                error!("DRM error: {:?}", err);
            }
        })?;

    let udev = state.udev.as_mut().ok_or("No udev data")?;
    // Store the renderer (replaces any previous one for multi-GPU, MVP uses one)
    udev.renderer = Some(renderer);

    compile_shaders(udev);

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

fn compile_shaders(udev: &mut crate::udev::UdevData) {
    let renderer = match udev.renderer.as_mut() {
        Some(r) => r,
        None => return,
    };

    match renderer.compile_custom_pixel_shader(
        CORNER_SHADER_SRC,
        &[
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("corner", UniformType::_2f),
        ],
    ) {
        Ok(shader) => {
            udev.corner_shader = Some(shader);
            info!("Corner rounding shader compiled");
        }
        Err(e) => warn!("Failed to compile corner shader: {:?}", e),
    }

    match renderer.compile_custom_pixel_shader(
        SHADOW_SHADER_SRC,
        &[
            UniformName::new("window_size", UniformType::_2f),
            UniformName::new("sigma", UniformType::_1f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("shadow_color", UniformType::_4f),
        ],
    ) {
        Ok(shader) => {
            udev.shadow_shader = Some(shader);
            info!("Shadow/glow shader compiled");
        }
        Err(e) => warn!("Failed to compile shadow shader: {:?}", e),
    }

    match renderer.compile_custom_pixel_shader(
        HOT_CORNER_GLOW_SHADER_SRC,
        &[
            UniformName::new("corner", UniformType::_2f),
            UniformName::new("glow_color", UniformType::_4f),
            UniformName::new("sigma", UniformType::_1f),
        ],
    ) {
        Ok(shader) => {
            udev.hot_corner_glow_shader = Some(shader);
            info!("Hot corner glow shader compiled");
        }
        Err(e) => warn!("Failed to compile hot corner glow shader: {:?}", e),
    }

    match renderer.compile_custom_pixel_shader(
        crate::shaders::SSD_ICON_SHADER_SRC,
        &[
            UniformName::new("icon_type", UniformType::_1f),
            UniformName::new("icon_color", UniformType::_4f),
        ],
    ) {
        Ok(shader) => {
            udev.ssd_icon_shader = Some(shader);
            info!("SSD icon shader compiled");
        }
        Err(e) => warn!("Failed to compile SSD icon shader: {:?}", e),
    }

    match renderer.compile_custom_pixel_shader(
        crate::shaders::SSD_HEADER_SHADER_SRC,
        &[
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("bar_color", UniformType::_4f),
        ],
    ) {
        Ok(shader) => {
            udev.ssd_header_shader = Some(shader);
            info!("SSD header shader compiled");
        }
        Err(e) => warn!("Failed to compile SSD header shader: {:?}", e),
    }

    match renderer.compile_custom_texture_shader(
        ROUNDED_TEX_SHADER_SRC,
        &[
            UniformName::new("tex_size", UniformType::_2f),
            UniformName::new("corner_radius", UniformType::_1f),
        ],
    ) {
        Ok(shader) => {
            udev.rounded_tex_shader = Some(shader);
            info!("Rounded-corner texture shader compiled");
        }
        Err(e) => warn!("Failed to compile rounded-corner texture shader: {:?}", e),
    }

    match renderer.compile_custom_texture_shader(
        crate::shaders::BLUR_DOWN_SHADER_SRC,
        &[UniformName::new("halfpixel", UniformType::_2f)],
    ) {
        Ok(shader) => {
            udev.blur_down_shader = Some(shader);
            info!("Blur downsample shader compiled");
        }
        Err(e) => warn!("Failed to compile blur down shader: {:?}", e),
    }

    match renderer.compile_custom_texture_shader(
        crate::shaders::BLUR_UP_SHADER_SRC,
        &[UniformName::new("halfpixel", UniformType::_2f)],
    ) {
        Ok(shader) => {
            udev.blur_up_shader = Some(shader);
            info!("Blur upsample shader compiled");
        }
        Err(e) => warn!("Failed to compile blur up shader: {:?}", e),
    }
}

pub fn device_changed(state: &mut Lantern, node: DrmNode) {
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
    output.change_current_state(
        Some(wl_mode),
        None,
        Some(Scale::Fractional(LANTERN_OUTPUT_SCALE)),
        Some((x, 0).into()),
    );
    state.space.map_output(&output, (x, 0));

    // Initialize canvas bounds from output size
    let mode_w = wl_mode.size.w as f64 / LANTERN_OUTPUT_SCALE;
    let mode_h = wl_mode.size.h as f64 / LANTERN_OUTPUT_SCALE;
    state.canvas.set_screen_size(mode_w, mode_h);

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
            pending_render: false,
            pending_interval: RENDER_INTERVAL,
            cooldown_until: Instant::now(),
        },
    );

    render_device(state, node, Some(crtc));
}

pub fn connector_disconnected(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
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

pub fn device_removed(state: &mut Lantern, node: DrmNode) {
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

pub fn render_device(state: &mut Lantern, node: DrmNode, crtc: Option<crtc::Handle>) {
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

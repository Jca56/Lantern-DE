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
        &[
            UniformName::new("halfpixel", UniformType::_2f),
            UniformName::new("tint_color", UniformType::_4f),
            UniformName::new("darken", UniformType::_1f),
        ],
    ) {
        Ok(shader) => {
            udev.blur_up_shader = Some(shader);
            info!("Blur upsample shader compiled");
        }
        Err(e) => warn!("Failed to compile blur up shader: {:?}", e),
    }

    match renderer.compile_custom_texture_shader(
        crate::shaders::ROUNDED_BACKDROP_SHADER_SRC,
        &[
            UniformName::new("tex_size", UniformType::_2f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("src_rect", UniformType::_4f),
        ],
    ) {
        Ok(shader) => {
            udev.backdrop_shader = Some(shader);
            info!("Rounded backdrop shader compiled");
        }
        Err(e) => warn!("Failed to compile backdrop shader: {:?}", e),
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

    // Find preferred resolution, then pick highest refresh rate at that resolution
    let preferred_idx = connector
        .modes()
        .iter()
        .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .unwrap_or(0);
    let preferred = connector.modes()[preferred_idx];
    let pref_size = preferred.size();

    let mode_id = connector
        .modes()
        .iter()
        .enumerate()
        .filter(|(_, m)| m.size() == pref_size)
        .max_by_key(|(_, m)| m.vrefresh())
        .map(|(i, _)| i)
        .unwrap_or(preferred_idx);
    let drm_mode = connector.modes()[mode_id];
    info!(
        "Selected mode: {}x{}@{}Hz for {}",
        drm_mode.size().0, drm_mode.size().1, drm_mode.vrefresh(), output_name
    );
    let wl_mode = WlMode::from(drm_mode);

    let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        output_name.clone(),
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: connector.subpixel().into(),
            make: "Unknown".into(),
            model: "Unknown".into(),
        },
    );

    let global = output.create_global::<Lantern>(&udev.display_handle);

    // Check monitor config for explicit position, otherwise auto-layout horizontally
    let monitor_configs = crate::read_monitor_configs();
    let (x, y) = if let Some(cfg) = monitor_configs.iter().find(|c| c.name == output_name) {
        info!("Using configured position for {}: ({}, {})", output_name, cfg.x, cfg.y);
        (cfg.x, cfg.y)
    } else {
        let auto_x = state
            .space
            .outputs()
            .fold(0, |acc, o| {
                acc + state.space.output_geometry(o).unwrap().size.w
            });
        (auto_x, 0)
    };

    output.set_preferred(wl_mode);
    output.change_current_state(
        Some(wl_mode),
        None,
        Some(Scale::Fractional(LANTERN_OUTPUT_SCALE)),
        Some((x, y).into()),
    );
    state.space.map_output(&output, (x, y));

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
    output
        .user_data()
        .insert_if_missing(|| crate::udev::UdevOutputModes {
            drm_modes: connector.modes().to_vec(),
            connector_handle: connector.handle(),
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

    // Announce to wlr-output-management clients
    state.output_management_state.add_head(
        &output_name,
        connector.modes(),
        mode_id,
        LANTERN_OUTPUT_SCALE,
        (x, y),
        (phys_w as i32, phys_h as i32),
    );
    state.output_management_state.broadcast_done();

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
        state.output_management_state.remove_head(&output.name());
        state.output_management_state.broadcast_done();
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

/// Apply output configuration changes from wlr-output-management.
pub fn apply_output_config(
    state: &mut Lantern,
    changes: Vec<crate::handlers::output_management::OutputChange>,
) -> bool {
    use smithay::output::{Mode as WlMode, Scale};

    for change in &changes {
        let output = match state.space.outputs().find(|o| o.name() == change.output_name).cloned() {
            Some(o) => o,
            None => return false,
        };

        let oid = match output.user_data().get::<UdevOutputId>() {
            Some(id) => *id,
            None => return false,
        };

        // Apply mode change
        if let Some(drm_idx) = change.drm_mode_index {
            let modes = match output.user_data().get::<crate::udev::UdevOutputModes>() {
                Some(m) => m,
                None => return false,
            };
            if drm_idx >= modes.drm_modes.len() { return false; }
            let drm_mode = modes.drm_modes[drm_idx];
            let wl_mode = WlMode::from(drm_mode);

            // Switch DRM mode via drm_output_manager
            let udev = match state.udev.as_mut() {
                Some(u) => u,
                None => return false,
            };
            let backend = match udev.backends.get_mut(&oid.device_id) {
                Some(b) => b,
                None => return false,
            };
            let renderer = match udev.renderer.as_mut() {
                Some(r) => r,
                None => return false,
            };
            if let Err(e) = backend.drm_output_manager.use_mode::<_, CustomRenderElements>(
                &oid.crtc,
                drm_mode,
                renderer,
                &DrmOutputRenderElements::default(),
            ) {
                tracing::warn!("Failed to switch DRM mode: {:?}", e);
                return false;
            }

            output.set_preferred(wl_mode);
            let cur_scale = change.scale.unwrap_or(LANTERN_OUTPUT_SCALE);
            output.change_current_state(
                Some(wl_mode),
                None,
                Some(Scale::Fractional(cur_scale)),
                None,
            );
        }

        // Apply scale change (if no mode change already applied it)
        if change.drm_mode_index.is_none() {
            if let Some(new_scale) = change.scale {
                output.change_current_state(
                    None,
                    None,
                    Some(Scale::Fractional(new_scale)),
                    None,
                );
            }
        }

        // Apply position change
        if let Some((x, y)) = change.position {
            output.change_current_state(
                None,
                None,
                None,
                Some((x, y).into()),
            );
            state.space.map_output(&output, (x, y));
        }
    }

    // Update output management state so clients see the new values
    for change in &changes {
        let mode_idx = change.drm_mode_index.and_then(|drm_idx| {
            state.output_management_state.heads.iter()
                .find(|h| h.output_name == change.output_name)
                .and_then(|h| h.modes.iter().position(|m| m.drm_mode_index == drm_idx))
        });
        state.output_management_state.update_head(
            &change.output_name,
            change.scale,
            change.position,
            mode_idx,
        );
    }

    // Reconfigure layer surfaces on affected outputs (bar, panels, etc.)
    reconfigure_layer_surfaces(state, &changes);

    // Invalidate wallpaper cache
    state.wallpaper.clear_cache();

    // Collect render targets, then schedule re-renders
    let render_targets: Vec<(DrmNode, crtc::Handle)> = changes
        .iter()
        .filter_map(|change| {
            let output = state.space.outputs().find(|o| o.name() == change.output_name)?;
            let oid = output.user_data().get::<UdevOutputId>()?;
            Some((oid.device_id, oid.crtc))
        })
        .collect();

    for (node, crtc) in render_targets {
        render_device(state, node, Some(crtc));
    }

    true
}

/// Reconfigure all layer surfaces on outputs affected by config changes.
/// This forces the bar (and other layer surfaces) to get updated dimensions
/// after a scale or mode change.
fn reconfigure_layer_surfaces(
    state: &mut Lantern,
    changes: &[crate::handlers::output_management::OutputChange],
) {
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::wlr_layer::{Anchor as A, ExclusiveZone, LayerSurfaceCachedState};

    let affected_outputs: Vec<String> = changes.iter().map(|c| c.output_name.clone()).collect();

    for ls in &state.layer_surfaces {
        let surface = ls.wl_surface();
        let output = match state.layer_surface_outputs.get(surface) {
            Some(o) if affected_outputs.contains(&o.name()) => o,
            _ => continue,
        };
        let geo = match state.space.output_geometry(output) {
            Some(g) => g,
            None => continue,
        };

        let cached = with_states(surface, |states| {
            *states.cached_state.get::<LayerSurfaceCachedState>().current()
        });

        let mut width = cached.size.w;
        let mut height = cached.size.h;

        // Compute exclusive zone reductions from other layer surfaces
        let mut excl_top = 0i32;
        let mut excl_bottom = 0i32;
        let mut excl_left = 0i32;
        let mut excl_right = 0i32;
        let is_neutral = matches!(cached.exclusive_zone, ExclusiveZone::Neutral);
        if is_neutral {
            for other in &state.layer_surfaces {
                if other.wl_surface() == surface { continue; }
                let oc = with_states(other.wl_surface(), |s| {
                    *s.cached_state.get::<LayerSurfaceCachedState>().current()
                });
                let ex = match oc.exclusive_zone {
                    ExclusiveZone::Exclusive(v) => v as i32,
                    _ => continue,
                };
                if oc.anchor.contains(A::BOTTOM) && !oc.anchor.contains(A::TOP) {
                    excl_bottom += ex;
                } else if oc.anchor.contains(A::TOP) && !oc.anchor.contains(A::BOTTOM) {
                    excl_top += ex;
                } else if oc.anchor.contains(A::LEFT) && !oc.anchor.contains(A::RIGHT) {
                    excl_left += ex;
                } else if oc.anchor.contains(A::RIGHT) && !oc.anchor.contains(A::LEFT) {
                    excl_right += ex;
                }
            }
        }

        if cached.anchor.anchored_horizontally() && width == 0 {
            width = geo.size.w - cached.margin.left - cached.margin.right - excl_left - excl_right;
        }
        if cached.anchor.anchored_vertically() && height == 0 {
            height = geo.size.h - cached.margin.top - cached.margin.bottom - excl_top - excl_bottom;
        }

        ls.with_pending_state(|s| {
            s.size = Some(smithay::utils::Size::from((width, height)));
        });
        ls.send_pending_configure();
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

/// Re-read monitor positions from config and reposition outputs if changed.
pub fn reload_monitor_positions(state: &mut Lantern) {
    let configs = crate::read_monitor_configs();
    if configs.is_empty() {
        return;
    }

    let mut changed = false;
    let outputs: Vec<_> = state.space.outputs().cloned().collect();

    for output in &outputs {
        let name = output.name();
        if let Some(cfg) = configs.iter().find(|c| c.name == name) {
            let current_geo = state.space.output_geometry(output).unwrap_or_default();
            if current_geo.loc.x != cfg.x || current_geo.loc.y != cfg.y {
                output.change_current_state(
                    None,
                    None,
                    None,
                    Some((cfg.x, cfg.y).into()),
                );
                state.space.map_output(output, (cfg.x, cfg.y));
                info!("Live-reloaded position for {}: ({}, {})", name, cfg.x, cfg.y);
                changed = true;
            }
        }
    }

    if changed {
        // Reconfigure maximized/snapped windows for new output positions
        state.check_exclusive_zone_change();
    }
}

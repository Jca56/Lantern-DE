/// DRM device hot-plug handling: adding, changing, and removing DRM
/// devices and their connectors / CRTCs.
use std::collections::HashMap;
use std::path::Path;

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
    TOP_CENTER_GLOW_SHADER_SRC,
};
use crate::udev::{GpuBackend, OutputSurface, UdevOutputId, SUPPORTED_FORMATS};
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
            .open(
                path,
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY,
            )
            .map_err(|e| format!("Failed to open device {:?}: {}", path, e))?
    };
    info!("Opened DRM device fd");

    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (drm, drm_notifier) =
        DrmDevice::new(fd.clone(), true).map_err(|e| format!("DrmDevice::new failed: {}", e))?;
    info!("DRM device created");

    let gbm = GbmDevice::new(fd.clone()).map_err(|e| format!("GbmDevice::new failed: {}", e))?;
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

    let drm_registration =
        state
            .loop_handle
            .insert_source(drm_notifier, move |event, metadata, state| {
                if state.debug_counters.enabled {
                    state.debug_counters.drm_fires += 1;
                }
                match event {
                    DrmEvent::VBlank(crtc) => {
                        // Hand the real scanout timestamp + vblank sequence to the
                        // pacing path — pre-refactor this metadata was discarded.
                        crate::udev::frame_finish(state, node, crtc, metadata.take());
                    }
                    DrmEvent::Error(err) => {
                        error!("DRM error: {:?}", err);
                    }
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
            connect_retries: HashMap::new(),
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
        crate::shaders::BORDER_SHADER_SRC,
        &[
            UniformName::new("window_size", UniformType::_2f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("border_width", UniformType::_1f),
            UniformName::new("border_color", UniformType::_4f),
        ],
    ) {
        Ok(shader) => {
            udev.border_shader = Some(shader);
            info!("Border shader compiled");
        }
        Err(e) => warn!("Failed to compile border shader: {:?}", e),
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
        TOP_CENTER_GLOW_SHADER_SRC,
        &[
            UniformName::new("glow_color", UniformType::_4f),
            UniformName::new("sigma", UniformType::_1f),
            UniformName::new("line_half_len", UniformType::_1f),
        ],
    ) {
        Ok(shader) => {
            udev.top_center_glow_shader = Some(shader);
            info!("Top-center glow shader compiled");
        }
        Err(e) => warn!("Failed to compile top-center glow shader: {:?}", e),
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
            UniformName::new("top_extend", UniformType::_1f),
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
    // Whatever was still waiting for its monitor gets another try once this
    // scan is handled: a hotplug event is the best hint that it is ready.
    let waiting = crate::output_recovery::pending(state, node);

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
            DrmScanEvent::Connected {
                connector,
                crtc: None,
            } => {
                warn!(
                    "Connector {}-{} connected, but no CRTC is free to drive it",
                    connector.interface().as_str(),
                    connector.interface_id()
                );
            }
            DrmScanEvent::Disconnected { connector, crtc } => {
                info!(
                    "Connector disconnected: {}-{}",
                    connector.interface().as_str(),
                    connector.interface_id()
                );
                crate::output_recovery::cancel_connect_retry(state, node, connector.handle());
                if let Some(crtc) = crtc {
                    connector_disconnected(state, node, crtc);
                }
            }
        }
    }

    crate::output_recovery::nudge(state, node, waiting);
}

pub(crate) fn connector_connected(
    state: &mut Lantern,
    node: DrmNode,
    connector: connector::Info,
    crtc: crtc::Handle,
) {
    // The last output is kept while its monitor is away. If this is that
    // monitor coming back, relighting it is all there is to do.
    if crate::output_recovery::reconnect_parked(state, node, &connector, crtc) {
        return;
    }

    let output_name = format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id()
    );
    // Where it is first mapped; `relayout` below settles the whole layout
    // once it is registered.
    let (x, y) = state.layout_admit(&output_name);

    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    if backend.surfaces.contains_key(&crtc) {
        warn!(
            "Connector {} connected on a CRTC that already drives an output; ignoring",
            output_name
        );
        return;
    }
    info!(
        "Connector connected: {} (modes: {})",
        output_name,
        connector.modes().len()
    );

    // A monitor that is still waking up can report itself connected before
    // its mode list is readable.
    if connector.modes().is_empty() {
        crate::output_recovery::schedule_connect_retry(
            state,
            node,
            connector.handle(),
            &output_name,
            "it reports no modes yet",
        );
        return;
    }

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
    let wl_mode = WlMode::from(drm_mode);
    let frame_interval_us = if wl_mode.refresh > 0 {
        1_000_000_000u64 / (wl_mode.refresh as u64)
    } else {
        16_666
    };
    info!(
        "Selected mode: {}x{}@{}Hz for {} (frame interval: {:.2}ms)",
        drm_mode.size().0,
        drm_mode.size().1,
        drm_mode.vrefresh(),
        output_name,
        frame_interval_us as f64 / 1000.0
    );

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

    output.set_preferred(wl_mode);
    output.change_current_state(
        Some(wl_mode),
        None,
        Some(Scale::Fractional(crate::monitor_scale(&output_name))),
        Some((x, y).into()),
    );

    output.user_data().insert_if_missing(|| UdevOutputId {
        crtc,
        device_id: node,
    });
    output
        .user_data()
        .insert_if_missing(|| crate::udev::UdevOutputModes {
            drm_modes: connector.modes().to_vec(),
            connector_handle: connector.handle(),
        });

    // Detect HDR capability from the display's EDID. Stored on the output so
    // the settings app (via lntrn_hdr_manager) and the DRM-prop path can read
    // it. None → pure-SDR panel, the HDR toggle never appears for this output.
    let hdr_caps = smithay_drm_extras::display_info::for_connector(
        backend.drm_output_manager.device(),
        connector.handle(),
    )
    .and_then(|info| crate::hdr::edid_caps::detect(&info));
    if let Some(caps) = &hdr_caps {
        info!(
            "Output {} HDR-capable: pq={} hlg={} max_lum={:.0}nits min_lum={:.4}nits",
            output_name,
            caps.pq_supported,
            caps.hlg_supported,
            caps.max_luminance,
            caps.min_luminance
        );
        output.user_data().insert_if_missing(|| caps.clone());
    } else {
        info!("Output {} reports no HDR support (SDR-only)", output_name);
    }

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
            // Nothing is registered yet, so a failure leaves no half-built
            // output behind. The driver can refuse the first modeset of a
            // monitor that is still waking up: keep trying.
            let why = format!("{e:?}");
            crate::output_recovery::schedule_connect_retry(
                state,
                node,
                connector.handle(),
                &output_name,
                &why,
            );
            return;
        }
    };

    // Only an output that can show a picture becomes part of the desktop.
    let global = output.create_global::<Lantern>(&udev.display_handle);
    state.space.map_output(&output, (x, y));
    state
        .workspaces
        .register_output(output.clone(), (x, y).into());

    backend.surfaces.insert(
        crtc,
        OutputSurface {
            device_id: node,
            global,
            drm_output,
            frame_pending: false,
            frame_pending_since: None,
            pending_render: false,
            scanout_active: false,
            cursor_plane_active: false,
            pending_feedback: None,
            watchdog_timer: None,
            noflip_timer: None,
            last_callbacks_at: None,
            primary_commit: None,
            blanked: false,
            parked: false,
            relight: None,
        },
    );

    // Announce to wlr-output-management clients
    state.output_management_state.add_head(
        &output_name,
        connector.modes(),
        mode_id,
        crate::monitor_scale(&output_name),
        (x, y),
        (phys_w as i32, phys_h as i32),
    );
    state.output_management_state.broadcast_done();

    // Publish HDR capability to the settings app (no-op caps line if SDR-only).
    state.announce_hdr_caps(&output);

    // NOTE: HDR is deliberately NOT engaged at startup, even when the output is
    // configured `hdr = true`. Engaging HDR can break the display link, and a
    // boot-time failure is the worst case (no chance to intervene → lockout).
    // HDR only ever engages from a live, watched toggle with an auto-revert
    // timer. See `hdr::safety` and `hdr::set_output_hdr`.

    render_device(state, node, Some(crtc));

    // If Gaming Mode is active and this is the primary coming back, it came
    // up at its desktop scale. No-op when gaming mode is off.
    state.reapply_gaming_layout();

    // Honor any monitor the user persisted as `enabled = false` in
    // lantern.toml. Runs on every connect so connect-ordering doesn't matter;
    // skips the output currently being deliberately re-enabled.
    crate::output_toggle::reconcile_disabled_outputs(state);

    crate::output_recovery::cancel_connect_retry(state, node, connector.handle());
    // A parked output was only kept because it was the last one.
    crate::output_recovery::retire_parked(state, &output_name);

    // Whatever was moved off this output while it was away comes back, and
    // the layout is resolved with it in: its configured position was right
    // for the sizes the others had when it was written, which a scale
    // change (or Gaming Mode) since may have made an overlap.
    state.welcome_home(&output_name);
    state.relayout();
}

pub fn connector_disconnected(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
    // The last output is kept rather than torn down, so the desktop never
    // loses its only screen. See `output_recovery`.
    if crate::output_recovery::park(state, node, crtc) {
        return;
    }
    teardown_output(state, node, crtc);
}

/// Remove an output for good: its DRM surface, its wl_output and its place
/// in the layout.
pub(crate) fn teardown_output(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
    let udev = match state.udev.as_mut() {
        Some(u) => u,
        None => return,
    };

    let backend = match udev.backends.get_mut(&node) {
        Some(b) => b,
        None => return,
    };

    if let Some(surface) = backend.surfaces.remove(&crtc) {
        udev.display_handle.remove_global::<Lantern>(surface.global);
        // Its one-shot timers would otherwise fire into a missing surface.
        let relight_timer = surface
            .relight
            .and_then(crate::output_recovery::Relight::into_timer);
        for tok in [surface.watchdog_timer, surface.noflip_timer, relight_timer]
            .into_iter()
            .flatten()
        {
            state.loop_handle.remove(tok);
        }
    }
    // GPU-side state this output owned — blur textures and the screencopy
    // readback buffer used to leak across every unplug / replug.
    udev.blur_states.remove(&UdevOutputId {
        device_id: node,
        crtc,
    });

    let drives = |o: &smithay::output::Output| {
        o.user_data()
            .get::<UdevOutputId>()
            .is_some_and(|id| id.device_id == node && id.crtc == crtc)
    };
    // A monitor switched off in System Settings is in no Space, only in
    // `disabled_outputs`. Unplugged and left there, its dead output would be
    // mapped back in by the next "enable", and the fresh one a replug builds
    // could never be switched off (it already counted as off).
    let output = state
        .space
        .outputs()
        .find(|o| drives(o))
        .cloned()
        .or_else(|| {
            state
                .disabled_outputs
                .values()
                .map(|d| &d.output)
                .find(|o| drives(o))
                .cloned()
        });

    if let Some(output) = output {
        let name = output.name();
        if let Some(slot) = state.screencopy_pbos.remove(&name) {
            if let Some(renderer) = udev.renderer.as_mut() {
                slot.release(renderer);
            }
        }
        state.screencopy_offscreen.remove(&name);
        state.window_chrome_cache.remove(&name);
        state.output_management_state.remove_head(&name);
        state.output_management_state.broadcast_done();
        state.hdr_ipc.remove_output(&name);
        state.disabled_outputs.remove(&name);
        // Its windows go to a monitor that is still there, and come back
        // when this one does. Left on it they sat at coordinates no screen
        // showed, reachable only by the monitor returning.
        if let Some(refuge) = crate::output_toggle::pick_target_output(state, &name) {
            state.evacuate_output(&name, &refuge);
        }
        state.space.unmap_output(&output);
        state.workspaces.unregister_output(&output);
        state.reroute_layer_surfaces_from(&output);
        // Its lock surface goes with it; typing has to land on one that's left.
        if let Some(lock) = state.session_lock.as_mut() {
            lock.surfaces.remove(&name);
            lock.presented.remove(&name);
        }
        state.ensure_lock_focus();
        // The outputs that remain close up, back to the origin.
        state.relayout();
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
        udev.display_handle.remove_global::<Lantern>(surface.global);
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
        state.workspaces.unregister_output(&output);
    }
}

/// Apply output configuration changes from wlr-output-management (and
/// Gaming Mode). Modes and scales are set here; positions are recorded as
/// intent and the layout is then resolved as a whole, so a batch that makes
/// one output wider can't leave it overlapping the next. Returns false if
/// any part of the batch could not be applied (the rest still is).
pub fn apply_output_config(
    state: &mut Lantern,
    changes: Vec<crate::handlers::output_management::OutputChange>,
) -> bool {
    let mut ok = true;
    let mut resized = false;
    // Before anything changes size under it.
    let pointer = state.pointer_anchor();

    for change in &changes {
        let name = &change.output_name;
        match change.enabled {
            Some(false) => {
                ok &= crate::output_toggle::disable_output(state, name);
                continue;
            }
            // Back on, then configured like any other head: a batch may
            // switch a monitor on *and* say where it goes.
            Some(true) if state.disabled_outputs.contains_key(name) => {
                crate::output_toggle::enable_output(state, name);
            }
            _ => {}
        }

        let Some(output) = state.workspaces.output_by_name(name).cloned() else {
            ok = false;
            continue;
        };

        if let Some(drm_idx) = change.drm_mode_index {
            if !switch_mode(state, &output, drm_idx) {
                ok = false;
                continue;
            }
            resized = true;
        }

        if let Some(scale) = change.scale {
            output.change_current_state(None, None, Some(Scale::Fractional(scale)), None);
            resized = true;
        }

        if change.drm_mode_index.is_some() || change.scale.is_some() {
            if let Some(geo) = state.workspaces.output_geometry(&output) {
                info!(
                    output = %name,
                    scale = output.current_scale().fractional_scale(),
                    logical = %format!("{}x{}", geo.size.w, geo.size.h),
                    "Output mode/scale applied"
                );
            }
        }

        if let Some(pos) = change.position {
            state.layout_intend(name, pos);
        }
    }

    if resized {
        // Wallpapers are decoded at the output's size.
        state.wallpaper.clear_cache();
    }
    // The primary output's scale may have changed — re-apply the XWayland
    // scale override so newly-spawned X11 games still see native physical
    // resolution under the new UI scale.
    crate::xwayland::refresh_xwayland_scale(state);

    state.output_management_state.sync_applied_modes(&changes);
    // Moves the outputs (windows in tow), re-fits maximized / fullscreen
    // windows and layer surfaces, pushes the new scales to clients and the
    // new head state to wlr-output-management clients, and repaints.
    state.relayout_from(pointer, true);
    ok
}

/// Switch `output` to the DRM mode at `drm_idx` of its mode list, keeping
/// its scale.
fn switch_mode(state: &mut Lantern, output: &Output, drm_idx: usize) -> bool {
    let Some(oid) = output.user_data().get::<UdevOutputId>().copied() else {
        return false;
    };
    let Some(drm_mode) = output
        .user_data()
        .get::<crate::udev::UdevOutputModes>()
        .and_then(|m| m.drm_modes.get(drm_idx).copied())
    else {
        return false;
    };
    let Some(udev) = state.udev.as_mut() else {
        return false;
    };
    let Some(backend) = udev.backends.get_mut(&oid.device_id) else {
        return false;
    };
    let Some(renderer) = udev.renderer.as_mut() else {
        return false;
    };
    if let Err(e) = backend
        .drm_output_manager
        .use_mode::<_, CustomRenderElements>(
            &oid.crtc,
            drm_mode,
            renderer,
            &DrmOutputRenderElements::default(),
        )
    {
        warn!("Failed to switch DRM mode: {:?}", e);
        return false;
    }
    let wl_mode = WlMode::from(drm_mode);
    output.set_preferred(wl_mode);
    output.change_current_state(Some(wl_mode), None, None, None);
    true
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

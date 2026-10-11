//! Manual monitor on/off (barrier mode).
//!
//! "Off" here does NOT power the panel down — on this NVIDIA + Smithay 0.7 stack
//! an atomic modeset-disable is unreliable (it leaves the connector scanning out
//! a frozen frame instead of going dark). What the user actually wants is for
//! the monitor to stop being part of the desktop: the pointer and windows can't
//! cross onto it, and screenshots/region-select ignore it. So "off" means we
//! exclude the output from the usable layout (unmap from every workspace + the
//! global space) while keeping its DRM scanout and wl_output alive. Everything
//! that bounds interaction — pointer clamping, window placement, total output
//! bounds — keys off `workspaces.outputs_iter()`, so a single unregister is a
//! clean, comprehensive wall. The monitor keeps showing its last frame; flipping
//! it back on just re-maps the still-alive output (no rebuild, no modeset).
//!
//! What the panel is left showing is one empty frame ([`blank_disabled`]),
//! not the desktop as it was when the monitor was switched off: that frozen
//! picture — windows, cursor and all — would otherwise still be on it when
//! the session is locked.

use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::DrmNode;
use smithay::output::Output;
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Logical, Point};
use tracing::{info, warn};

use crate::render::CustomRenderElements;
use crate::udev::BG_COLOR;

use crate::Lantern;

/// What we stash about a monitor in "barrier" mode: the live [`Output`] (kept
/// alive while it's excluded from the layout) and where to map it back on
/// re-enable. The DRM scanout + wl_output stay up the whole time — we only
/// remove it from the usable workspace area, never tear the connector down.
#[derive(Clone)]
pub struct DisabledOutput {
    pub output: Output,
    pub loc: Point<i32, Logical>,
}

/// Switch a monitor to barrier mode: migrate its windows away and exclude it
/// from the usable layout (unmap from the global space + every workspace) so the
/// pointer, windows, and screenshots can't reach it. Its DRM scanout + wl_output
/// stay alive (no flaky modeset teardown) — the panel keeps showing its last
/// frame but is no longer enterable. Keeps the wlr head (advertised disabled) so
/// System Settings shows the off state. Returns false if it can't / shouldn't
/// (unknown output, last screen, nowhere to migrate to).
pub fn disable_output(state: &mut Lantern, name: &str) -> bool {
    if state.disabled_outputs.contains_key(name) {
        return true; // already off
    }

    let output = match state
        .workspaces
        .outputs_iter()
        .find(|o| o.name() == name)
        .cloned()
    {
        Some(o) => o,
        None => {
            warn!("disable_output: no live output named {name}");
            return false;
        }
    };

    // Never disable the last screen — that would leave the session blind.
    if state.workspaces.outputs_iter().count() <= 1 {
        warn!("disable_output: refusing to disable the only output ({name})");
        return false;
    }

    // Where windows (and the cursor) should land.
    let Some(target) = pick_target_output(state, name) else {
        warn!("disable_output: no other output to migrate {name} onto");
        return false;
    };

    // Remember where to map the output back on re-enable.
    let loc = state
        .workspaces
        .output_geometry(&output)
        .map(|g| g.loc)
        .unwrap_or_default();

    info!("Disabling output {name} (barrier mode); migrating windows -> {target}");
    state.evacuate_output(name, &target);

    // Exclude from the usable layout. The pointer is kept on the outputs that
    // are registered, so this walls it off; it also drops the output from
    // every workspace and stops new windows landing on it — the whole "can't
    // cross over" behaviour. The DRM scanout + wl_output are deliberately
    // left up.
    state.space.unmap_output(&output);
    state.workspaces.unregister_output(&output);
    state.hdr_ipc.remove_output(name);
    // Panels / notifications assigned to this output would otherwise stop
    // rendering entirely (its render pass bails once it's unregistered).
    state.reroute_layer_surfaces_from(&output);

    state
        .disabled_outputs
        .insert(name.to_string(), DisabledOutput { output, loc });

    // Advertise the head disabled so Settings shows the off state and can flip
    // it back.
    state.output_management_state.set_head_enabled(name, false);
    state.output_management_state.broadcast_done();

    // The outputs that remain close up around the gap (and back to the
    // origin, if this was the leftmost), windows in tow; whatever was
    // maximized on the monitor that left is fitted to the one it moved to,
    // and a pointer left outside the layout is brought back in.
    state.relayout();
    true
}

/// The render pass of a CRTC whose output is not in the layout: if that is
/// because it was switched off, give it an empty frame, once. No elements
/// also means nothing on its cursor plane.
pub(crate) fn blank_disabled(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
    let disabled = state.disabled_outputs.values().any(|d| {
        d.output
            .user_data()
            .get::<crate::udev::UdevOutputId>()
            .is_some_and(|id| id.device_id == node && id.crtc == crtc)
    });
    if !disabled {
        return;
    }
    let Some(udev) = state.udev.as_mut() else {
        return;
    };
    let Some(renderer) = udev.renderer.as_mut() else {
        return;
    };
    let Some(surface) = udev
        .backends
        .get_mut(&node)
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    if surface.blanked {
        return;
    }
    match surface.drm_output.render_frame(
        renderer,
        &[] as &[CustomRenderElements],
        BG_COLOR,
        FrameFlags::empty(),
    ) {
        Ok(frame) => {
            surface.blanked = true;
            if !frame.is_empty {
                match surface.drm_output.queue_frame(()) {
                    Ok(()) => {
                        surface.frame_pending = true;
                        surface.frame_pending_since = Some(std::time::Instant::now());
                    }
                    Err(e) => warn!("blanking a switched-off output: queue_frame failed: {e:?}"),
                }
            }
        }
        Err(e) => warn!("blanking a switched-off output: render failed: {e:?}"),
    }
}

/// Mark a re-enabled output's surface as showing the desktop again.
fn unblank(state: &mut Lantern, output: &Output) {
    let Some(id) = output.user_data().get::<crate::udev::UdevOutputId>().copied() else {
        return;
    };
    if let Some(surface) = state
        .udev
        .as_mut()
        .and_then(|u| u.backends.get_mut(&id.device_id))
        .and_then(|b| b.surfaces.get_mut(&id.crtc))
    {
        surface.blanked = false;
    }
}

/// Switch a barrier-mode monitor back on by re-mapping its still-alive output
/// into the layout at its old position. No connector rebuild or modeset — the
/// DRM scanout was never torn down, so this just makes the output usable again.
pub fn enable_output(state: &mut Lantern, name: &str) -> bool {
    let Some(d) = state.disabled_outputs.remove(name) else {
        return true; // not disabled — nothing to do
    };

    info!("Re-enabling output {name} (barrier mode) at {:?}", d.loc);

    // Re-map the output back into the global space and every workspace. Both
    // map_output/register_output are idempotent, so re-adding the same Output is
    // safe; rendering to it resumes automatically once it's back in the layout.
    state.space.map_output(&d.output, d.loc);
    state.workspaces.register_output(d.output.clone(), d.loc);
    unblank(state, &d.output);

    // Advertise the head enabled again so Settings reflects the on state.
    state.output_management_state.set_head_enabled(name, true);
    state.output_management_state.broadcast_done();

    // Its windows come back, and the layout is resolved with it in: the
    // position it was switched off at may overlap a neighbour whose scale
    // changed since.
    state.welcome_home(name);
    state.relayout();
    true
}

/// Honor monitors persisted as `enabled = false` in lantern.toml. Called at the
/// tail of `connector_connected`, so a monitor configured off is torn down as
/// soon as it (and at least one other output) is up. Skips the output currently
/// being deliberately re-enabled.
pub fn reconcile_disabled_outputs(state: &mut Lantern) {
    let off: Vec<String> = crate::read_monitor_configs()
        .into_iter()
        .filter(|c| !c.enabled)
        .map(|c| c.name)
        .collect();
    for name in off {
        if state.enabling_output.as_deref() == Some(name.as_str()) {
            continue;
        }
        if state.disabled_outputs.contains_key(&name) {
            continue;
        }
        if state.workspaces.outputs_iter().any(|o| o.name() == name) {
            disable_output(state, &name);
        }
    }
}

/// The primary output if it's a different live output, else any other live one.
pub(crate) fn pick_target_output(state: &Lantern, exclude: &str) -> Option<String> {
    if let Some(p) = crate::primary_output_name() {
        if p != exclude && state.workspaces.outputs_iter().any(|o| o.name() == p) {
            return Some(p);
        }
    }
    state
        .workspaces
        .outputs_iter()
        .map(|o| o.name())
        .find(|n| n != exclude)
}

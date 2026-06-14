//! Variable Refresh Rate (VRR / adaptive sync / FreeSync / G-Sync) support.
//!
//! VRR is enabled **on demand**: only while a fullscreen window owns an output
//! that the user flagged `vrr = true` in `[[monitors]]`. This is deliberate —
//! the desktop renders on demand (often a couple frames per second when idle),
//! which is far below a VRR panel's minimum refresh (e.g. 48Hz). Leaving VRR on
//! for a near-static desktop would push the panel below its floor and cause
//! flicker/blanking. Games, on the other hand, produce a steady high frame rate
//! that VRR can track exactly — the panel refreshes the instant each frame is
//! ready, eliminating the judder you get from a fixed refresh that doesn't
//! evenly divide the game's framerate (e.g. 100fps on a fixed 144Hz panel).
//!
//! Everything degrades gracefully: if the DRM driver doesn't expose the
//! `VRR_ENABLED` / `vrr_capable` KMS properties (notably some NVIDIA setups),
//! `vrr_supported` returns `NotSupported` and we simply log and no-op.

use smithay::backend::drm::VrrSupport;
use smithay::output::Output;
use smithay::utils::Point;

use crate::state::Lantern;
use crate::udev::{UdevOutputId, UdevOutputModes};

impl Lantern {
    /// Reconcile VRR state across all outputs. Enables adaptive sync on an
    /// output when (a) the user flagged it `vrr = true` and (b) a fullscreen
    /// window currently owns it; disables it otherwise. Cheap and idempotent —
    /// safe to call after any fullscreen transition.
    pub fn refresh_vrr(&mut self) {
        // udev (real DRM) is the only backend with VRR; winit/dev = no-op.
        if self.udev.is_none() {
            return;
        }
        let outputs: Vec<Output> = self.workspaces.outputs_iter().cloned().collect();
        for output in outputs {
            let allow = crate::output_vrr_enabled(&output.name());
            let desired = allow && self.output_has_fullscreen(&output);
            self.set_output_vrr(&output, desired);
        }
    }

    /// True if any fullscreen window is currently assigned to `output`. Uses the
    /// output geometry captured at fullscreen time (`FullscreenWindow::target`),
    /// which is stable even while the enter animation is still in flight. Also
    /// used by the render path to gate direct scanout.
    pub(crate) fn output_has_fullscreen(&self, output: &Output) -> bool {
        self.fullscreen_windows.iter().any(|fw| {
            // Probe just inside the target's top-left corner, NOT its center. A
            // window sized larger than a monitor has a center that can fall on a
            // neighbouring output (known footgun) — but `fw.target` is the output
            // geometry captured at fullscreen time, so its origin reliably lands
            // on the output it covers. The +1,+1 nudge disambiguates a window
            // whose origin sits exactly on a boundary between two outputs.
            // Getting this right matters for pacing: a wrong "false" here runs
            // the 4K dual-kawase blur and disables direct scanout during
            // gameplay, blowing the vblank budget and stuttering the game.
            let probe = Point::<f64, smithay::utils::Logical>::from((
                fw.target.loc.x as f64 + 1.0,
                fw.target.loc.y as f64 + 1.0,
            ));
            self.output_at_point(probe).as_ref() == Some(output)
        })
    }

    /// Toggle VRR on a single output's DRM surface. Only commits a change when
    /// the requested state differs from the current one and the driver/display
    /// actually support VRR. Schedules a render so the pending state is flushed
    /// to KMS on the next page-flip.
    fn set_output_vrr(&mut self, output: &Output, enable: bool) {
        let Some(oid) = output.user_data().get::<UdevOutputId>().copied() else {
            return;
        };
        let Some(conn) = output
            .user_data()
            .get::<UdevOutputModes>()
            .map(|m| m.connector_handle)
        else {
            return;
        };
        let name = output.name();
        let mut toggled = false;
        {
            let Some(udev) = self.udev.as_mut() else { return };
            let Some(backend) = udev.backends.get_mut(&oid.device_id) else { return };
            let Some(surface) = backend.surfaces.get_mut(&oid.crtc) else { return };
            surface.drm_output.with_compositor(|comp| {
                if comp.vrr_enabled() == enable {
                    return;
                }
                match comp.vrr_supported(conn) {
                    Ok(VrrSupport::NotSupported) | Err(_) => {
                        if enable {
                            tracing::info!(
                                output = %name,
                                "VRR requested but not supported by driver/display — \
                                 KMS VRR_ENABLED/vrr_capable property missing"
                            );
                        }
                    }
                    Ok(_) => match comp.use_vrr(enable) {
                        Ok(()) => {
                            toggled = true;
                            tracing::info!(output = %name, enabled = enable, "VRR toggled");
                        }
                        Err(e) => {
                            tracing::warn!(output = %name, "VRR toggle failed: {e:?}");
                        }
                    },
                }
            });
        }
        if toggled {
            self.schedule_render();
        }
    }
}

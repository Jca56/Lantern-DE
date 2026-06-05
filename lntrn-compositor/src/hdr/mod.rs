//! HDR support: EDID capability detection, DRM connector-property signalling,
//! the color-aware render pipeline, and the `wp_color_management_v1` server.
//!
//! HDR is opt-in per output (`hdr = true` in `[[monitors]]`) and only ever
//! engages on displays that report HDR support via EDID. Everything degrades
//! gracefully: an SDR-only display, or a GPU/driver that doesn't expose the
//! HDR connector properties, simply renders as it always has.

pub mod drm_props;
pub mod edid_caps;
pub mod safety;

pub use edid_caps::HdrCaps;

use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use tracing::{info, warn};

use crate::hdr_ipc::OutputCaps;
use crate::state::Lantern;
use crate::udev::{UdevOutputId, UdevOutputModes};

/// How long the user has to confirm "keep HDR" before it auto-reverts. Long
/// enough to read the prompt, short enough that a dark/locked screen recovers
/// quickly.
pub const HDR_CONFIRM_SECS: u64 = 15;

impl Lantern {
    /// Publish an output's HDR capability to the settings app over the HDR IPC
    /// socket. Called when an output is connected (after EDID detection).
    pub fn announce_hdr_caps(&mut self, output: &Output) {
        let name = output.name();
        let caps = output.user_data().get::<HdrCaps>().map(|c| OutputCaps {
            output: name.clone(),
            hdr_capable: c.is_hdr_capable(),
            max_nits: c.max_luminance as u32,
            min_milli_nits: (c.min_luminance * 1000.0) as u32,
        });
        let caps = caps.unwrap_or(OutputCaps {
            output: name,
            hdr_capable: false,
            max_nits: 0,
            min_milli_nits: 0,
        });
        self.hdr_ipc.update_caps(caps);
    }

    /// Poll the HDR IPC socket and apply any live enable/disable/confirm
    /// requests.
    pub fn poll_hdr_ipc(&mut self) {
        let commands = self.hdr_ipc.poll();
        for cmd in commands {
            // A bare confirm keeps whatever HDR state is pending for the output.
            if cmd.confirm {
                self.confirm_hdr(&cmd.output);
                continue;
            }

            let Some(output) = self
                .workspaces
                .outputs_iter()
                .find(|o| o.name() == cmd.output)
                .cloned()
            else {
                info!("HDR set request for unknown output {}", cmd.output);
                continue;
            };
            info!(
                "HDR set request: output={} enable={} sdr_nits={}",
                cmd.output, cmd.enable, cmd.sdr_nits
            );
            self.set_output_hdr(&output, cmd.enable, cmd.sdr_nits);
        }
    }

    /// Confirm that HDR is working on an output ("Keep" clicked). Cancels the
    /// pending auto-revert and clears the crash marker so it sticks.
    pub fn confirm_hdr(&mut self, output: &str) {
        if self.hdr_pending_confirm.remove(output).is_some() {
            safety::clear_marker(output);
            info!(output = %output, "HDR confirmed kept");
            // Tell the settings app the countdown is over.
            self.hdr_ipc.notify_confirmed(output);
        }
    }

    /// Auto-revert any output whose confirmation deadline has passed. Driven by
    /// a calloop timer, which keeps firing even if rendering stalls — so a dark
    /// screen recovers on its own.
    pub fn check_hdr_confirmations(&mut self) {
        let now = std::time::Instant::now();
        let expired: Vec<String> = self
            .hdr_pending_confirm
            .iter()
            .filter(|(_, deadline)| now >= **deadline)
            .map(|(name, _)| name.clone())
            .collect();
        for name in expired {
            self.hdr_pending_confirm.remove(&name);
            warn!(output = %name, "HDR not confirmed in time — auto-reverting to SDR");
            let output = self
                .workspaces
                .outputs_iter()
                .find(|o| o.name() == name)
                .cloned();
            if let Some(output) = output {
                self.set_output_hdr(&output, false, 203);
            }
            safety::clear_marker(&name);
            self.hdr_ipc.notify_reverted(&name);
        }
    }

    /// Toggle HDR on a single output by committing the DRM connector properties.
    /// Mirrors `vrr::set_output_vrr`: reach the surface via `with_compositor`,
    /// then schedule a render so the change flushes on the next page-flip.
    /// No-ops gracefully when the output has no HDR caps or the driver doesn't
    /// expose the properties.
    pub fn set_output_hdr(&mut self, output: &Output, enable: bool, sdr_nits: u32) {
        let _ = sdr_nits; // used by the render pipeline (Phase 4), not the props

        // Only meaningful on HDR-capable displays.
        let Some(caps) = output.user_data().get::<HdrCaps>().cloned() else {
            if enable {
                info!(output = %output.name(), "HDR requested but display is SDR-only");
            }
            return;
        };
        let Some(oid) = output.user_data().get::<UdevOutputId>().copied() else { return };
        let Some(conn) = output
            .user_data()
            .get::<UdevOutputModes>()
            .map(|m| m.connector_handle)
        else {
            return;
        };

        let name = output.name();

        // Write the crash marker BEFORE the risky commit. If this commit takes
        // the compositor down, the marker survives and the next startup forces
        // SDR — so the user can't get permanently locked out.
        if enable {
            safety::write_marker(&name);
        }

        let mut applied = false;
        {
            let Some(udev) = self.udev.as_mut() else { return };
            let Some(backend) = udev.backends.get_mut(&oid.device_id) else { return };
            let Some(surface) = backend.surfaces.get_mut(&oid.crtc) else { return };
            surface.drm_output.with_compositor(|comp| {
                let drm_surface = comp.surface();
                let handles = drm_props::resolve_props(drm_surface, conn);
                if !handles.any() {
                    if enable {
                        info!(
                            output = %name,
                            "HDR requested but driver exposes no HDR connector properties"
                        );
                    }
                    return;
                }
                drm_props::set_hdr_metadata(drm_surface, conn, &handles, &caps, enable);
                applied = true;
            });
        }

        if !applied {
            // Nothing committed → no marker should linger.
            safety::clear_marker(&name);
            return;
        }

        if enable {
            self.hdr_active_outputs.insert(name.clone());
            // Arm the auto-revert: the user must confirm within HDR_CONFIRM_SECS
            // or HDR is rolled back. The deadline is checked by a calloop timer
            // (input/timer sources keep running even if the GPU render stalls).
            let deadline = std::time::Instant::now() + Duration::from_secs(HDR_CONFIRM_SECS);
            self.hdr_pending_confirm.insert(name.clone(), deadline);
            self.arm_hdr_revert_timer();
            self.hdr_ipc.notify_pending(&name, HDR_CONFIRM_SECS as u32);
            info!(output = %name, "HDR engaged — awaiting confirmation ({HDR_CONFIRM_SECS}s)");
        } else {
            self.hdr_active_outputs.remove(&name);
            self.hdr_pending_confirm.remove(&name);
            safety::clear_marker(&name);
            info!(output = %name, "HDR disabled");
        }
        self.schedule_render();
    }

    /// Insert a one-shot calloop timer that re-checks pending HDR confirmations.
    /// Cheap and idempotent; we just register one each time HDR is engaged.
    fn arm_hdr_revert_timer(&mut self) {
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs(HDR_CONFIRM_SECS) + Duration::from_millis(500)),
            |_, _, state| {
                state.check_hdr_confirmations();
                TimeoutAction::Drop
            },
        );
    }
}

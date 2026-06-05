//! HDR support: EDID capability detection, DRM connector-property signalling,
//! the color-aware render pipeline, and the `wp_color_management_v1` server.
//!
//! HDR is opt-in per output (`hdr = true` in `[[monitors]]`) and only ever
//! engages on displays that report HDR support via EDID. Everything degrades
//! gracefully: an SDR-only display, or a GPU/driver that doesn't expose the
//! HDR connector properties, simply renders as it always has.

pub mod drm_props;
pub mod edid_caps;

pub use edid_caps::HdrCaps;

use smithay::output::Output;
use tracing::info;

use crate::hdr_ipc::OutputCaps;
use crate::state::Lantern;
use crate::udev::{UdevOutputId, UdevOutputModes};

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

    /// Poll the HDR IPC socket and apply any live enable/disable requests.
    pub fn poll_hdr_ipc(&mut self) {
        let commands = self.hdr_ipc.poll();
        for cmd in commands {
            let Some(output) = self
                .workspaces
                .outputs_iter()
                .find(|o| o.name() == cmd.output)
                .cloned()
            else {
                info!("HDR set request for unknown output {}", cmd.output);
                continue;
            };
            // Phase 2 wires this into the DRM connector-property apply. For now,
            // record the request so the round-trip is observable end to end.
            info!(
                "HDR set request: output={} enable={} sdr_nits={}",
                cmd.output, cmd.enable, cmd.sdr_nits
            );
            self.set_output_hdr(&output, cmd.enable, cmd.sdr_nits);
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

        if applied {
            if enable {
                self.hdr_active_outputs.insert(name.clone());
            } else {
                self.hdr_active_outputs.remove(&name);
            }
            info!(output = %name, enable, "HDR connector props committed");
            self.schedule_render();
        }
    }
}

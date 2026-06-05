//! HDR support: EDID capability detection, DRM connector-property signalling,
//! the color-aware render pipeline, and the `wp_color_management_v1` server.
//!
//! HDR is opt-in per output (`hdr = true` in `[[monitors]]`) and only ever
//! engages on displays that report HDR support via EDID. Everything degrades
//! gracefully: an SDR-only display, or a GPU/driver that doesn't expose the
//! HDR connector properties, simply renders as it always has.

pub mod edid_caps;

pub use edid_caps::HdrCaps;

use smithay::output::Output;
use tracing::info;

use crate::hdr_ipc::OutputCaps;
use crate::state::Lantern;

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

    /// Toggle HDR on a single output. Phase 2 fills this in with the real DRM
    /// connector-property commit; for Phase 0 it's a logging stub so the IPC
    /// path is testable on its own.
    pub fn set_output_hdr(&mut self, output: &Output, enable: bool, sdr_nits: u32) {
        let _ = (output, enable, sdr_nits);
    }
}

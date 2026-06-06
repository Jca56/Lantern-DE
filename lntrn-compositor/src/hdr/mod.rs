//! HDR support: EDID capability detection, DRM connector-property signalling,
//! the color-aware render pipeline, and the `wp_color_management_v1` server.
//!
//! HDR is opt-in per output (`hdr = true` in `[[monitors]]`) and only ever
//! engages on displays that report HDR support via EDID. Everything degrades
//! gracefully: an SDR-only display, or a GPU/driver that doesn't expose the
//! HDR connector properties, simply renders as it always has.

// `drm_props` holds the connector-property machinery. It is intentionally kept
// in the tree (and compiled) but NOT called — real HDR engagement is hard-
// disabled (see `set_output_hdr`). It documents the approach for the eventual
// correct (atomic + TEST_ONLY) reimplementation.
pub mod drm_props;
pub mod edid_caps;
pub mod safety;

pub use edid_caps::HdrCaps;

use smithay::output::Output;
use tracing::{info, warn};

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

    /// Confirm "Keep HDR" from the settings app. HDR engagement is currently
    /// hard-disabled, so there's nothing pending to keep — clear any stale state
    /// and acknowledge.
    pub fn confirm_hdr(&mut self, output: &str) {
        self.hdr_pending_confirm.remove(output);
        safety::clear_marker(output);
    }

    /// Engaging real HDR output is **hard-disabled**.
    ///
    /// The implementation (setting DRM connector properties — HDR_OUTPUT_METADATA,
    /// Colorspace, max bpc) hard-locked the machine three times on NVIDIA. Root
    /// causes, all confirmed:
    ///   * On an atomic-modeset driver (NVIDIA), a *legacy* `set_property` becomes
    ///     an implicit blocking modeset that wedges the single-threaded event loop
    ///     — taking input and the auto-revert timer down with it, so nothing can
    ///     recover. The correct path is one atomic commit (connector + CRTC +
    ///     plane) gated behind `DRM_MODE_ATOMIC_TEST_ONLY` — which Smithay 0.7
    ///     exposes no API for (requires a Smithay fork).
    ///   * 4K@240 10-bit BT.2020 exceeds the DisplayPort link budget, and NVIDIA
    ///     exposes no `max bpc` lever, so enabling HDR drops link sync → black.
    ///
    /// Until that's done correctly (atomic + TEST_ONLY, bandwidth-aware mode
    /// selection, never hardcoded to a monitor), this function MUST stay a no-op.
    /// Detection / UI / config plumbing remain live and harmless; the toggle just
    /// reports back that HDR couldn't be engaged.
    pub fn set_output_hdr(&mut self, output: &Output, enable: bool, sdr_nits: u32) {
        let _ = (sdr_nits, enable);
        let name = output.name();

        // Belt-and-suspenders: never leave a crash marker or pending state around.
        safety::clear_marker(&name);
        self.hdr_active_outputs.remove(&name);
        self.hdr_pending_confirm.remove(&name);

        if enable {
            warn!(
                output = %name,
                "HDR engage requested but is hard-disabled (unsafe on this stack — \
                 see hdr::set_output_hdr). No DRM properties were touched."
            );
            // Tell the settings app it didn't take, so the toggle reflects reality.
            self.hdr_ipc.notify_reverted(&name);
        }
    }
}

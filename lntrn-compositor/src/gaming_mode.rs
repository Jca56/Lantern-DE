//! Gaming Mode — drop the primary output to native scale (1.0) for fullscreen
//! X11 games.
//!
//! XWayland has a single GLOBAL coordinate scale, so on a mixed-scale
//! multi-monitor desktop (e.g. 4K@1.3 + 1080p@1.0) it cannot give games true
//! native resolution AND honour each monitor's fractional scale at the same
//! time — that mismatch is what produced black-quadrant rendering, cursor
//! desync, and wrong resolution/refresh in games.
//!
//! Rather than fight that, Gaming Mode temporarily sets the PRIMARY output
//! (where `place_new_window` sends X11 games) to scale 1.0: logical == physical,
//! so a fullscreen game renders 1:1 at the panel's real mode (e.g.
//! 3840x2160@240) with pixel-aligned input and no buffer/viewport mismatch.
//! Toggling it off restores the configured desktop scale. The secondary
//! monitor is never touched. Toggle with Super+G (before launching the game).

use crate::state::Lantern;

impl Lantern {
    /// Toggle Gaming Mode on the primary output (closest to the compositor
    /// origin — where X11 games spawn). Idempotent flip; safe to bind to a key.
    pub fn toggle_gaming_mode(&mut self) {
        // Don't hardcode an output name — this runs on both the single-display
        // laptop and the multi-monitor PC. Pick the output nearest the origin.
        let primary = self
            .workspaces
            .known_outputs()
            .min_by_key(|(_, loc)| loc.x.abs() + loc.y.abs())
            .map(|(o, _)| o.name());
        let Some(output_name) = primary else {
            tracing::warn!("Gaming mode: no output found");
            return;
        };

        self.gaming_mode = !self.gaming_mode;
        // ON → native 1.0 (logical == physical). OFF → configured desktop scale.
        let target_scale = if self.gaming_mode {
            1.0
        } else {
            crate::monitor_scale(&output_name)
        };

        tracing::info!(
            gaming_mode = self.gaming_mode,
            output = %output_name,
            target_scale,
            "Gaming mode toggled"
        );

        // Reuse the same path the System Settings scale slider uses — a real
        // output scale change (not a client_scale hack), so XWayland, the bar,
        // and layer surfaces all reconfigure coherently.
        let changes = vec![crate::handlers::output_management::OutputChange {
            output_name,
            drm_mode_index: None,
            position: None,
            scale: Some(target_scale),
        }];
        crate::udev_device::apply_output_config(self, changes);
    }
}

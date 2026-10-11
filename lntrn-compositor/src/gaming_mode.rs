//! Gaming Mode — drop the primary output to native scale (1.0) for fullscreen
//! X11 games.
//!
//! XWayland has a single GLOBAL coordinate scale, so on a mixed-scale
//! multi-monitor desktop (e.g. 4K@1.4 + 1080p@1.0) it cannot give games true
//! native resolution AND honour each monitor's fractional scale at the same
//! time — that mismatch is what produced black-quadrant rendering, cursor
//! desync, and wrong resolution/refresh in games.
//!
//! Rather than fight that, Gaming Mode temporarily sets the PRIMARY output
//! (where `place_new_window` sends X11 games) to scale 1.0: logical == physical,
//! so a fullscreen game renders 1:1 at the panel's real mode (e.g.
//! 3840x2160@240) with pixel-aligned input and no buffer/viewport mismatch.
//!
//! Because the primary's LOGICAL size grows when its scale drops (2742 → 3840
//! wide at 4K/1.4), the layout is resolved again afterwards (`output_layout`):
//! every other output keeps its side of the primary and moves out of its way,
//! windows included — no overlap, no dead seam the cursor can hide in.
//! Toggling off restores the configured desktop scale and the outputs close
//! back up. Toggle with Super+G (before launching the game) or the Command
//! Center tile.

use crate::handlers::output_management::OutputChange;
use crate::state::Lantern;

impl Lantern {
    /// Toggle Gaming Mode. Idempotent flip; safe to bind to a key.
    pub fn toggle_gaming_mode(&mut self) {
        let Some(primary_name) = self.gaming_primary_output() else {
            tracing::warn!("Gaming mode: no output found");
            return;
        };

        self.gaming_mode = !self.gaming_mode;

        tracing::info!(
            gaming_mode = self.gaming_mode,
            output = %primary_name,
            "Gaming mode toggled"
        );

        // Reuse the same path the System Settings scale slider uses — a real
        // output scale change (not a client_scale hack), so XWayland, the bar,
        // and layer surfaces all reconfigure coherently.
        let changes = self.gaming_layout_changes(&primary_name, self.gaming_mode);
        crate::udev_device::apply_output_config(self, changes);
        let enabled = self.gaming_mode;
        self.gaming_ipc.broadcast_state(enabled);
    }

    /// Re-apply the gaming scale for the current flag state. Called from
    /// `connector_connected` so the primary, hotplugged mid-game, doesn't
    /// stay at the desktop scale it comes up with.
    pub fn reapply_gaming_layout(&mut self) {
        if !self.gaming_mode {
            return;
        }
        let Some(primary_name) = self.gaming_primary_output() else {
            return;
        };
        let changes = self.gaming_layout_changes(&primary_name, true);
        if !changes.is_empty() {
            tracing::info!("Gaming mode: primary back at native scale after hotplug");
            crate::udev_device::apply_output_config(self, changes);
        }
    }

    /// The output Gaming Mode rescales: the same `primary = true` output that
    /// `place_new_window` sends X11 games to, falling back to the first
    /// output in workspace order (single-monitor laptop case).
    fn gaming_primary_output(&self) -> Option<String> {
        crate::primary_output_name()
            .filter(|n| self.workspaces.output_by_name(n).is_some())
            .or_else(|| self.workspaces.outputs_iter().next().map(|o| o.name()))
    }

    /// The OutputChange set for entering (`engage`) or leaving gaming mode:
    /// the primary's scale, and nothing else. The primary's logical size
    /// changes with it, and `relayout` moves the other outputs (with their
    /// windows) out of its way or back up against it — whichever side they
    /// are on, not only in a left-to-right row.
    fn gaming_layout_changes(&self, primary_name: &str, engage: bool) -> Vec<OutputChange> {
        let Some(primary) = self.workspaces.output_by_name(primary_name) else {
            return Vec::new();
        };
        let cur_scale = primary.current_scale().fractional_scale();
        let target_scale = if engage {
            1.0
        } else {
            crate::monitor_scale(primary_name)
        };
        if (target_scale - cur_scale).abs() <= 0.001 {
            return Vec::new();
        }
        vec![OutputChange {
            output_name: primary_name.to_string(),
            drm_mode_index: None,
            position: None,
            scale: Some(target_scale),
            enabled: None,
        }]
    }
}

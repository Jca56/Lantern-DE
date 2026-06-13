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
//! wide at 4K/1.4), every other output is re-positioned so the layout keeps a
//! clean left-to-right chain — no overlap, no dead seam the cursor can hide
//! in. Toggling off restores the configured desktop scale and the exact
//! `[[monitors]]` positions from lantern.toml. Toggle with Super+G (before
//! launching the game) or the Command Center tile.

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

    /// Re-apply the gaming layout for the current flag state. Called from
    /// `connector_connected` so an output hotplugged mid-game doesn't land on
    /// its configured position (computed for desktop scale → overlap).
    pub fn reapply_gaming_layout(&mut self) {
        if !self.gaming_mode {
            return;
        }
        let Some(primary_name) = self.gaming_primary_output() else {
            return;
        };
        let changes = self.gaming_layout_changes(&primary_name, true);
        if !changes.is_empty() {
            tracing::info!("Gaming mode: re-chaining outputs after hotplug");
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

    /// Compute the OutputChange set for entering (`engage`) or leaving gaming
    /// mode. The primary keeps its left edge and changes scale; every other
    /// output keeps its scale. X positions are re-chained left-to-right using
    /// post-change logical widths so outputs neither overlap nor leave a gap.
    /// On restore, outputs with a `[[monitors]]` entry snap back to their
    /// exact configured position instead (config is the source of truth).
    fn gaming_layout_changes(&self, primary_name: &str, engage: bool) -> Vec<OutputChange> {
        let mut snapshot: Vec<(String, i32, i32, f64, i32)> = self
            .workspaces
            .known_outputs()
            .filter_map(|(o, loc)| {
                let phys_w = o.current_mode()?.size.w;
                let scale = o.current_scale().fractional_scale();
                Some((o.name(), loc.x, loc.y, scale, phys_w))
            })
            .collect();
        snapshot.sort_by_key(|(_, x, _, _, _)| *x);

        let configs = if engage {
            Vec::new()
        } else {
            crate::read_monitor_configs()
        };
        let config_pos =
            |name: &str| configs.iter().find(|c| c.name == name).map(|c| (c.x, c.y));

        let mut changes = Vec::new();
        // Right edge of the previous output in the chain; None = leftmost
        // output, which anchors the chain at its current left edge.
        let mut next_x: Option<i32> = None;

        for (name, cur_x, cur_y, cur_scale, phys_w) in &snapshot {
            let is_primary = name == primary_name;
            let target_scale = if is_primary {
                if engage {
                    1.0
                } else {
                    crate::monitor_scale(name)
                }
            } else {
                *cur_scale
            };
            let logical_w = (*phys_w as f64 / target_scale).round() as i32;

            let (target_x, target_y) = match config_pos(name) {
                // Restoring with an explicit config entry: exact positions.
                Some(pos) => pos,
                None => (next_x.unwrap_or(*cur_x), *cur_y),
            };
            next_x = Some(target_x + logical_w);

            let scale_change =
                (is_primary && (target_scale - cur_scale).abs() > 0.001).then_some(target_scale);
            let pos_change =
                ((target_x, target_y) != (*cur_x, *cur_y)).then_some((target_x, target_y));
            if scale_change.is_some() || pos_change.is_some() {
                changes.push(OutputChange {
                    output_name: name.clone(),
                    drm_mode_index: None,
                    position: pos_change,
                    scale: scale_change,
                    enabled: None,
                });
            }
        }
        changes
    }
}

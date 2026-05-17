//! "Solo tile" puff: grow a focused window to the output minus exclusive
//! zones, inset by `SINGLE_WINDOW_OUTER_GAP` on all sides. Drives the
//! Super+Up / Super+Down ladder along with the existing minimize/maximize
//! state.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size, Serial},
};

use crate::state::{Lantern, SoloTiledWindow};
use crate::window_ext::WindowExt;
use crate::window_management::PoseSlot;

impl Lantern {
    /// True if the window is currently in solo-tile state. The entry
    /// persists across a subsequent maximize, so this stays `true` while
    /// the window is also maximized — the state ladder unwinds one rung
    /// at a time.
    pub fn is_solo_tiled(&self, surface: &WlSurface) -> bool {
        self.solo_tiled_windows
            .iter()
            .any(|entry| entry.surface == *surface)
    }

    /// Compute the "solo tile" target rect for a window on a given output:
    /// the output's logical bounds minus its exclusive zones (bar, layer
    /// shells), inset by `SINGLE_WINDOW_OUTER_GAP` on every side.
    fn solo_tile_target(
        &self,
        output: &smithay::output::Output,
    ) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(output);
        let work_x = geo.loc.x + left;
        let work_y = geo.loc.y + top;
        let work_w = geo.size.w - left - right;
        let work_h = geo.size.h - top - bot;
        if work_w <= 0 || work_h <= 0 { return None; }

        // Large rung — `size_large_pct` of the work area, centered. At 95%
        // (default) this leaves a small visible margin around the window;
        // pushing the slider higher essentially produces "near-fill".
        let pct = crate::size_large_pct();
        let w = (((work_w as f32) * pct).round() as i32).max(1).min(work_w);
        let h = (((work_h as f32) * pct).round() as i32).max(1).min(work_h);
        let x = work_x + (work_w - w) / 2;
        let y = work_y + (work_h - h) / 2;
        Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
    }

    /// Move a window into solo-tile state. Captures the current rect as
    /// the restore target so Super+Down can return it to normal size.
    /// No-op (returns false) if the window is already solo-tiled,
    /// maximized, or can't be located on any output.
    pub fn solo_tile_surface(&mut self, surface: &WlSurface) -> bool {
        if self.is_solo_tiled(surface) {
            return false;
        }
        if self.is_maximized(surface) {
            return false;
        }

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };
        let output = self.output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        let Some(target) = self.solo_tile_target(&output) else {
            return false;
        };

        let cur_loc = self.workspaces.element_location(&window).unwrap_or(target.loc);
        // If the window is currently in a pose slot (Left/Middle/Right half),
        // the Normal rung of the ladder is the Middle 1500×1000 rect, not
        // the tall half rect — see `half_pose.rs`. Otherwise prefer the
        // in-flight animation target (handles slow clients mid-resize; see
        // long-form rationale in `maximize_surface`).
        let restore = if self.posed_windows.contains_key(surface) {
            self.middle_pose_rect(&output)
                .unwrap_or_else(|| Rectangle::new(cur_loc, window.geometry().size))
        } else {
            self.window_state_anim
                .target_rect(surface)
                .unwrap_or_else(|| Rectangle::new(cur_loc, window.geometry().size))
        };
        self.posed_windows.remove(surface);

        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(restore);

        // Push the SoloTiledWindow entry BEFORE configure/remap/animate so
        // the renderer's state-target fallback chain (in render/surface.rs)
        // can immediately see solo state when it queries. Mirrors the
        // ordering in `maximize_surface`.
        self.solo_tiled_windows.push(SoloTiledWindow {
            surface: surface.clone(),
            window: window.clone(),
            restore,
            target,
        });

        window.configure_rect(target);
        self.remap_tracked_window(window.clone(), target.loc, true);
        self.window_state_anim
            .animate_default(surface, anim_start, target);
        true
    }

    /// Restore a solo-tiled window to its original normal-size rect.
    /// Returns false if the surface wasn't solo-tiled.
    pub fn unsolo_tile_surface(&mut self, surface: &WlSurface) -> bool {
        let idx = self
            .solo_tiled_windows
            .iter()
            .position(|entry| entry.surface == *surface);
        let Some(idx) = idx else { return false };
        let entry = self.solo_tiled_windows.remove(idx);

        let cur_loc = self
            .workspaces
            .element_location(&entry.window)
            .unwrap_or(entry.restore.loc);
        let geo = entry.window.geometry();
        let current_rect = Rectangle::new(cur_loc, geo.size);
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(current_rect);

        entry.window.configure_rect(entry.restore);
        self.remap_tracked_window(entry.window.clone(), entry.restore.loc, true);
        self.window_state_anim
            .animate_default(surface, anim_start, entry.restore);
        true
    }

    /// Forget solo-tile state for a surface (used on close/forget without
    /// triggering the restore animation).
    pub fn forget_solo_tile(&mut self, surface: &WlSurface) {
        self.solo_tiled_windows.retain(|e| e.surface != *surface);
    }

    // ── Super+Up / Super+Down state-ladder entry points ──────────────────

    /// Shift+Super+Up: state-grow ladder with two interjections.
    /// Priority:
    ///   1. Restore most-recently-minimized (LIFO) if any are minimized.
    ///   2. Corner-posed → step out to the half-pose for that column.
    ///   3. Tiny → step back up to Middle.
    ///   4. Half-posed (Left/Right) → shrink into top corner of that side.
    ///      (Per user spec: "fully L/R + Up/Down shrinks into the corner.")
    ///      This creates an intentional Half ↔ TopCorner toggle on repeated
    ///      Up presses; to grow past Half, cycle out via Shift+Super+L/R.
    ///   5. Normal grow: Normal → SoloTile → Maximized.
    pub fn ladder_size_up(&mut self) -> bool {
        let serial = Serial::from(0);
        if let Some(last) = self.minimized_windows.last().map(|e| e.surface.clone()) {
            return self.restore_minimized_surface(&last).is_some();
        }
        if self.try_uncorner_to_half() {
            return true;
        }
        if self.try_untiny_to_middle() {
            return true;
        }
        if self.try_corner_shrink_up() {
            return true;
        }
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        if self.is_maximized(&surface) {
            false
        } else if self.is_solo_tiled(&surface) {
            self.maximize_surface(&surface, serial)
        } else {
            self.solo_tile_surface(&surface)
        }
    }

    /// Shift+Super+Down: state-shrink ladder. Priority:
    ///   1. Maximized → unmax (SoloTile).
    ///   2. SoloTile → unsolo (Normal / Middle).
    ///   3. Half-posed (Left/Right) → shrink into bottom corner of that side.
    ///   4. Any corner (TL/TR/BL/BR) → regrow back to its half-pose.
    ///      Minimize from a corner is intentionally forbidden — corners are
    ///      "rest" positions, not a shortcut into the minimize tray.
    ///   5. Tiny (center) → Minimize. This is the ONLY way to minimize via
    ///      the ladder ("minimize only happens from the center").
    ///   6. Otherwise (Normal / Middle / unposed) → shrink to Tiny.
    pub fn ladder_size_down(&mut self) -> bool {
        let serial = Serial::from(0);
        let Some(surface) = self
            .focused_window()
            .and_then(|w| w.get_wl_surface())
        else {
            return false;
        };
        if self.is_maximized(&surface) {
            // Unmaximize: window returns to maximize.restore. If the
            // window was solo-tiled before maximize, that restore rect
            // IS the solo-tile rect, so the next Super+Down (handled in
            // a later press) will unsolo it back to normal.
            return self.unmaximize_surface(&surface, serial);
        }
        if self.is_solo_tiled(&surface) {
            return self.unsolo_tile_surface(&surface);
        }
        if self.try_corner_shrink_down() {
            return true;
        }
        if let Some(slot) = self.posed_windows.get(&surface).copied() {
            if slot.is_corner() {
                // Down from a corner regrows (same as Up). Corners never
                // minimize via the ladder.
                return self.try_uncorner_to_half();
            }
            if slot.is_tiny_variant() {
                return self.minimize_surface(&surface, serial);
            }
            // PoseSlot::Middle falls through to the Tiny shrink below.
        }
        self.pose_tiny()
    }
}

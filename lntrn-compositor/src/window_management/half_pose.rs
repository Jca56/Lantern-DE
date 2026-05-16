//! "Pose" the focused window to a Left half, Middle (1500×1000 centered),
//! or Right half slot. Shift+Super+Left/Right cycles through the slots:
//! `Left ↔ Middle ↔ Right`. From an unposed window, the first press jumps
//! straight to the requested half. From a posed window, subsequent presses
//! step through Middle. Edges (Left+Left, Right+Right) are no-ops.
//!
//! Pose is a one-shot animated resize — no edge resistance, no snap-aware
//! drag behavior. Rounded corners + SSD stay intact; the window is just
//! a normal free-floating window at the slot's rect.
//!
//! Middle is also used as the "Normal" rung of the Shift+Super+Up/Down
//! ladder, so unsoloing after a pose lands at Middle rather than at the
//! tall half rect — see `solo_tile.rs` and `maximize.rs`.

use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PoseSlot {
    Left,
    Middle,
    Right,
}

impl Lantern {
    pub fn pose_half_left(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let next = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Left) => return false,
            Some(PoseSlot::Middle) => PoseSlot::Left,
            Some(PoseSlot::Right) => PoseSlot::Middle,
            None => PoseSlot::Left,
        };
        self.apply_pose(&surface, next)
    }

    pub fn pose_half_right(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let next = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Right) => return false,
            Some(PoseSlot::Middle) => PoseSlot::Right,
            Some(PoseSlot::Left) => PoseSlot::Middle,
            None => PoseSlot::Right,
        };
        self.apply_pose(&surface, next)
    }

    /// Middle (1500×1000 default-window-size, centered) rect on the work
    /// area of the given output. Used both by `pose_half_*` for the Middle
    /// slot and by `solo_tile`/`maximize` as the captured "Normal" restore
    /// rect when the window is posed.
    pub fn middle_pose_rect(
        &self,
        output: &smithay::output::Output,
    ) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left_off, right_off) = self.exclusive_zone_offsets_for_output(output);
        let outer = crate::tiling::SINGLE_WINDOW_OUTER_GAP;

        let work_x = geo.loc.x + left_off + outer;
        let work_y = geo.loc.y + top + outer;
        let work_w = geo.size.w - left_off - right_off - outer * 2;
        let work_h = geo.size.h - top - bot - outer * 2;
        if work_w <= 0 || work_h <= 0 { return None; }

        let (def_w, def_h) = self.default_window_size.unwrap_or((1500, 1000));
        let w = def_w.min(work_w);
        let h = def_h.min(work_h);
        let x = work_x + (work_w - w) / 2;
        let y = work_y + (work_h - h) / 2;
        Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
    }

    fn pose_rect(
        &self,
        output: &smithay::output::Output,
        slot: PoseSlot,
    ) -> Option<Rectangle<i32, Logical>> {
        if slot == PoseSlot::Middle {
            return self.middle_pose_rect(output);
        }
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left_off, right_off) = self.exclusive_zone_offsets_for_output(output);
        let outer = crate::tiling::SINGLE_WINDOW_OUTER_GAP;
        // 20px gap between halves (10px on either side of center).
        let middle = outer / 2;

        let work_x = geo.loc.x + left_off + outer;
        let work_y = geo.loc.y + top + outer;
        let work_w = geo.size.w - left_off - right_off - outer * 2;
        let work_h = geo.size.h - top - bot - outer * 2;
        if work_w <= middle || work_h <= 0 { return None; }

        let half_w = (work_w - middle) / 2;
        let (x, w) = match slot {
            PoseSlot::Left => (work_x, half_w),
            PoseSlot::Right => (work_x + half_w + middle, work_w - half_w - middle),
            PoseSlot::Middle => unreachable!(),
        };
        Some(Rectangle::new(Point::from((x, work_y)), Size::from((w, work_h))))
    }

    fn apply_pose(&mut self, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, slot: PoseSlot) -> bool {
        let Some(window) = self.find_mapped_window(surface) else { return false };
        let output = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        let Some(target) = self.pose_rect(&output, slot) else { return false };

        // Drop any persistent state silently so subsequent ladder steps
        // treat this as a normal window at its current rect.
        self.maximized_windows.retain(|e| e.surface != *surface);
        self.solo_tiled_windows.retain(|e| e.surface != *surface);
        self.snapped_windows.retain(|e| e.surface != *surface);

        let cur_loc = self.workspaces.element_location(&window).unwrap_or(target.loc);
        let current_rect: Rectangle<i32, Logical> =
            Rectangle::new(cur_loc, window.geometry().size);
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(current_rect);

        self.posed_windows.insert(surface.clone(), slot);

        window.configure_rect(target);
        self.remap_tracked_window(window.clone(), target.loc, true);
        self.window_state_anim
            .animate_default(surface, anim_start, target);
        true
    }
}

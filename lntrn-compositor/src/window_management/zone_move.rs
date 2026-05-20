//! 9-zone "move preserving size" target for the focused window.
//!
//! Drives the Super+Arrow scheme. Each press moves the focused window
//! one cell in the arrow direction on a 3×3 grid covering the active
//! output's work area. Two quick presses naturally stack into a two-
//! cell jump because the animation redirects mid-flight. Cells at the
//! grid edge clamp (no wrap). Window size is preserved — the arrow
//! keys never resize (Super+Shift+Arrow handles that). If the focused
//! window is maximized, the maximize state is dropped first and the
//! move proceeds from the restore size.

use smithay::utils::{Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveZone {
    TopLeft,    TopCenter,    TopRight,
    MidLeft,    Center,       MidRight,
    BotLeft,    BotCenter,    BotRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowDir { Left, Right, Up, Down }

fn zone_from_grid(col: i32, row: i32) -> MoveZone {
    match (col.clamp(0, 2), row.clamp(0, 2)) {
        (0, 0) => MoveZone::TopLeft,
        (1, 0) => MoveZone::TopCenter,
        (2, 0) => MoveZone::TopRight,
        (0, 1) => MoveZone::MidLeft,
        (1, 1) => MoveZone::Center,
        (2, 1) => MoveZone::MidRight,
        (0, 2) => MoveZone::BotLeft,
        (1, 2) => MoveZone::BotCenter,
        _      => MoveZone::BotRight,
    }
}


impl Lantern {
    pub fn move_focused_to_zone(&mut self, zone: MoveZone) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };

        let captured_restore = self.take_maximized_restore(&surface);
        if captured_restore.is_some() {
            window.set_maximized(false);
            self.update_foreign_toplevel_states(&surface);
        }

        let output = self.output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        let Some(output_geo) = self.workspaces.output_geometry(&output) else { return false };

        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(&output);
        let gap = crate::default_gap();
        let work_x = output_geo.loc.x + left + gap;
        let work_y = output_geo.loc.y + top + gap;
        let work_w = (output_geo.size.w - left - right - 2 * gap).max(1);
        let work_h = (output_geo.size.h - top - bot - 2 * gap).max(1);

        let preserved_size = self.window_state_anim
            .target_rect(&surface).map(|r| r.size)
            .or_else(|| captured_restore.map(|r| r.size))
            .unwrap_or_else(|| window.geometry().size);

        let w = preserved_size.w.clamp(1, work_w);
        let h = preserved_size.h.clamp(1, work_h);

        let (col, row) = match zone {
            MoveZone::TopLeft    => (0, 0),
            MoveZone::TopCenter  => (1, 0),
            MoveZone::TopRight   => (2, 0),
            MoveZone::MidLeft    => (0, 1),
            MoveZone::Center     => (1, 1),
            MoveZone::MidRight   => (2, 1),
            MoveZone::BotLeft    => (0, 2),
            MoveZone::BotCenter  => (1, 2),
            MoveZone::BotRight   => (2, 2),
        };
        let x = match col {
            0 => work_x,
            1 => work_x + (work_w - w) / 2,
            _ => work_x + work_w - w,
        };
        let y = match row {
            0 => work_y,
            1 => work_y + (work_h - h) / 2,
            _ => work_y + work_h - h,
        };
        let target = Rectangle::new(Point::from((x, y)), Size::from((w, h)));

        self.posed_windows.remove(&surface);

        let cur_loc = self.workspaces.element_location(&window).unwrap_or(target.loc);
        let current_rect = Rectangle::new(cur_loc, window.geometry().size);
        let anim_start = self.window_state_anim.current_rect(&surface).unwrap_or(current_rect);
        self.animate_resize(&surface, &window, anim_start, target);
        true
    }

    /// Move the focused window one cell in the arrow direction on the
    /// 3×3 work-area grid. Clamps at the grid edge (no wrap). The
    /// current cell is determined from the window's center point.
    pub fn move_focused_one_cell(&mut self, arrow: ArrowDir) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };

        let output = self.output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        let Some(output_geo) = self.workspaces.output_geometry(&output) else { return false };

        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(&output);
        let gap = crate::default_gap();
        let work_x = output_geo.loc.x + left + gap;
        let work_y = output_geo.loc.y + top + gap;
        let work_w = (output_geo.size.w - left - right - 2 * gap).max(1);
        let work_h = (output_geo.size.h - top - bot - 2 * gap).max(1);

        // Locate current cell by matching the window's top-left against
        // the three cell anchor positions on each axis. Center-based
        // detection broke for windows tall/wide enough that their center
        // never reaches the outer thirds of the work area — e.g. a
        // 900-tall window at "BotCenter" position still had its center
        // in row 1, so a single Up press skipped Center and jumped to
        // TopCenter. Anchor-distance matching is invariant to size.
        let pending = self.window_state_anim.target_rect(&surface);
        let cur_loc = pending.map(|r| r.loc)
            .or_else(|| self.workspaces.element_location(&window))
            .unwrap_or(smithay::utils::Point::from((work_x, work_y)));
        let cur_size = pending.map(|r| r.size)
            .unwrap_or_else(|| window.geometry().size);
        let w = cur_size.w.clamp(1, work_w);
        let h = cur_size.h.clamp(1, work_h);

        let anchors_x = [
            work_x,
            work_x + (work_w - w) / 2,
            work_x + work_w - w,
        ];
        let anchors_y = [
            work_y,
            work_y + (work_h - h) / 2,
            work_y + work_h - h,
        ];
        let cur_col = (0..3i32)
            .min_by_key(|&i| (cur_loc.x - anchors_x[i as usize]).abs())
            .unwrap_or(1);
        let cur_row = (0..3i32)
            .min_by_key(|&i| (cur_loc.y - anchors_y[i as usize]).abs())
            .unwrap_or(1);

        // Cross-monitor: at the leftmost cell pressing Left jumps to the
        // adjacent left monitor's rightmost cell (and same row). Same for
        // right. If no adjacent output exists, clamp at the current edge.
        let cross = match arrow {
            ArrowDir::Left  if cur_col == 0 => self.cross_to_adjacent_output(&window, &output, output_geo, ArrowDir::Left,  cur_row, w, h),
            ArrowDir::Right if cur_col == 2 => self.cross_to_adjacent_output(&window, &output, output_geo, ArrowDir::Right, cur_row, w, h),
            _ => None,
        };
        if let Some(target) = cross {
            self.posed_windows.remove(&surface);
            let cur_loc_now = self.workspaces.element_location(&window).unwrap_or(target.loc);
            let current_rect = Rectangle::new(cur_loc_now, window.geometry().size);
            let anim_start = self.window_state_anim.current_rect(&surface).unwrap_or(current_rect);
            self.animate_resize(&surface, &window, anim_start, target);
            return true;
        }

        let (new_col, new_row) = match arrow {
            ArrowDir::Left  => ((cur_col - 1).max(0), cur_row),
            ArrowDir::Right => ((cur_col + 1).min(2), cur_row),
            ArrowDir::Up    => (cur_col, (cur_row - 1).max(0)),
            ArrowDir::Down  => (cur_col, (cur_row + 1).min(2)),
        };
        if (new_col, new_row) == (cur_col, cur_row) {
            return false;
        }
        self.move_focused_to_zone(zone_from_grid(new_col, new_row))
    }

    /// Find the output directly adjacent in `arrow` direction (left or right)
    /// and return the target rect placing `(w, h)` in column 2 (left jump)
    /// or column 0 (right jump) of that output's work area, at `dst_row`.
    /// Returns None if no adjacent output exists on that side.
    fn cross_to_adjacent_output(
        &mut self,
        window: &smithay::desktop::Window,
        current_output: &smithay::output::Output,
        current_geo: Rectangle<i32, smithay::utils::Logical>,
        arrow: ArrowDir,
        dst_row: i32,
        w: i32, h: i32,
    ) -> Option<Rectangle<i32, smithay::utils::Logical>> {
        let cur_left = current_geo.loc.x;
        let cur_right = current_geo.loc.x + current_geo.size.w;

        let mut best: Option<(i32, smithay::output::Output)> = None;
        for o in self.workspaces.outputs_iter() {
            if o == current_output { continue; }
            let Some(g) = self.workspaces.output_geometry(o) else { continue };
            let o_left = g.loc.x;
            let o_right = g.loc.x + g.size.w;
            let dist = match arrow {
                ArrowDir::Left  => cur_left - o_right,
                ArrowDir::Right => o_left - cur_right,
                _ => continue,
            };
            if dist < 0 { continue; }
            if best.as_ref().map_or(true, |(d, _)| dist < *d) {
                best = Some((dist, o.clone()));
            }
        }
        let (_, target_output) = best?;
        let target_geo = self.workspaces.output_geometry(&target_output)?;

        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(&target_output);
        let gap = crate::default_gap();
        let tw_x = target_geo.loc.x + left + gap;
        let tw_y = target_geo.loc.y + top + gap;
        let tw_w = (target_geo.size.w - left - right - 2 * gap).max(1);
        let tw_h = (target_geo.size.h - top - bot - 2 * gap).max(1);

        let w = w.clamp(1, tw_w);
        let h = h.clamp(1, tw_h);
        let new_x = match arrow {
            ArrowDir::Left  => tw_x + tw_w - w,
            ArrowDir::Right => tw_x,
            _ => tw_x,
        };
        let new_y = match dst_row {
            0 => tw_y,
            1 => tw_y + (tw_h - h) / 2,
            _ => tw_y + tw_h - h,
        };

        // Hand window ownership over to the destination output's active
        // workspace; remap_tracked_window uses the new location's monitor
        // to pick the new owner. Without this, future per-output queries
        // would still resolve to the source monitor and the window would
        // visually stop updating after crossing the seam.
        let new_loc = Point::from((new_x, new_y));
        self.remap_tracked_window(window.clone(), new_loc, true);

        Some(Rectangle::new(new_loc, Size::from((w, h))))
    }
}


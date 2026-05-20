//! Proportional resize locked to the output's aspect ratio.
//!
//! Drives the Super+Arrow scheme. Stages come from
//! `[windows].size_{small,medium,large}_pct` and apply to the work
//! area of the output the focused window lives on. The window is
//! always sized to the output's aspect ratio — different monitors
//! produce different ratios, no hardcoded values.
//!
//!   - Super+Up / Super+Right — grow to the next size stage
//!   - Super+Down / Super+Left — shrink to the prev size stage
//!
//! Clamps at small/large (no wrap). Auto-unmaximizes first if the
//! window was maximized. Always re-centers in the work area.

use smithay::utils::{Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeAction {
    Grow,
    Shrink,
}

impl Lantern {
    pub fn resize_focused(&mut self, action: ResizeAction) -> bool {
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

        let stages = [
            crate::size_small_pct(),
            crate::size_medium_pct(),
            crate::size_large_pct(),
        ];

        let pending = self.window_state_anim.target_rect(&surface);
        let cur_loc = pending.map(|r| r.loc)
            .or_else(|| self.workspaces.element_location(&window))
            .unwrap_or_else(|| Point::from((work_x, work_y)));
        let cur_size = pending.map(|r| r.size)
            .or_else(|| captured_restore.map(|r| r.size))
            .unwrap_or_else(|| window.geometry().size);
        let cur_w = cur_size.w.max(1);
        let cur_h = cur_size.h.max(1);

        // Detect which column of the 9-zone grid the window sits in,
        // using the same anchor-distance method as zone_move. Edge
        // columns (0, 2) flip resize to vertical-only — same stage
        // cycle, but only height changes; width and x stay put.
        let anchors_x = [
            work_x,
            work_x + (work_w - cur_w) / 2,
            work_x + work_w - cur_w,
        ];
        let cur_col = (0..3i32)
            .min_by_key(|&i| (cur_loc.x - anchors_x[i as usize]).abs())
            .unwrap_or(1);
        let at_edge = cur_col == 0 || cur_col == 2;

        let (new_w, new_h, new_x, new_y) = if at_edge {
            // Vertical-only: keep width, cycle height through stages,
            // keep x at the edge anchor, re-center vertically.
            let cur_h_idx = nearest_stage_idx(cur_h as f32 / work_h as f32, &stages);
            let target_idx = match action {
                ResizeAction::Grow   => (cur_h_idx + 1).min(2),
                ResizeAction::Shrink => cur_h_idx.saturating_sub(1),
            };
            if target_idx == cur_h_idx {
                return false;
            }
            let new_h = ((work_h as f32) * stages[target_idx]).round() as i32;
            let new_h = new_h.clamp(1, work_h);
            let new_w = cur_w.clamp(1, work_w);
            let new_x = if cur_col == 0 { work_x } else { work_x + work_w - new_w };
            let new_y = work_y + (work_h - new_h) / 2;
            (new_w, new_h, new_x, new_y)
        } else {
            // Aspect-locked resize centered in the work area.
            let cur_pct = (cur_w as f32 / work_w as f32).max(cur_h as f32 / work_h as f32);
            let cur_idx = nearest_stage_idx(cur_pct, &stages);
            let aspect = (work_w as f32) / (work_h as f32);
            let target_idx = match action {
                ResizeAction::Grow   => (cur_idx + 1).min(2),
                ResizeAction::Shrink => cur_idx.saturating_sub(1),
            };
            let aspect_now = cur_w as f32 / cur_h as f32;
            let aspect_unchanged = (aspect - aspect_now).abs() < 0.01;
            if target_idx == cur_idx && aspect_unchanged {
                return false;
            }
            let stage_pct = stages[target_idx];
            let max_w = (work_w as f32) * stage_pct;
            let max_h = (work_h as f32) * stage_pct;
            let (new_w_f, new_h_f) = if max_w / aspect <= max_h {
                (max_w, max_w / aspect)
            } else {
                (max_h * aspect, max_h)
            };
            let new_w = (new_w_f.round() as i32).clamp(1, work_w);
            let new_h = (new_h_f.round() as i32).clamp(1, work_h);
            let new_x = work_x + (work_w - new_w) / 2;
            let new_y = work_y + (work_h - new_h) / 2;
            (new_w, new_h, new_x, new_y)
        };

        let target = Rectangle::new(Point::from((new_x, new_y)), Size::from((new_w, new_h)));

        self.posed_windows.remove(&surface);

        let live_loc = self.workspaces.element_location(&window).unwrap_or(target.loc);
        let current_rect = Rectangle::new(live_loc, window.geometry().size);
        let anim_start = self.window_state_anim.current_rect(&surface).unwrap_or(current_rect);
        self.animate_resize(&surface, &window, anim_start, target);
        true
    }
}

fn nearest_stage_idx(pct: f32, stages: &[f32; 3]) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (i, &s) in stages.iter().enumerate() {
        let d = (s - pct).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

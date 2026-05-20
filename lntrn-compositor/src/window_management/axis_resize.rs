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
        let cur_size = pending.map(|r| r.size)
            .or_else(|| captured_restore.map(|r| r.size))
            .unwrap_or_else(|| window.geometry().size);
        let cur_w = cur_size.w.max(1);
        let cur_h = cur_size.h.max(1);

        // The "current stage" is whichever bucket the LARGER axis fits
        // into relative to work area. Used as the index to step from.
        let cur_pct = (cur_w as f32 / work_w as f32).max(cur_h as f32 / work_h as f32);
        let cur_idx = nearest_stage_idx(cur_pct, &stages);

        // Aspect ratio is ALWAYS the output's — no per-window override,
        // no square/wide toggle. Detect from the work area so laptops
        // (16:9) and external monitors (e.g. 16:9 / 21:9 / 16:10) each
        // produce their native ratio without hardcoded values.
        let aspect = (work_w as f32) / (work_h as f32);
        let target_idx = match action {
            ResizeAction::Grow   => (cur_idx + 1).min(2),
            ResizeAction::Shrink => cur_idx.saturating_sub(1),
        };

        // Cycle no-op: already at the requested stage with this aspect.
        let aspect_now = cur_w as f32 / cur_h as f32;
        let aspect_unchanged = (aspect - aspect_now).abs() < 0.01;
        let stage_unchanged = target_idx == cur_idx;
        if stage_unchanged && aspect_unchanged {
            return false;
        }

        let stage_pct = stages[target_idx];
        // Fit a box of (aspect) into stage_pct * work_w by stage_pct * work_h.
        let max_w = (work_w as f32) * stage_pct;
        let max_h = (work_h as f32) * stage_pct;
        let (new_w_f, new_h_f) = if max_w / aspect <= max_h {
            (max_w, max_w / aspect)
        } else {
            (max_h * aspect, max_h)
        };
        let new_w = (new_w_f.round() as i32).clamp(1, work_w);
        let new_h = (new_h_f.round() as i32).clamp(1, work_h);

        // Re-center in work area.
        let new_x = work_x + (work_w - new_w) / 2;
        let new_y = work_y + (work_h - new_h) / 2;
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

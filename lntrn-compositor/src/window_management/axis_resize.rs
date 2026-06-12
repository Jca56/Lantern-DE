//! Edge-pinned axis resize — the second resize mode for the focused window.
//!
//! When a window hugs the LEFT or RIGHT edge of its output's usable area,
//! Super+Up/Down and Super+Left/Right stop running the centred size/aspect
//! ladders (see `smart_snap`) and instead resize one axis at a time, keeping
//! the window pinned to that edge:
//!   * **Super+Up/Down** → cycle HEIGHT through 1/3 → 2/3 → 3/3 of the work
//!     area, width untouched. Vertically the window keeps its current spot,
//!     re-anchoring to whichever of top / centre / bottom it's nearest (the
//!     same three stops the Super+Shift move uses).
//!   * **Super+Left/Right** → cycle WIDTH through 1/3 → 2/3 → 3/3, height
//!     untouched, staying flush to the pinned edge (Right = wider, Left =
//!     narrower).
//!
//! Thirds are measured against the snap region (output minus bars) inset by
//! `SINGLE_WINDOW_OUTER_GAP` on both ends, so a resized window lands exactly
//! where the Super+Shift move stops put it. Both ladders clamp at the ends.
//!
//! Detection is geometric: a window counts as edge-pinned when its left or
//! right edge sits within a gap-sized tolerance of the usable area's edge, so
//! both keyboard-moved and mouse-snapped windows qualify with no extra state.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

/// Which screen edge a window is currently pinned to.
#[derive(Clone, Copy)]
pub(crate) enum EdgePin {
    Left,
    Right,
}

impl Lantern {
    /// If the focused window hugs the left or right edge of its output's
    /// usable area, return which edge — otherwise `None`, meaning the centred
    /// size/aspect ladders apply. Maximized windows are never edge-pinned
    /// (they keep the unmaximize-to-size-ladder behaviour); a window touching
    /// BOTH edges (full width) is treated as left-pinned so Super+Left narrows
    /// from the right.
    pub(crate) fn focused_edge_pin(&self) -> Option<EdgePin> {
        let window = self.focused_window()?;
        let surface = window.get_wl_surface()?;
        if self.is_maximized(&surface) {
            return None;
        }
        let output = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        let region = self.snap_region(&output)?;
        let (loc, size) = self.op_start_rect(&window, &surface, region.loc);
        // Tolerance spans the gap variants a window may sit at (40px keyboard
        // stop, 8px snap gap, …) so any edge-hugging window is recognised.
        let tol = crate::SINGLE_WINDOW_OUTER_GAP + crate::default_gap();
        let near_left = loc.x - region.loc.x <= tol;
        let near_right = (region.loc.x + region.size.w) - (loc.x + size.w) <= tol;
        match (near_left, near_right) {
            (true, _) => Some(EdgePin::Left),
            (false, true) => Some(EdgePin::Right),
            (false, false) => None,
        }
    }

    /// Super+Up/Down for an edge-pinned window: cycle the HEIGHT through the
    /// 1/3 / 2/3 / 3/3 thirds, keeping the width and horizontal pin. The window
    /// re-anchors vertically to whichever of top / centre / bottom it's
    /// nearest, so its rough vertical position is preserved. `grow` = taller.
    pub(crate) fn axis_resize_height(&mut self, grow: bool) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };
        let Some(region) = self.edge_op_region(&window, &surface) else { return false };

        let (loc, size) = self.op_start_rect(&window, &surface, region.loc);
        let gap = crate::SINGLE_WINDOW_OUTER_GAP;
        let thirds = axis_thirds(region.size.h - 2 * gap, gap);
        let cur = nearest_i32_idx(size.h, &thirds);
        let next = step_idx(cur, grow);
        if next == cur {
            return false;
        }
        let new_h = thirds[next].clamp(1, region.size.h);

        // Re-anchor to the nearest of the three vertical stops, classified from
        // the CURRENT height so a tall window near the bottom stays at bottom.
        let stop = nearest_i32_idx(loc.y, &axis_stops(region.loc.y, region.size.h, size.h, gap));
        let ny = axis_stops(region.loc.y, region.size.h, new_h, gap)[stop];

        let target = Rectangle::new(Point::from((loc.x, ny)), Size::from((size.w, new_h)));
        self.commit_edge_resize(&surface, &window, target);
        true
    }

    /// Super+Left/Right for an edge-pinned window: cycle the WIDTH through the
    /// 1/3 / 2/3 / 3/3 thirds, keeping the height and vertical position. The
    /// window stays flush to its pinned edge — `wider` (Super+Right) grows it,
    /// `!wider` (Super+Left) shrinks it.
    pub(crate) fn axis_resize_width(&mut self, wider: bool, pin: EdgePin) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };
        let Some(region) = self.edge_op_region(&window, &surface) else { return false };

        let (loc, size) = self.op_start_rect(&window, &surface, region.loc);
        let gap = crate::SINGLE_WINDOW_OUTER_GAP;
        let thirds = axis_thirds(region.size.w - 2 * gap, gap);
        let cur = nearest_i32_idx(size.w, &thirds);
        let next = step_idx(cur, wider);
        if next == cur {
            return false;
        }
        let new_w = thirds[next].clamp(1, region.size.w);
        let nx = match pin {
            EdgePin::Left => region.loc.x + gap,
            EdgePin::Right => region.loc.x + region.size.w - gap - new_w,
        };

        let target = Rectangle::new(Point::from((nx, loc.y)), Size::from((new_w, size.h)));
        self.commit_edge_resize(&surface, &window, target);
        true
    }

    /// Shared setup for an edge-pinned resize: drop any maximize so we reshape
    /// a normal rect, then return the snap region to measure thirds against.
    fn edge_op_region(
        &mut self,
        window: &Window,
        surface: &WlSurface,
    ) -> Option<Rectangle<i32, Logical>> {
        if self.take_maximized_restore(surface).is_some() {
            window.set_maximized(false);
            self.update_foreign_toplevel_states(surface);
        }
        let output = self
            .output_for_window(window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        self.snap_region(&output)
    }

    /// Clear stale pose/snap tracking and animate the focused window to its new
    /// edge-pinned rect. We've taken over the geometry, so any prior snap-zone
    /// entry no longer describes the window.
    fn commit_edge_resize(
        &mut self,
        surface: &WlSurface,
        window: &Window,
        target: Rectangle<i32, Logical>,
    ) {
        self.posed_windows.remove(surface);
        self.snapped_windows.retain(|e| e.surface != *surface);
        self.animate_focused_to(surface, window, target);
    }
}

/// The three heights/widths a window cycles through — 1/3, 2/3, 3/3 of the
/// available axis length, laid out as a 3-cell grid with `gap` BETWEEN cells so
/// adjacent windows (e.g. a 2/3 above a 1/3) don't touch or shadow-overlap.
/// `avail` already excludes the two outer edge gaps; `gap` is subtracted twice
/// more for the two inner gaps. The 2/3 size is two cells plus one inner gap;
/// 3/3 spans the whole `avail` (its inner gaps become window content).
fn axis_thirds(avail: i32, gap: i32) -> [i32; 3] {
    let avail = avail.max(1);
    let cell = ((avail - 2 * gap) / 3).max(1);
    [cell, (2 * cell + gap).min(avail), avail]
}

/// The three placement stops for a span of `len` within `[rpos, rlen]`, inset
/// by `gap`: start, centre, end. Mirrors `smart_snap::step_pos`.
fn axis_stops(rpos: i32, rlen: i32, len: i32, gap: i32) -> [i32; 3] {
    [rpos + gap, rpos + (rlen - len) / 2, rpos + rlen - len - gap]
}

/// Step one ladder index toward the high end (`forward`) or the low end,
/// clamped to the 3-stop range (no wrap).
fn step_idx(cur: usize, forward: bool) -> usize {
    if forward {
        (cur + 1).min(2)
    } else {
        cur.saturating_sub(1)
    }
}

/// Index of the value in `options` nearest `val`.
fn nearest_i32_idx(val: i32, options: &[i32]) -> usize {
    options
        .iter()
        .enumerate()
        .min_by_key(|(_, &o)| (o - val).abs())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

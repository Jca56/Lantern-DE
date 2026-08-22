/// Drag-to-edge snapping. Snapping here is PURE FLOATING: a drag-release near
/// an edge/corner move+resizes the window onto the 40px grid (shared with the
/// keyboard axis-resize) but leaves it an ordinary free-floating window — no
/// lock, no `snapped_windows` tracking, no eviction, no restore-on-redrag.
///
/// `SnapZone` / `snap_zone_geometry*` / `SnappedWindow` remain only because the
/// renderer, SSD and fullscreen paths still reference them; nothing populates
/// `snapped_windows` anymore, so those readers operate on an empty list.
use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
};

use crate::state::Lantern;

/// Zone a window can be snapped to — forms a 3×3 grid (halves + quarters + full).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    TopLeft,
    TopHalf,
    TopRight,
    Left,
    Full,
    Right,
    BottomLeft,
    BottomHalf,
    BottomRight,
}

/// Saved state for a snapped window. Legacy: retained for the renderer/SSD
/// fallback rect and corner-rounding lookups; no longer written to.
#[derive(Clone)]
pub struct SnappedWindow {
    pub surface: WlSurface,
    pub zone: SnapZone,
    pub restore: Rectangle<i32, Logical>,
    /// Target rect — render fallback after the state animation finishes but
    /// before the client acks the new size, so the window doesn't briefly
    /// snap to its stale buffer size.
    pub target: Rectangle<i32, Logical>,
}

impl Lantern {
    /// Detect which snap zone the pointer is in, if any. 8px threshold —
    /// used for non-drag pointer queries.
    pub fn detect_snap_zone(&self, pos: Point<f64, Logical>) -> Option<SnapZone> {
        self.detect_snap_zone_with_threshold(pos, 8.0)
    }

    /// Wider snap-zone detection for active drags so the user doesn't have
    /// to slam the cursor into the very edge of the screen.
    pub fn detect_snap_zone_drag(&self, pos: Point<f64, Logical>) -> Option<SnapZone> {
        self.detect_snap_zone_with_threshold(pos, 30.0)
    }

    fn detect_snap_zone_with_threshold(
        &self,
        pos: Point<f64, Logical>,
        edge_threshold: f64,
    ) -> Option<SnapZone> {
        let output = self.output_at_point(pos)?;
        let geo = self.workspaces.output_geometry(&output)?;

        let near_left = pos.x - geo.loc.x as f64 <= edge_threshold;
        let near_right = (geo.loc.x + geo.size.w) as f64 - pos.x <= edge_threshold;
        let near_top = pos.y - geo.loc.y as f64 <= edge_threshold;
        let near_bottom = (geo.loc.y + geo.size.h) as f64 - pos.y <= edge_threshold;

        match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => Some(SnapZone::TopLeft),
            (_, true, true, _) => Some(SnapZone::TopRight),
            (true, _, _, true) => Some(SnapZone::BottomLeft),
            (_, true, _, true) => Some(SnapZone::BottomRight),
            // Top edge alone triggers maximize (handled separately)
            (true, _, _, _) => Some(SnapZone::Left),
            (_, true, _, _) => Some(SnapZone::Right),
            _ => None,
        }
    }

    /// Compute the target rectangle for a snap zone, respecting exclusive zones.
    pub fn snap_zone_geometry(&self, zone: SnapZone) -> Option<Rectangle<i32, Logical>> {
        let pointer_pos = self
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let output = self
            .output_at_point(pointer_pos)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        self.snap_zone_geometry_on_output(&output, zone)
    }

    /// Same as `snap_zone_geometry` but for a specific output.
    /// Applies tiling gaps: `outer_gap` from screen edges and `DEFAULT_GAP/2` between
    /// adjacent zones so snapped windows sit inside the tiled layout grid cleanly.
    pub fn snap_zone_geometry_on_output(
        &self,
        output: &Output,
        zone: SnapZone,
    ) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;

        let (top_excl, bottom_excl, left_excl, right_excl) =
            self.exclusive_zone_offsets_for_output(output);
        let outer = self.workspaces.outer_gap;
        let inner = crate::default_gap();
        let half_inner = inner / 2;

        let x = geo.loc.x + left_excl + outer;
        let y = geo.loc.y + top_excl + outer;
        let w = (geo.size.w - left_excl - right_excl - outer * 2).max(1);
        let h = (geo.size.h - top_excl - bottom_excl - outer * 2).max(1);

        let half_w = w / 2;
        let half_h = h / 2;
        let left_w = (half_w - half_inner).max(1);
        let right_x = x + half_w + half_inner;
        let right_w = (w - half_w - half_inner).max(1);
        let top_h = (half_h - half_inner).max(1);
        let bottom_y = y + half_h + half_inner;
        let bottom_h = (h - half_h - half_inner).max(1);

        let rect = match zone {
            SnapZone::Full => Rectangle::new((x, y).into(), (w, h).into()),
            SnapZone::Left => Rectangle::new((x, y).into(), (left_w, h).into()),
            SnapZone::Right => Rectangle::new((right_x, y).into(), (right_w, h).into()),
            SnapZone::TopHalf => Rectangle::new((x, y).into(), (w, top_h).into()),
            SnapZone::BottomHalf => Rectangle::new((x, bottom_y).into(), (w, bottom_h).into()),
            SnapZone::TopLeft => Rectangle::new((x, y).into(), (left_w, top_h).into()),
            SnapZone::TopRight => Rectangle::new((right_x, y).into(), (right_w, top_h).into()),
            SnapZone::BottomLeft => Rectangle::new((x, bottom_y).into(), (left_w, bottom_h).into()),
            SnapZone::BottomRight => {
                Rectangle::new((right_x, bottom_y).into(), (right_w, bottom_h).into())
            }
        };
        Some(rect)
    }

    /// Check if a window is currently snapped. Always false now (nothing
    /// populates `snapped_windows`); kept for the lifecycle centering guard.
    pub fn is_snapped(&self, surface: &WlSurface) -> bool {
        self.snapped_windows.iter().any(|e| e.surface == *surface)
    }

    /// Target rect for a FLOATING drag-snap, on the 40px grid shared with the
    /// keyboard axis-resize: `snap_region` inset by `SINGLE_WINDOW_OUTER_GAP`,
    /// with halves/quarters separated by the SAME gap. So two opposite halves —
    /// or four quarters — tile with a uniform 40px channel between and around
    /// them, and the result lands a window edge-pinned (Super+arrow axis-resize
    /// then works on it).
    fn floating_snap_rect(
        &self,
        output: &Output,
        zone: SnapZone,
    ) -> Option<Rectangle<i32, Logical>> {
        let region = self.snap_region(output)?;
        let gap = crate::SINGLE_WINDOW_OUTER_GAP;
        let full_w = (region.size.w - 2 * gap).max(1);
        let full_h = (region.size.h - 2 * gap).max(1);
        let half_w = ((full_w - gap) / 2).max(1);
        let half_h = ((full_h - gap) / 2).max(1);
        let left_x = region.loc.x + gap;
        let right_x = region.loc.x + region.size.w - gap - half_w;
        let top_y = region.loc.y + gap;
        let bottom_y = region.loc.y + region.size.h - gap - half_h;
        use SnapZone::*;
        let (x, y, w, h) = match zone {
            Left => (left_x, top_y, half_w, full_h),
            Right => (right_x, top_y, half_w, full_h),
            TopLeft => (left_x, top_y, half_w, half_h),
            TopRight => (right_x, top_y, half_w, half_h),
            BottomLeft => (left_x, bottom_y, half_w, half_h),
            BottomRight => (right_x, bottom_y, half_w, half_h),
            TopHalf => (left_x, top_y, full_w, half_h),
            BottomHalf => (left_x, bottom_y, full_w, half_h),
            Full => (left_x, top_y, full_w, full_h),
        };
        Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
    }

    /// Move+resize the dragged window onto `zone` as a FLOATING window: it
    /// lands on the 40px grid but is NOT tracked, NOT locked, and evicts
    /// nothing — it stays an ordinary free-floating window. Replaces the legacy
    /// locked `apply_snap_with_eviction` on drag-release.
    pub fn apply_floating_snap(&mut self, surface: &WlSurface, zone: SnapZone) -> bool {
        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };
        let output = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        let Some(target) = self.floating_snap_rect(&output, zone) else {
            return false;
        };

        // Shed any leftover locked/posed/maximized state so this is clean float.
        if self.is_maximized(surface) {
            self.take_maximized_restore(surface);
            crate::window_ext::WindowExt::set_maximized(&window, false);
            self.update_foreign_toplevel_states(surface);
        }
        self.snapped_windows.retain(|e| e.surface != *surface);
        self.posed_windows.remove(surface);

        let live_loc = self
            .workspaces
            .element_location(&window)
            .unwrap_or(target.loc);
        let current_rect = Rectangle::new(live_loc, window.geometry().size);
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(current_rect);
        self.animate_resize(surface, &window, anim_start, target);
        self.schedule_client_render();
        tracing::info!(?zone, "Drag-snapped (floating) to zone");
        true
    }

    /// Check if pointer is at the top edge (for maximize-on-drag).
    /// Returns Some(()) if near the top edge but NOT near a corner.
    pub fn detect_top_edge(&self, pos: Point<f64, Logical>) -> Option<()> {
        const EDGE_THRESHOLD: f64 = 8.0;

        let output = self.output_at_point(pos)?;
        let geo = self.workspaces.output_geometry(&output)?;

        let near_left = pos.x - geo.loc.x as f64 <= EDGE_THRESHOLD;
        let near_right = (geo.loc.x + geo.size.w) as f64 - pos.x <= EDGE_THRESHOLD;
        let near_top = pos.y - geo.loc.y as f64 <= EDGE_THRESHOLD;

        if near_top && !near_left && !near_right {
            Some(())
        } else {
            None
        }
    }
}

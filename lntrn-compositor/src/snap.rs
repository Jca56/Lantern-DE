/// Snap zones: edge/corner window snapping with restore.

use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::state::Lantern;

/// Zone a window can be snapped to — forms a 3×3 grid (halves + quarters + full).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    TopLeft,    TopHalf,    TopRight,
    Left,       Full,       Right,
    BottomLeft, BottomHalf, BottomRight,
}

/// Direction for keyboard-driven zone cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneDir {
    Left,
    Right,
    Up,
    Down,
}

impl SnapZone {
    /// (col, row) in the 3×3 zone grid. col: 0=left 1=center 2=right, row: 0=top 1=middle 2=bottom.
    fn to_grid(self) -> (i32, i32) {
        match self {
            Self::TopLeft    => (0, 0), Self::TopHalf    => (1, 0), Self::TopRight    => (2, 0),
            Self::Left       => (0, 1), Self::Full       => (1, 1), Self::Right       => (2, 1),
            Self::BottomLeft => (0, 2), Self::BottomHalf => (1, 2), Self::BottomRight => (2, 2),
        }
    }

    fn from_grid(col: i32, row: i32) -> Self {
        match (col, row) {
            (0, 0) => Self::TopLeft,    (1, 0) => Self::TopHalf,    (2, 0) => Self::TopRight,
            (0, 1) => Self::Left,       (1, 1) => Self::Full,       (2, 1) => Self::Right,
            (0, 2) => Self::BottomLeft, (1, 2) => Self::BottomHalf, (2, 2) => Self::BottomRight,
            _ => Self::Full,
        }
    }

    /// Step one cell in a direction, clamped to the 3×3 grid.
    pub fn step(self, dir: ZoneDir) -> Self {
        let (mut c, mut r) = self.to_grid();
        match dir {
            ZoneDir::Left  => c = (c - 1).max(0),
            ZoneDir::Right => c = (c + 1).min(2),
            ZoneDir::Up    => r = (r - 1).max(0),
            ZoneDir::Down  => r = (r + 1).min(2),
        }
        Self::from_grid(c, r)
    }

    /// The grid cells this zone covers (in the 3×3 grid). Used for overlap checks.
    fn cells(self) -> &'static [(i32, i32)] {
        match self {
            Self::Full        => &[(0,0),(1,0),(2,0),(0,1),(1,1),(2,1),(0,2),(1,2),(2,2)],
            Self::TopHalf     => &[(0,0),(1,0),(2,0)],
            Self::BottomHalf  => &[(0,2),(1,2),(2,2)],
            Self::Left        => &[(0,0),(0,1),(0,2)],
            Self::Right       => &[(2,0),(2,1),(2,2)],
            Self::TopLeft     => &[(0,0)],
            Self::TopRight    => &[(2,0)],
            Self::BottomLeft  => &[(0,2)],
            Self::BottomRight => &[(2,2)],
        }
    }

    /// True if two zones share any grid cell (overlap when both placed).
    pub fn overlaps_zone(self, other: Self) -> bool {
        let a = self.cells();
        let b = other.cells();
        a.iter().any(|c| b.contains(c))
    }
}

/// Do two logical rectangles overlap (not merely touch)?
fn rects_overlap(a: &Rectangle<i32, Logical>, b: &Rectangle<i32, Logical>) -> bool {
    a.loc.x < b.loc.x + b.size.w
        && b.loc.x < a.loc.x + a.size.w
        && a.loc.y < b.loc.y + b.size.h
        && b.loc.y < a.loc.y + a.size.h
}

/// Pick the largest free zone that doesn't overlap any already-taken zone.
/// Halves come first so we fill space efficiently before falling back to quarters.
fn find_free_zone(taken: &[SnapZone]) -> Option<SnapZone> {
    use SnapZone::*;
    let candidates = [Left, Right, TopHalf, BottomHalf, TopLeft, TopRight, BottomLeft, BottomRight];
    candidates
        .into_iter()
        .find(|z| taken.iter().all(|t| !t.overlaps_zone(*z)))
}

/// Saved state for a snapped window so we can restore it.
#[derive(Clone)]
pub struct SnappedWindow {
    pub surface: WlSurface,
    pub zone: SnapZone,
    pub restore: Rectangle<i32, Logical>,
}

impl Lantern {
    /// Detect which snap zone the pointer is in, if any.
    /// `pos` is the pointer position in logical coordinates.
    pub fn detect_snap_zone(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<SnapZone> {
        const EDGE_THRESHOLD: f64 = 8.0;

        let output = self.output_at_point(pos)?;
        let geo = self.space.output_geometry(&output)?;

        let near_left = pos.x - geo.loc.x as f64 <= EDGE_THRESHOLD;
        let near_right = (geo.loc.x + geo.size.w) as f64 - pos.x <= EDGE_THRESHOLD;
        let near_top = pos.y - geo.loc.y as f64 <= EDGE_THRESHOLD;
        let near_bottom = (geo.loc.y + geo.size.h) as f64 - pos.y <= EDGE_THRESHOLD;

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
        let pointer_pos = self.seat.get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let output = self.output_at_point(pointer_pos)
            .or_else(|| self.space.outputs().next().cloned())?;
        self.snap_zone_geometry_on_output(&output, zone)
    }

    /// Same as `snap_zone_geometry` but for a specific output (used by keyboard cycling).
    /// Applies tiling gaps: `outer_gap` from screen edges and `DEFAULT_GAP/2` between
    /// adjacent zones so snapped windows sit inside the tiled layout grid cleanly.
    pub fn snap_zone_geometry_on_output(
        &self,
        output: &Output,
        zone: SnapZone,
    ) -> Option<Rectangle<i32, Logical>> {
        let geo = self.space.output_geometry(output)?;

        let (top_excl, bottom_excl, left_excl, right_excl) = self.exclusive_zone_offsets_for_output(output);
        let outer = self.workspaces.outer_gap;
        let inner = crate::tiling::DEFAULT_GAP;
        let half_inner = inner / 2;

        let x = geo.loc.x + left_excl + outer;
        let y = geo.loc.y + top_excl + outer;
        let w = (geo.size.w - left_excl - right_excl - outer * 2).max(1);
        let h = (geo.size.h - top_excl - bottom_excl - outer * 2).max(1);

        let half_w = w / 2;
        let half_h = h / 2;
        let left_w   = (half_w - half_inner).max(1);
        let right_x  = x + half_w + half_inner;
        let right_w  = (w - half_w - half_inner).max(1);
        let top_h    = (half_h - half_inner).max(1);
        let bottom_y = y + half_h + half_inner;
        let bottom_h = (h - half_h - half_inner).max(1);

        let rect = match zone {
            SnapZone::Full        => Rectangle::new((x, y).into(), (w, h).into()),
            SnapZone::Left        => Rectangle::new((x, y).into(), (left_w, h).into()),
            SnapZone::Right       => Rectangle::new((right_x, y).into(), (right_w, h).into()),
            SnapZone::TopHalf     => Rectangle::new((x, y).into(), (w, top_h).into()),
            SnapZone::BottomHalf  => Rectangle::new((x, bottom_y).into(), (w, bottom_h).into()),
            SnapZone::TopLeft     => Rectangle::new((x, y).into(), (left_w, top_h).into()),
            SnapZone::TopRight    => Rectangle::new((right_x, y).into(), (right_w, top_h).into()),
            SnapZone::BottomLeft  => Rectangle::new((x, bottom_y).into(), (left_w, bottom_h).into()),
            SnapZone::BottomRight => Rectangle::new((right_x, bottom_y).into(), (right_w, bottom_h).into()),
        };
        Some(rect)
    }

    /// Snap a window to a zone. Saves the pre-snap geometry for later restore.
    pub fn snap_window_to_zone(&mut self, surface: &WlSurface, zone: SnapZone) -> bool {
        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        let Some(target) = self.snap_zone_geometry(zone) else {
            return false;
        };

        // If already snapped, remove old snap state (we'll overwrite with new zone)
        // But keep the *original* restore geometry if re-snapping from another zone.
        let restore = if let Some(idx) = self.snapped_windows.iter().position(|e| e.surface == *surface) {
            self.snapped_windows.remove(idx).restore
        } else if let Some(idx) = self.maximized_windows.iter().position(|e| e.surface == *surface) {
            // Unsnap from maximized state first
            self.maximized_windows.remove(idx).restore
        } else {
            let location = self.space.element_location(&window).unwrap_or_default();
            Rectangle::new(location, window.geometry().size)
        };

        self.snapped_windows.push(SnappedWindow {
            surface: surface.clone(),
            zone,
            restore,
        });

        crate::window_ext::WindowExt::configure_size(&window, target.size);

        self.space.map_element(window, target.loc, true);
        self.schedule_client_render();
        tracing::info!(?zone, "Snapped window to zone");
        true
    }

    /// Unsnap a window and restore its pre-snap geometry.
    pub fn unsnap_window(&mut self, surface: &WlSurface) -> bool {
        let Some(idx) = self.snapped_windows.iter().position(|e| e.surface == *surface) else {
            return false;
        };
        let snap = self.snapped_windows.remove(idx);

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        crate::window_ext::WindowExt::configure_size(&window, snap.restore.size);

        self.space.map_element(window, snap.restore.loc, true);
        self.schedule_client_render();
        true
    }

    /// Check if a window is currently snapped.
    pub fn is_snapped(&self, surface: &WlSurface) -> bool {
        self.snapped_windows.iter().any(|e| e.surface == *surface)
    }

    /// Snap the currently focused window to a zone.
    pub fn snap_focused(&mut self, zone: SnapZone) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };
        let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&window) else { return false };
        self.snap_window_to_zone(&surface, zone)
    }

    /// Approximate which zone a rectangle best matches on a given output.
    /// Used as the starting point when cycling a window that hasn't been snapped yet.
    fn approximate_zone_for(
        &self,
        output: &Output,
        rect: Rectangle<i32, Logical>,
    ) -> SnapZone {
        let Some(geo) = self.space.output_geometry(output) else { return SnapZone::Full };
        let (top_excl, bottom_excl, left_excl, right_excl) = self.exclusive_zone_offsets_for_output(output);
        let x = geo.loc.x + left_excl;
        let y = geo.loc.y + top_excl;
        let w = (geo.size.w - left_excl - right_excl).max(1);
        let h = (geo.size.h - top_excl - bottom_excl).max(1);

        // Width class: if covers >75% → full, else left/right by center.
        let col = if rect.size.w * 4 >= w * 3 {
            1
        } else if rect.loc.x + rect.size.w / 2 < x + w / 2 {
            0
        } else {
            2
        };
        // Height class: same idea.
        let row = if rect.size.h * 4 >= h * 3 {
            1
        } else if rect.loc.y + rect.size.h / 2 < y + h / 2 {
            0
        } else {
            2
        };
        SnapZone::from_grid(col, row)
    }

    /// Cycle the focused window's snap zone one step in the given direction
    /// within a 3×3 grid. Other windows that would overlap the new rect get
    /// re-snapped to the largest free zone that fits.
    pub fn cycle_snap_focused(&mut self, dir: ZoneDir) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&window) else { return false };

        let output = self.output_for_window(&window)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else { return false };
        let output_name = output.name();

        // Determine current zone: explicit snap > approximate from rect.
        let current = if let Some(existing) = self.snapped_windows.iter().find(|s| s.surface == surface) {
            existing.zone
        } else {
            let loc = self.space.element_location(&window).unwrap_or_default();
            let rect = Rectangle::new(loc, window.geometry().size);
            self.approximate_zone_for(&output, rect)
        };

        let next = current.step(dir);
        if next == current {
            // At grid edge — no change, but still intercept the key.
            return true;
        }

        let Some(target) = self.snap_zone_geometry_on_output(&output, next) else { return false };

        // Snap the focused window first so we have its authoritative rect.
        self.apply_snap(&surface, next, target);

        // Collect other same-output windows whose current rect overlaps the new focused rect.
        let overlapping: Vec<(WlSurface, Rectangle<i32, Logical>)> = self.space.elements()
            .filter_map(|w| {
                let s = crate::window_ext::WindowExt::get_wl_surface(w)?;
                if s == surface { return None; }
                let same_output = self.output_for_window(w)
                    .map(|o| o.name() == output_name)
                    .unwrap_or(false);
                if !same_output { return None; }
                let loc = self.space.element_location(w).unwrap_or_default();
                let rect = Rectangle::new(loc, w.geometry().size);
                if rects_overlap(&rect, &target) { Some((s, rect)) } else { None }
            })
            .collect();

        // Place each overlapping window in the largest still-free zone.
        let mut taken: Vec<SnapZone> = vec![next];
        // Existing snapped windows (other than focused) also occupy their zones.
        for existing in &self.snapped_windows {
            if existing.surface != surface && !taken.contains(&existing.zone) {
                taken.push(existing.zone);
            }
        }
        for (other_surface, other_rect) in overlapping {
            let Some(free) = find_free_zone(&taken) else { break };
            let Some(other_window) = self.find_mapped_window(&other_surface) else { continue };
            let other_output = self.output_for_window(&other_window)
                .or_else(|| self.space.outputs().next().cloned());
            let Some(other_output) = other_output else { continue };
            let Some(other_target) = self.snap_zone_geometry_on_output(&other_output, free) else { continue };

            // Remove any stale snap/maximize entry, preserving restore rect if present.
            let _ = self.snapped_windows.iter().position(|e| e.surface == other_surface)
                .map(|i| self.snapped_windows.remove(i));
            let _ = self.maximized_windows.iter().position(|e| e.surface == other_surface)
                .map(|i| self.maximized_windows.remove(i));

            self.snapped_windows.push(SnappedWindow {
                surface: other_surface.clone(),
                zone: free,
                restore: Rectangle::new(other_rect.loc, other_rect.size),
            });
            crate::window_ext::WindowExt::configure_size(&other_window, other_target.size);
            self.space.map_element(other_window, other_target.loc, true);
            taken.push(free);
        }

        self.schedule_client_render();
        tracing::info!(?next, "Cycled snap zone");
        true
    }

    /// Snap a window to a zone given a precomputed rect. Updates/creates the
    /// snapped_windows entry and re-maps/resizes the window.
    fn apply_snap(
        &mut self,
        surface: &WlSurface,
        zone: SnapZone,
        target: Rectangle<i32, Logical>,
    ) {
        let Some(window) = self.find_mapped_window(surface) else { return };

        let restore = if let Some(idx) = self.snapped_windows.iter().position(|e| e.surface == *surface) {
            self.snapped_windows.remove(idx).restore
        } else if let Some(idx) = self.maximized_windows.iter().position(|e| e.surface == *surface) {
            self.maximized_windows.remove(idx).restore
        } else {
            let location = self.space.element_location(&window).unwrap_or_default();
            Rectangle::new(location, window.geometry().size)
        };

        self.snapped_windows.push(SnappedWindow {
            surface: surface.clone(),
            zone,
            restore,
        });

        crate::window_ext::WindowExt::configure_size(&window, target.size);
        self.space.map_element(window, target.loc, true);
    }

    /// Check if pointer is at the top edge (for maximize-on-drag).
    /// Returns Some(()) if near the top edge but NOT near a corner.
    pub fn detect_top_edge(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<()> {
        const EDGE_THRESHOLD: f64 = 8.0;

        let output = self.output_at_point(pos)?;
        let geo = self.space.output_geometry(&output)?;

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

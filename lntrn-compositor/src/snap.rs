/// Snap zones: edge/corner window snapping with restore.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::state::Lantern;

/// Edge/corner zone a window can be snapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
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

        let geo = self.space.outputs().next()
            .and_then(|output| self.space.output_geometry(output))?;

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
        let geo = self.space.outputs().next()
            .and_then(|output| self.space.output_geometry(output))?;

        let (top_excl, bottom_excl, left_excl, right_excl) = self.exclusive_zone_offsets();
        let x = geo.loc.x + left_excl;
        let y = geo.loc.y + top_excl;
        let w = geo.size.w - left_excl - right_excl;
        let h = geo.size.h - top_excl - bottom_excl;

        let half_w = w / 2;
        let half_h = h / 2;

        let rect = match zone {
            SnapZone::Left => Rectangle::new((x, y).into(), (half_w, h).into()),
            SnapZone::Right => Rectangle::new((x + half_w, y).into(), (w - half_w, h).into()),
            SnapZone::TopLeft => Rectangle::new((x, y).into(), (half_w, half_h).into()),
            SnapZone::TopRight => Rectangle::new((x + half_w, y).into(), (w - half_w, half_h).into()),
            SnapZone::BottomLeft => Rectangle::new((x, y + half_h).into(), (half_w, h - half_h).into()),
            SnapZone::BottomRight => Rectangle::new((x + half_w, y + half_h).into(), (w - half_w, h - half_h).into()),
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

    /// Check if pointer is at the top edge (for maximize-on-drag).
    /// Returns Some(()) if near the top edge but NOT near a corner.
    pub fn detect_top_edge(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<()> {
        const EDGE_THRESHOLD: f64 = 8.0;

        let geo = self.space.outputs().next()
            .and_then(|output| self.space.output_geometry(output))?;

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

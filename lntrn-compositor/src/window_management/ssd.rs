//! Server-side decoration hover/click handling + foreign-toplevel
//! state broadcasting + output-geometry helpers.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

use super::SsdClickAction;

impl Lantern {
    /// Compute and broadcast foreign-toplevel state flags for a surface.
    pub(crate) fn update_foreign_toplevel_states(&mut self, surface: &WlSurface) {
        // Protocol state constants
        const STATE_MAXIMIZED: u32 = 0;
        const STATE_MINIMIZED: u32 = 1;
        const STATE_ACTIVATED: u32 = 2;
        const STATE_FULLSCREEN: u32 = 3;

        let is_minimized = self.minimized_windows.iter().any(|e| e.surface == *surface);

        let mut states = Vec::new();
        // Don't advertise maximized while minimized — the window isn't
        // visibly maximized, and the bar uses this to decide docking.
        if self.is_maximized(surface) && !is_minimized {
            states.push(STATE_MAXIMIZED);
        }
        if is_minimized {
            states.push(STATE_MINIMIZED);
        }
        if self.focused_surface.as_ref() == Some(surface) {
            states.push(STATE_ACTIVATED);
        }
        if self.is_fullscreen(surface) {
            states.push(STATE_FULLSCREEN);
        }
        self.foreign_toplevel_state.set_states(surface, states);
    }

    pub(crate) fn window_output_geometry(
        &self,
        window: &Window,
    ) -> Option<Rectangle<i32, Logical>> {
        let output = self
            .output_for_window(window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        let geo = self.workspaces.output_geometry(&output)?;

        let mut result = Rectangle::new(geo.loc.into(), geo.size);

        // Subtract exclusive zones only from layer surfaces on this output
        let (top_excl, bottom_excl, left_excl, right_excl) =
            self.exclusive_zone_offsets_for_output(&output);
        result.loc.x += left_excl;
        result.loc.y += top_excl;
        result.size.w -= left_excl + right_excl;
        result.size.h -= top_excl + bottom_excl;
        Some(result)
    }

    /// Check if exclusive zone offsets changed and reconfigure maximized/snapped windows.
    pub fn check_exclusive_zone_change(&mut self) {
        let offsets = self.exclusive_zone_offsets();
        if offsets == self.last_exclusive_offsets {
            return;
        }
        self.last_exclusive_offsets = offsets;

        // Reconfigure all maximized windows with new geometry
        let maximized_surfaces: Vec<_> = self
            .maximized_windows
            .iter()
            .map(|e| e.surface.clone())
            .collect();
        for surface in &maximized_surfaces {
            if let Some(window) = self.find_mapped_window(surface) {
                if let Some(geo) = self.window_output_geometry(&window) {
                    window.configure_rect(geo);
                    self.remap_tracked_window(window, geo.loc, false);
                }
            }
        }

        // Reconfigure snapped windows too
        let snapped: Vec<_> = self
            .snapped_windows
            .iter()
            .map(|e| (e.surface.clone(), e.zone))
            .collect();
        for (surface, zone) in &snapped {
            if let Some(target) = self.snap_zone_geometry(*zone) {
                if let Some(window) = self.find_mapped_window(&surface) {
                    window.configure_rect(target);
                    self.remap_tracked_window(window, target.loc, false);
                }
            }
        }

        self.schedule_render();
    }

    // ── SSD interaction ─────────────────────────────────────────────────

    /// Update SSD hover state based on logical pointer position.
    /// Returns true if any hover state changed (needs re-render).
    pub fn ssd_update_hover(
        &mut self,
        pointer_pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        // This runs per motion event (up to 1000Hz) — bail before any
        // allocation when no window has server-side decorations (the common
        // case: CSD apps and fullscreen games).
        if self.ssd.windows.is_empty() {
            return false;
        }
        let mut changed = false;

        // Collect SSD surfaces first to avoid borrow conflict, reusing a
        // scratch buffer so the per-event allocation happens only once.
        let mut ssd_surfaces = std::mem::take(&mut self.ssd.hover_scratch);
        ssd_surfaces.clear();
        ssd_surfaces.extend(self.ssd.windows.keys().cloned());

        for surface in &ssd_surfaces {
            let window = match self.find_mapped_window(surface) {
                Some(w) => w,
                None => continue,
            };
            // Skip fullscreen windows (no SSD shown)
            if self.is_fullscreen(surface) {
                continue;
            }
            let win_loc = self
                .workspaces
                .element_location(&window)
                .unwrap_or_default();
            let win_size = window.geometry().size;

            let new_hover = match crate::ssd::hit_test(pointer_pos, win_loc, win_size) {
                Ok(btn) => btn,
                Err(()) => None, // Not over this window's decoration
            };

            if let Some(state) = self.ssd.get_mut(surface) {
                if state.hovered_button != new_hover {
                    state.hovered_button = new_hover;
                    changed = true;
                }
            }
        }

        self.ssd.hover_scratch = ssd_surfaces;
        changed
    }

    /// Handle a click on SSD decorations. Returns true if the click was consumed.
    /// `pointer_pos` is the pointer position in canvas-space.
    pub fn ssd_handle_click(
        &mut self,
        pointer_pos: smithay::utils::Point<f64, smithay::utils::Logical>,
        serial: smithay::utils::Serial,
    ) -> Option<SsdClickAction> {
        let ssd_surfaces: Vec<WlSurface> = self.ssd.windows.keys().cloned().collect();

        // Check front-to-back (space elements are front-first)
        for window in self.space.elements().cloned().collect::<Vec<_>>() {
            let Some(surface) = window.get_wl_surface() else {
                continue;
            };
            if !ssd_surfaces.contains(&surface) {
                continue;
            }
            if self.is_fullscreen(&surface) {
                continue;
            }
            // Skip windows on non-active workspaces — they're not visible.
            if let Some((out, ws)) = self.workspaces.window_workspace(&surface) {
                if ws != self.workspaces.active_id(&out) {
                    continue;
                }
            }
            let win_loc = self
                .workspaces
                .element_location(&window)
                .unwrap_or_default();
            let win_size = window.geometry().size;

            match crate::ssd::hit_test(pointer_pos, win_loc, win_size) {
                Ok(Some(crate::ssd::SsdButton::Close)) => {
                    self.focus_window(&window, serial);
                    return Some(SsdClickAction::Close(surface));
                }
                Ok(Some(crate::ssd::SsdButton::Maximize)) => {
                    self.focus_window(&window, serial);
                    return Some(SsdClickAction::ToggleMaximize(surface));
                }
                Ok(Some(crate::ssd::SsdButton::Minimize)) => {
                    self.focus_window(&window, serial);
                    return Some(SsdClickAction::Minimize(surface));
                }
                Ok(None) => {
                    // Drag area — initiate a move
                    self.focus_window(&window, serial);
                    return Some(SsdClickAction::Move(window));
                }
                Err(()) => continue, // Not over this decoration
            }
        }

        None
    }
}

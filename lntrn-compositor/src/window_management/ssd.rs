//! Server-side decoration hover/click handling + foreign-toplevel
//! state broadcasting + output-geometry helpers.

use smithay::{
    desktop::Window,
    output::Output,
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
        // Maximized, fullscreen and minimized all change here.
        self.broadcast_covered(false);
    }

    /// The work area of the output a window belongs to (see
    /// `output_for_window`).
    pub(crate) fn window_output_geometry(
        &self,
        window: &Window,
    ) -> Option<Rectangle<i32, Logical>> {
        let output = self
            .output_for_window(window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        self.output_work_area(&output)
    }

    /// An output's geometry less the exclusive zones of the layer surfaces
    /// on it.
    pub(crate) fn output_work_area(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let (top_excl, bottom_excl, left_excl, right_excl) =
            self.exclusive_zone_offsets_for_output(output);
        Some(Rectangle::new(
            (geo.loc.x + left_excl, geo.loc.y + top_excl).into(),
            (
                geo.size.w - left_excl - right_excl,
                geo.size.h - top_excl - bottom_excl,
            )
                .into(),
        ))
    }

    /// Re-fit maximized and solo-tiled windows when an output's exclusive
    /// zones changed. Compared per output: the insets summed over all of
    /// them stay the same when a panel moves from one monitor to the other,
    /// and that is exactly when both monitors' windows need re-fitting.
    pub fn check_exclusive_zone_change(&mut self) {
        if !self.remember_insets() {
            return;
        }
        self.reflow_held_windows();
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

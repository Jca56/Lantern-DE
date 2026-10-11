//! Where a new window goes: which output, and where on it.

use smithay::{
    desktop::Window,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

impl Lantern {
    /// The output of the window this one belongs to — a dialog's parent, an
    /// X11 transient's owner, a portal file chooser's importer. Such a
    /// window opens with its parent: by the usual rules (X11 on the primary,
    /// Wayland under the pointer) Steam on the second monitor put every
    /// dialog on the first, and Ctrl+O with the mouse resting on the other
    /// monitor opened the file dialog over there.
    fn parent_output(&self, window: &Window) -> Option<Output> {
        let parent = if let Some(x11) = window.x11_surface() {
            let id = x11.is_transient_for()?;
            self.space
                .elements()
                .find(|w| w.x11_surface().is_some_and(|p| p.window_id() == id))
                .cloned()?
        } else {
            let surface = window.get_wl_surface()?;
            let parent = window
                .toplevel()
                .and_then(|t| t.parent())
                .or_else(|| self.xdg_foreign_state.parent_of(&surface).cloned())?;
            self.find_mapped_window(&parent)?
        };
        self.output_for_window(&parent)
    }

    /// Returns a temporary position at the output center.
    /// The window will be repositioned to true center (accounting for its size)
    /// after its first commit, via `center_pending_window`.
    pub fn place_new_window(&mut self, window: &Window) -> Point<i32, Logical> {
        // X11 windows (Wine/Proton games, Steam launchers) ignore the
        // pointer's current output and always start on the primary monitor.
        // Why: the cursor often lives on whatever monitor Steam/Wine is on,
        // but games carry a saved fullscreen-resolution preference from a
        // previous launch on the primary. Spawning them on a smaller
        // secondary output triggers a configure/fullscreen oscillation as
        // the game keeps re-requesting its preferred size. Wayland-native
        // apps don't have this saved-resolution problem, so they keep the
        // friendlier "spawn under the cursor" behavior.
        //
        // A window that belongs to another one is the exception either way:
        // it opens where its parent is (see `parent_output`).
        let output = if let Some(parent) = self.parent_output(window) {
            Some(parent)
        } else if window.x11_surface().is_some() {
            self.workspaces.outputs_iter().next().cloned()
        } else {
            let pointer_pos = self
                .seat
                .get_pointer()
                .map(|p| p.current_location())
                .unwrap_or_default();
            self.output_at_point(pointer_pos)
                .or_else(|| self.workspaces.outputs_iter().next().cloned())
        };
        let Some(output_geo) = output.and_then(|o| self.workspaces.output_geometry(&o)) else {
            return (0, 0).into();
        };

        // Temporary: place at output center (will be refined after first commit)
        Point::from((
            output_geo.loc.x + output_geo.size.w / 2,
            output_geo.loc.y + output_geo.size.h / 2,
        ))
    }

    /// Called on commit: if a window is pending centering and now has real
    /// geometry, reposition it to the true center of its output with a
    /// cascade offset so stacked windows fan out.
    pub fn center_pending_window(&mut self, surface: &WlSurface) {
        if !self.pending_center.contains(surface) {
            return;
        }
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.get_wl_surface().as_ref() == Some(surface))
            .cloned()
        else {
            return;
        };
        let win_geo = window.geometry();
        if win_geo.size.w == 0 || win_geo.size.h == 0 {
            return; // Not ready yet
        }

        // If the window's position is already owned by some compositor state
        // (e.g. it requested fullscreen/maximize on startup, opened into a
        // pose slot, or it's a tiled-mode tile), DON'T re-center — the state
        // owner has already remapped it to the correct position, and centering
        // to the usable-area would shift it off by the exclusive-zone size,
        // breaking click hit-tests for the whole session.
        if self.is_fullscreen(surface)
            || self.is_maximized(surface)
            || self.is_snapped(surface)
            || self.posed_windows.contains_key(surface)
            || self
                .solo_tiled_windows
                .iter()
                .any(|e| e.surface == *surface)
            || self.workspaces.contains(surface)
            || self.window_state_anim.current_rect(surface).is_some()
        {
            self.pending_center.remove(surface);
            return;
        }

        self.pending_center.remove(surface);

        // Pick the output by the window's TOP-LEFT corner, not its center.
        // place_new_window puts the top-left at the placement-output's center
        // — but if the window's real geometry is huge (e.g. a Proton game
        // restoring 2560x1440 from a previous run), the window's logical
        // center lands on a different monitor than the one we placed it on.
        // Using the top-left keeps us anchored to the intended output, and
        // the centering math below + clamp will pull the window onto that
        // output's usable area.
        let placed_loc = self
            .workspaces
            .element_location(&window)
            .unwrap_or_default();
        //
        // Unless it has a parent: an xdg toplevel only says so after it was
        // created, so this first commit is the first chance to put it on
        // its parent's monitor.
        let output = self
            .parent_output(&window)
            .or_else(|| {
                self.output_at_point(Point::<f64, smithay::utils::Logical>::from((
                    placed_loc.x as f64,
                    placed_loc.y as f64,
                )))
            })
            .or_else(|| self.output_for_window(&window))
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(ref out) = output else { return };
        let Some(output_geo) = self.workspaces.output_geometry(out) else {
            return;
        };

        // Account for panels/bars so we center in the usable area
        let (top_excl, bottom_excl, left_excl, right_excl) =
            self.exclusive_zone_offsets_for_output(out);
        let usable_x = output_geo.loc.x + left_excl;
        let usable_y = output_geo.loc.y + top_excl;
        let usable_w = output_geo.size.w - left_excl - right_excl;
        let usable_h = output_geo.size.h - top_excl - bottom_excl;

        let x = usable_x + (usable_w - win_geo.size.w) / 2;
        let y = usable_y + (usable_h - win_geo.size.h) / 2;

        // Clamp to stay within usable area
        let x = x.clamp(
            usable_x,
            (usable_x + usable_w - win_geo.size.w).max(usable_x),
        );
        let y = y.clamp(
            usable_y,
            (usable_y + usable_h - win_geo.size.h).max(usable_y),
        );

        tracing::info!(
            x,
            y,
            w = win_geo.size.w,
            h = win_geo.size.h,
            "Centering new window"
        );
        self.remap_tracked_window(window, Point::from((x, y)), false);
    }
}

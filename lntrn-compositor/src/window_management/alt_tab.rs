//! Alt-Tab switcher + round-robin window cycling.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Serial,
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

impl Lantern {
    /// Round-robin window cycling including minimized windows.
    pub fn cycle_next_window(&mut self, serial: Serial) {
        // Build combined list: mapped windows + minimized windows (by surface)
        let mut all_surfaces: Vec<WlSurface> = self.space.elements()
            .filter_map(|w| w.get_wl_surface())
            .collect();
        for entry in &self.minimized_windows {
            if !all_surfaces.contains(&entry.surface) {
                all_surfaces.push(entry.surface.clone());
            }
        }

        if all_surfaces.len() < 2 {
            return;
        }

        let focused_idx = self.focused_surface.as_ref().and_then(|focused| {
            all_surfaces.iter().position(|s| s == focused)
        });

        let next_idx = match focused_idx {
            Some(idx) => (idx + 1) % all_surfaces.len(),
            None => 0,
        };

        let next_surface = all_surfaces[next_idx].clone();

        // If the target is minimized, restore it first
        if let Some(window) = self.restore_minimized_surface(&next_surface) {
            self.focus_window(&window, serial);
        } else if let Some(window) = self.find_mapped_window(&next_surface) {
            self.focus_window(&window, serial);
        }
    }

    /// Open the alt-tab switcher immediately in visible mode (for hot corner).
    /// The user clicks a thumbnail or presses ESC to dismiss.
    pub fn open_hot_corner_switcher(&mut self) {
        self.compact_window_mru();

        let all_surfaces = self.switcher_entries();

        if all_surfaces.len() < 2 {
            return;
        }

        let original = self.focused_surface.clone();
        let minimized: std::collections::HashSet<_> = self.minimized_windows
            .iter()
            .map(|m| m.surface.clone())
            .collect();

        self.alt_tab_switcher.start_visible(all_surfaces, original, minimized);
        self.schedule_render();
    }

    /// Switcher entry list: windows on the focused output's active workspace,
    /// plus any untracked mapped windows as a safety fallback (e.g. XWayland
    /// transients that never routed through the standard map path).
    /// Spawn-order preserved.
    fn switcher_entries(&self) -> Vec<WlSurface> {
        let focused_output = self.focused_output_name();
        self.window_spawn_order
            .iter()
            .filter(|s| self.find_any_window(s).is_some())
            .filter(|s| match self.workspaces.window_workspace(s) {
                Some((out, ws)) => {
                    focused_output.as_deref() == Some(out.as_str())
                        && ws == self.workspaces.active_id(&out)
                }
                None => true,
            })
            .cloned()
            .collect()
    }

    pub fn focus_next_window(&mut self, serial: Serial) -> bool {
        self.compact_window_mru();

        let all_surfaces = self.switcher_entries();

        let pending_surface = if self.alt_tab_switcher.is_active() {
            self.alt_tab_switcher.advance()
        } else {
            let original = self.focused_surface.clone();
            let minimized: std::collections::HashSet<_> = self.minimized_windows
                .iter()
                .map(|m| m.surface.clone())
                .collect();
            self.alt_tab_switcher.start_silent(all_surfaces, original, minimized)
        };

        let Some(surface) = pending_surface else {
            return false;
        };

        tracing::info!(
            switcher_entries = self.alt_tab_switcher.entry_count(),
            "Alt+Tab selected pending window"
        );
        let _ = serial;
        self.schedule_render();
        self.find_any_window(&surface).is_some()
    }

    pub fn hide_alt_tab_switcher(&mut self) {
        self.alt_tab_switcher.hide();
        self.schedule_render();
    }

    /// Close a window from the switcher overlay (close button click).
    /// Removes it from the switcher and sends close to the toplevel.
    /// Close all windows matching an app_id (used by hover preview close button).
    pub fn close_windows_by_app_id(&mut self, app_id: &str) {
        let surfaces: Vec<_> = self.foreign_toplevel_state.surface_app_ids()
            .into_iter()
            .filter(|(_, id)| id == app_id)
            .map(|(s, _)| s)
            .collect();
        for surface in surfaces {
            if let Some(window) = self.find_mapped_window(&surface) {
                window.request_close();
            } else if let Some(mw) = self.minimized_windows.iter().find(|m| m.surface == surface) {
                mw.window.request_close();
            }
        }
    }

    pub fn close_switcher_window(&mut self, index: usize) {
        let Some(surface) = self.alt_tab_switcher.remove_entry(index) else {
            return;
        };
        // Send close request to the window
        if let Some(window) = self.find_mapped_window(&surface) {
            window.request_close();
        } else if let Some(mw) = self.minimized_windows.iter().find(|m| m.surface == surface) {
            mw.window.request_close();
        }
        self.schedule_render();
    }

    pub fn commit_alt_tab(&mut self, serial: Serial) -> bool {
        let Some(surface) = self.alt_tab_switcher.selected_surface().cloned() else {
            return false;
        };

        self.alt_tab_switcher.hide();

        if let Some(window) = self.restore_minimized_surface(&surface) {
            self.focus_window(&window, serial);
            return true;
        }

        if let Some(window) = self.find_mapped_window(&surface) {
            self.focus_window(&window, serial);
            return true;
        }

        self.forget_window(&surface);
        self.schedule_render();
        false
    }

    /// Cancel Alt+Tab: restore the original focus and hide the overlay.
    pub fn cancel_alt_tab(&mut self, serial: Serial) {
        let original = self.alt_tab_switcher.original_focus().cloned();
        self.alt_tab_switcher.hide();

        if let Some(surface) = original {
            if let Some(window) = self.find_mapped_window(&surface) {
                self.focus_window(&window, serial);
                return;
            }
        }
        self.schedule_render();
    }
}

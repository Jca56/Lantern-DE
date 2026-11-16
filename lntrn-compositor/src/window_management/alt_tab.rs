//! Alt-Tab switcher + round-robin window cycling.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Point, Rectangle, Serial},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

/// The entries of `mru` that pass `listed`, in its order (most recent
/// first), then any others from `spawn` that pass it, in theirs.
fn recent_first<T: Clone + PartialEq>(
    mru: &[T],
    spawn: &[T],
    listed: impl Fn(&T) -> bool,
) -> Vec<T> {
    let mut entries: Vec<T> = mru.iter().filter(|s| listed(s)).cloned().collect();
    for entry in spawn.iter().filter(|s| listed(s)) {
        if !entries.contains(entry) {
            entries.push(entry.clone());
        }
    }
    entries
}

impl Lantern {
    /// Round-robin window cycling including minimized windows.
    pub fn cycle_next_window(&mut self, serial: Serial) {
        // Build combined list: mapped windows + minimized windows (by surface)
        let mut all_surfaces: Vec<WlSurface> = self
            .space
            .elements()
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

        let focused_idx = self
            .focused_surface
            .as_ref()
            .and_then(|focused| all_surfaces.iter().position(|s| s == focused));

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
        let minimized: std::collections::HashSet<_> = self
            .minimized_windows
            .iter()
            .map(|m| m.surface.clone())
            .collect();

        let app_ids = self.switcher_app_ids(&all_surfaces);
        self.alt_tab_switcher
            .start_visible(all_surfaces, app_ids, original, minimized);
        self.schedule_render();
    }

    /// Resolve the app_id for each entry surface (parallel to `surfaces`),
    /// for the switcher's icon badges. Falls back to an empty string when
    /// the window can't be found (icon resolution then yields no badge).
    fn switcher_app_ids(&self, surfaces: &[WlSurface]) -> Vec<String> {
        surfaces
            .iter()
            .map(|s| {
                self.find_any_window(s)
                    .map(|w| w.get_app_id())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Switcher entry list, most recently used first: the focused window,
    /// then the one used before it, and so on. A quick Alt+Tab picks the
    /// second entry, so it flips between the last two windows (the game and
    /// the browser) however many others are open. In the order windows were
    /// opened, "Alt+Tab back" from one of three windows landed on the third.
    ///
    /// Windows on EVERY output's active workspace, not just the cursor's: a
    /// fullscreen game alone on its monitor had nothing to switch to, so
    /// Alt+Tab couldn't get you to the other screen. Windows without a
    /// workspace are listed too (a safety fallback, e.g. XWayland transients
    /// that never routed through the standard map path), and any window
    /// missing from the MRU list follows in the order it was opened.
    fn switcher_entries(&self) -> Vec<WlSurface> {
        let listed = |s: &WlSurface| {
            self.find_any_window(s).is_some()
                && match self.workspaces.window_workspace(s) {
                    Some((out, ws)) => ws == self.workspaces.active_id(&out),
                    None => true,
                }
        };
        recent_first(&self.window_mru, &self.window_spawn_order, listed)
    }

    pub fn focus_next_window(&mut self, serial: Serial) -> bool {
        self.step_switcher(serial, false)
    }

    /// Backward sibling of [`focus_next_window`] — Shift+Tab / Left arrow.
    /// On a fresh start (first chord press is backward) it lands on the
    /// window *before* the focused one instead of after it.
    pub fn focus_prev_window(&mut self, serial: Serial) -> bool {
        self.step_switcher(serial, true)
    }

    /// Shared advance/retreat path for the Alt-Tab switcher. Starts the
    /// switcher in silent mode if it isn't active yet, otherwise steps the
    /// existing selection in `backward`'s direction.
    fn step_switcher(&mut self, serial: Serial, backward: bool) -> bool {
        self.compact_window_mru();

        let all_surfaces = self.switcher_entries();

        let pending_surface = if self.alt_tab_switcher.is_active() {
            if backward {
                self.alt_tab_switcher.retreat()
            } else {
                self.alt_tab_switcher.advance()
            }
        } else {
            let original = self.focused_surface.clone();
            let minimized: std::collections::HashSet<_> = self
                .minimized_windows
                .iter()
                .map(|m| m.surface.clone())
                .collect();
            let app_ids = self.switcher_app_ids(&all_surfaces);
            let started =
                self.alt_tab_switcher
                    .start_silent(all_surfaces, app_ids, original, minimized);
            // A fresh start lands on focused+1. For a backward first press we
            // want focused-1, so step back twice from there.
            if backward && started.is_some() {
                self.alt_tab_switcher.retreat();
                self.alt_tab_switcher.retreat()
            } else {
                started
            }
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
        self.close_switcher();
        self.schedule_render();
    }

    /// Take the switcher down. While it was up it kept the pointer away from
    /// every window (hovering picks a card instead), so pointer focus goes
    /// back to whatever is under the cursor once the loop is idle.
    fn close_switcher(&mut self) {
        self.alt_tab_switcher.hide();
        self.queue_pointer_sync();
    }

    /// Close a window from the switcher overlay (close button click).
    /// Removes it from the switcher and sends close to the toplevel.
    /// Close all windows matching an app_id (used by hover preview close button).
    pub fn close_windows_by_app_id(&mut self, app_id: &str) {
        let surfaces: Vec<_> = self
            .foreign_toplevel_state
            .surface_app_ids()
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

        self.close_switcher();
        // Which window a switch landed on: the first question of any "Alt+Tab
        // didn't focus it" report, and the log couldn't answer it.
        tracing::info!(
            app_id = %self.find_any_window(&surface).map(|w| w.get_app_id()).unwrap_or_default(),
            "Alt+Tab switching to window"
        );

        if let Some(window) = self.restore_minimized_surface(&surface) {
            self.focus_window(&window, serial);
            self.bring_cursor_to(&window);
            return true;
        }

        if let Some(window) = self.find_mapped_window(&surface) {
            self.focus_window(&window, serial);
            self.bring_cursor_to(&window);
            return true;
        }

        self.forget_window(&surface);
        self.schedule_render();
        false
    }

    /// After switching to a window on ANOTHER monitor, put the cursor in the
    /// middle of it, so it's ready to use. Out of a fullscreen game the
    /// cursor sat hidden over the game; you'd have had to find it and drag
    /// it across. Same-monitor switches leave the cursor where it is, unless
    /// a fullscreen window is hiding it and it isn't over the new window:
    /// out of a game to a window shown over it on the same monitor, the game
    /// goes on hiding the cursor wherever it still shows around that window.
    fn bring_cursor_to(&mut self, window: &Window) {
        let Some(pos) = self.seat.get_pointer().map(|p| p.current_location()) else {
            return;
        };
        let Some(output) = self.output_for_window(window) else {
            return;
        };
        let on_its_monitor = self.output_at_point(pos).as_ref() == Some(&output);
        if on_its_monitor && !self.cursor_hidden_by_fullscreen() {
            return;
        }
        let Some(output_geo) = self.workspaces.output_geometry(&output) else {
            return;
        };
        let loc = self.workspaces.element_location(window).unwrap_or(output_geo.loc);
        // The part of the window on its monitor: a window hanging off the
        // edge would put its middle off-screen.
        let visible = Rectangle::new(loc, window.geometry().size)
            .intersection(output_geo)
            .unwrap_or(output_geo);
        // Hidden but already over the window: it gets the pointer, and shows
        // its own cursor, without a jump.
        if on_its_monitor && visible.to_f64().contains(pos) {
            return;
        }
        let center = Point::from((
            visible.loc.x as f64 + visible.size.w as f64 / 2.0,
            visible.loc.y as f64 + visible.size.h as f64 / 2.0,
        ));
        self.warp_pointer_to(center);
        // Hand the window pointer focus at the new spot.
        self.refocus_pointer_at_cursor();
    }

    /// Cancel Alt+Tab: restore the original focus and hide the overlay.
    pub fn cancel_alt_tab(&mut self, serial: Serial) {
        let original = self.alt_tab_switcher.original_focus().cloned();
        self.close_switcher();

        if let Some(surface) = original {
            if let Some(window) = self.find_mapped_window(&surface) {
                self.focus_window(&window, serial);
                return;
            }
        }
        self.schedule_render();
    }
}

#[cfg(test)]
mod tests {
    use super::recent_first;

    #[test]
    fn most_recent_window_comes_first() {
        // Opened as game, browser, guide; used last: browser, then game.
        let spawn = ["game", "browser", "guide"];
        let mru = ["browser", "game", "guide"];
        assert_eq!(recent_first(&mru, &spawn, |_| true), ["browser", "game", "guide"]);
    }

    #[test]
    fn unlisted_windows_are_left_out() {
        let spawn = ["game", "browser", "guide"];
        let mru = ["browser", "game", "guide"];
        let entries = recent_first(&mru, &spawn, |w| *w != "game");
        assert_eq!(entries, ["browser", "guide"]);
    }

    #[test]
    fn windows_missing_from_the_mru_follow_in_spawn_order() {
        let spawn = ["game", "popup", "browser", "dialog"];
        let mru = ["browser", "game"];
        let entries = recent_first(&mru, &spawn, |_| true);
        assert_eq!(entries, ["browser", "game", "popup", "dialog"]);
    }
}

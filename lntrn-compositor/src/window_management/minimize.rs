//! Minimize / restore state and animations.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle, Serial, Size},
};

use crate::minimize_anim::MinimizeKind;
use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_state::MinimizedWindow;

impl Lantern {
    pub fn minimize_focused(&mut self, serial: Serial) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };

        let Some(surface) = window.get_wl_surface() else {
            return false;
        };
        self.minimize_surface(&surface, serial)
    }

    pub fn minimize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.minimize_surface(surface, Serial::from(0))
    }

    /// Compute the rect a window should minimize to / unminimize from.
    /// Bottom-middle of the output, ~80px above the bottom edge so the window
    /// looks like it's docking into the bar.
    pub(crate) fn minimize_target_for(&self, window: &Window) -> Rectangle<i32, Logical> {
        let output_geo = self
            .output_for_window(window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1920, 1080).into()));
        Self::minimize_target_in_output(output_geo)
    }

    /// Same trajectory as `minimize_target_for` but driven by an output
    /// rectangle directly. Used by the zombie close path where the window is
    /// already dead and we only have its last-known location.
    pub(crate) fn minimize_target_in_output(
        output_geo: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let icon_size = 48;
        let bottom_margin = 80;
        let cx = output_geo.loc.x + output_geo.size.w / 2;
        let cy = output_geo.loc.y + output_geo.size.h - bottom_margin;
        Rectangle::new(
            (cx - icon_size / 2, cy - icon_size / 2).into(),
            Size::from((icon_size, icon_size)),
        )
    }

    /// Look up the output that contains the given logical point (or the first
    /// output as a fallback) and return its minimize target rect.
    pub(crate) fn minimize_target_at_point(
        &self,
        point: smithay::utils::Point<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let output_geo = self
            .workspaces
            .outputs_iter()
            .find_map(|o| {
                self.workspaces.output_geometry(o).and_then(|g| {
                    if g.contains(point) {
                        Some(g)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                self.workspaces
                    .outputs_iter()
                    .next()
                    .and_then(|o| self.workspaces.output_geometry(o))
            })
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1920, 1080).into()));
        Self::minimize_target_in_output(output_geo)
    }

    pub(crate) fn minimize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        if self
            .minimized_windows
            .iter()
            .any(|entry| entry.surface == *surface)
        {
            return false;
        }
        // If a Minimize anim is already in flight, leave it alone.
        if self
            .minimize_anim
            .get(surface)
            .map(|a| a.kind == MinimizeKind::Minimize)
            .unwrap_or(false)
        {
            return false;
        }

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        let location = self
            .workspaces
            .element_location(&window)
            .unwrap_or_default();
        let source_rect = Rectangle::new(location, window.geometry().size);
        let target_rect = self.minimize_target_for(&window);

        // Start the animation; the window stays mapped and renders during
        // the anim. `finish_minimize_animation` does the actual unmap when
        // the anim's tick reports it as finished.
        self.minimize_anim
            .start_minimize(surface, source_rect, target_rect);

        // Clear focus immediately so the window doesn't look "active" while
        // it shrinks toward the tray icon.
        if serial != Serial::from(0) {
            self.clear_focus(serial);
        } else {
            self.focused_surface = None;
        }
        window.set_activated(false);

        self.schedule_render();
        true
    }

    /// Called by the render loop when a minimize animation has finished.
    /// Performs the actual unmap and bookkeeping that was deferred so the
    /// window could keep rendering during the shrink.
    pub fn finish_minimize_animation(&mut self, surface: &WlSurface) {
        let Some(window) = self.find_mapped_window(surface) else {
            return;
        };
        let location = self
            .workspaces
            .element_location(&window)
            .unwrap_or_default();
        self.minimized_windows.push(MinimizedWindow {
            surface: surface.clone(),
            window: window.clone(),
            location,
        });
        window.send_pending_configure();
        self.unmap_window_everywhere(&window);

        self.workspaces.remove(surface);

        // X11: perform a REAL iconify as far as the client can tell, without
        // unmapping the frame (which would make XWayland destroy the
        // wl_surface — see x11_wm_state.rs). Two signals, two audiences:
        //   - `_NET_WM_STATE_HIDDEN` → SDL2-style clients (they treat it as
        //     HIDDEN/MINIMIZED and drop exclusive fullscreen accordingly).
        //   - `WM_STATE = IconicState` → Wine/Proton, which arms a serial on
        //     XIconifyWindow and will NOT sync the Win32 window again until
        //     the WM acknowledges the iconify. Unanswered, the game stays
        //     Win32-minimized (DXVK stops presenting) and comes back black.
        if let Some(x11) = window.x11_surface().cloned() {
            if let Err(e) = x11.set_suspended(true) {
                tracing::warn!(class = x11.class(), "minimize: set_suspended(true) failed: {e}");
            }
            self.x11_set_iconic(&x11, true);
        }

        self.update_foreign_toplevel_states(surface);
        self.schedule_client_render();
    }

    /// Public alias for foreign-toplevel unset_minimized.
    pub fn restore_minimized_by_surface(&mut self, surface: &WlSurface) -> Option<Window> {
        self.restore_minimized_surface(surface)
    }

    pub(crate) fn restore_minimized_surface(&mut self, surface: &WlSurface) -> Option<Window> {
        let index = self
            .minimized_windows
            .iter()
            .position(|entry| entry.surface == *surface)?;
        let entry = self.minimized_windows.remove(index);

        let location = if self.is_maximized(surface) {
            // Window was maximized when minimized — restore to maximized
            // using current output geometry (not the pre-maximize restore rect).
            if let Some(output_geo) = self.window_output_geometry(&entry.window) {
                entry.window.set_maximized(true);
                entry.window.configure_rect(output_geo);
                output_geo.loc
            } else {
                entry.location
            }
        } else {
            entry.location
        };

        // Decide the output the window will live on from its restore
        // location (it's not yet in any Space, so `output_for_window`
        // can't help us).
        let output_name = self
            .output_at_point(smithay::utils::Point::from((
                location.x as f64,
                location.y as f64,
            )))
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .map(|o| o.name())
            .unwrap_or_default();
        let active_ws_id = self.workspaces.active_id(&output_name);

        // Re-register the window with its workspace BEFORE mapping. This
        // is the critical part: if we don't re-track here, the restored
        // window becomes a ghost — it lives in the global Space but no
        // workspace claims it, so workspace switches never hide it.
        if self.workspaces.window_workspace(&entry.surface).is_none() {
            self.workspaces
                .track_window(&output_name, entry.surface.clone());
        }

        self.map_window_in_workspace(
            entry.window.clone(),
            location,
            &output_name,
            active_ws_id,
            true,
        );

        // For X11 (XWayland) windows, undo the iconify from
        // `finish_minimize_animation`: clear `_NET_WM_STATE_HIDDEN` and set
        // `WM_STATE` back to Normal. Wine sees Iconic→Normal, syncs the Win32
        // window (SC_RESTORE) and the game re-enters exclusive fullscreen on
        // its own; SDL2 sees HIDDEN drop and fires RESTORED. `set_mapped(true)`
        // writes WM_STATE=Normal and maps the WM *frame* — which is still
        // mapped (we never unmapped it), so this cannot spawn a new surface or
        // a duplicate Window.
        if let Some(x11) = entry.window.x11_surface() {
            if let Err(e) = x11.set_suspended(false) {
                tracing::warn!(class = x11.class(), "restore: set_suspended(false) failed: {e}");
            }
            match x11.set_mapped(true) {
                Ok(()) => tracing::info!(class = x11.class(), "restore: X11 set_mapped(true)"),
                Err(e) => tracing::warn!(
                    class = x11.class(),
                    "restore: X11 set_mapped(true) failed: {e}"
                ),
            }
        }

        // Start the unminimize animation: emerge from the tray icon rect and
        // grow into the restored position.
        let source_rect = Rectangle::new(location, entry.window.geometry().size);
        let target_rect = self.minimize_target_for(&entry.window);
        self.minimize_anim
            .start_unminimize(&entry.surface, source_rect, target_rect);
        self.schedule_render();

        self.update_foreign_toplevel_states(&entry.surface);
        Some(entry.window)
    }
}

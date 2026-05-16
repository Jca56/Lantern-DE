//! Maximize / unmaximize state and animations.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle, Serial},
};

use crate::state::{Lantern, MaximizedWindow};
use crate::window_ext::WindowExt;

impl Lantern {
    pub fn toggle_maximize_focused(&mut self, serial: Serial) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };

        let Some(surface) = window.get_wl_surface() else { return false };
        if self.is_maximized(&surface) {
            self.unmaximize_surface(&surface, serial)
        } else {
            self.maximize_surface(&surface, serial)
        }
    }

    pub fn maximize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.maximize_surface(surface, Serial::from(0))
    }

    pub fn unmaximize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.unmaximize_surface(surface, Serial::from(0))
    }

    pub(crate) fn maximize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        let Some(window) = self.find_mapped_window(surface) else {
            tracing::warn!("maximize_surface: window not found in space");
            return false;
        };

        if self.is_maximized(surface) {
            tracing::info!("maximize_surface: already maximized");
            return false;
        }

        let Some(location) = self.workspaces.element_location(&window) else {
            tracing::warn!("maximize_surface: no element location");
            return false;
        };
        // If a state animation is already in flight (e.g. the user just
        // pressed Super+Up to grow the window and is now mashing Up again
        // to go straight to maximize), the live `window.geometry().size`
        // is stale — the client may not have acked the previous configure
        // yet. The animation's target rect is the size we *asked for*, so
        // use that as the canonical pre-maximize rect. Without this fix,
        // slow clients like Firefox would capture a wrong restore size,
        // and unmaximize would teleport the window to that wrong rect.
        let Some(output_geo) = self.window_output_geometry(&window) else {
            tracing::warn!("maximize_surface: no output geometry");
            return false;
        };
        // If the window is currently in a pose slot (Left/Middle/Right
        // half), the Normal rung of the ladder is the Middle 1500×1000
        // rect, not the tall half rect — see `half_pose.rs`.
        let restore = if self.posed_windows.contains_key(surface) {
            let output = self.output_for_window(&window)
                .or_else(|| self.workspaces.outputs_iter().next().cloned());
            output.as_ref().and_then(|o| self.middle_pose_rect(o))
                .unwrap_or_else(|| Rectangle::new(location, window.geometry().size))
        } else {
            self.window_state_anim
                .target_rect(surface)
                .unwrap_or_else(|| Rectangle::new(location, window.geometry().size))
        };
        self.posed_windows.remove(surface);
        let geo = window.geometry();
        tracing::info!(
            "maximize_surface: location={:?} geometry={:?} restore={:?} target={:?}",
            location, geo, restore, output_geo
        );

        self.maximized_windows.push(MaximizedWindow {
            surface: surface.clone(),
            restore,
            target: output_geo,
        });

        // Capture pre-maximize rect for animation start; if a previous rect
        // anim is already running for this surface, redirect from its current
        // interpolated rect instead.
        let existing_anim = self.window_state_anim.current_rect(surface);
        let anim_start = existing_anim.unwrap_or(restore);
        tracing::info!("maximize_surface: existing_anim={:?} anim_start={:?}", existing_anim, anim_start);

        window.set_maximized(true);
        window.configure_rect(output_geo);

        self.remap_tracked_window(window.clone(), output_geo.loc, true);
        self.window_state_anim.animate_default(surface, anim_start, output_geo);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        true
    }

    pub(crate) fn unmaximize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };
        let Some(restore) = self.take_maximized_restore(surface) else {
            return false;
        };

        // Animation start = current visible rect (handles redirect mid-maximize).
        let current_loc = self.workspaces.element_location(&window).unwrap_or(restore.loc);
        let geo = window.geometry();
        let current_rect = Rectangle::new(current_loc, geo.size);
        let existing_anim = self.window_state_anim.current_rect(surface);
        let anim_start = existing_anim.unwrap_or(current_rect);
        tracing::info!(
            "unmaximize_surface: current_loc={:?} geometry={:?} current_rect={:?} restore={:?} existing_anim={:?} anim_start={:?}",
            current_loc, geo, current_rect, restore, existing_anim, anim_start
        );

        window.set_maximized(false);
        window.configure_rect(restore);

        self.remap_tracked_window(window.clone(), restore.loc, true);
        self.window_state_anim.animate_default(surface, anim_start, restore);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        true
    }

    pub fn is_maximized(&self, surface: &WlSurface) -> bool {
        self.maximized_windows
            .iter()
            .any(|entry| entry.surface == *surface)
    }

    #[allow(dead_code)]
    pub(crate) fn maximized_restore(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.maximized_windows
            .iter()
            .find(|entry| entry.surface == *surface)
            .map(|entry| entry.restore)
    }

    pub(crate) fn take_maximized_restore(&mut self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        let index = self
            .maximized_windows
            .iter()
            .position(|entry| entry.surface == *surface)?;
        Some(self.maximized_windows.remove(index).restore)
    }
}

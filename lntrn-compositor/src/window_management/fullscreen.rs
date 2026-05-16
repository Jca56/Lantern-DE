//! Fullscreen state, animation, and the Wine-titlebar special case.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Point, Rectangle, Serial},
};

use crate::state::{FullscreenWindow, Lantern};
use crate::window_ext::WindowExt;

impl Lantern {
    pub fn is_fullscreen(&self, surface: &WlSurface) -> bool {
        self.fullscreen_windows.iter().any(|e| e.surface == *surface)
    }

    pub fn fullscreen_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        if self.is_fullscreen(surface) {
            return false;
        }

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        // Get the raw output geometry (no exclusive zone subtraction)
        let Some(output_geo) = self.output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
        else {
            return false;
        };

        // Save restore geometry
        let location = self.workspaces.element_location(&window).unwrap_or_default();
        let restore = Rectangle::new(location, window.geometry().size);

        // If maximized, use the maximized restore instead
        let restore = if let Some(max_restore) = self.take_maximized_restore(surface) {
            max_restore
        } else {
            restore
        };

        // If snapped, use the snapped restore instead
        let restore = if let Some(idx) = self.snapped_windows.iter().position(|e| e.surface == *surface) {
            self.snapped_windows.remove(idx).restore
        } else {
            restore
        };

        self.fullscreen_windows.push(FullscreenWindow {
            surface: surface.clone(),
            restore,
            target: output_geo,
        });

        // Capture animation start (or redirect from in-flight anim).
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or_else(|| Rectangle::new(location, window.geometry().size));

        window.set_fullscreen(true);
        window.configure_rect(output_geo);

        self.remap_tracked_window(window.clone(), output_geo.loc, true);
        self.window_state_anim.animate_default(surface, anim_start, output_geo);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        tracing::info!("Window entered fullscreen");
        true
    }

    pub fn unfullscreen_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        let Some(idx) = self.fullscreen_windows.iter().position(|e| e.surface == *surface) else {
            return false;
        };
        let restore = self.fullscreen_windows.remove(idx).restore;

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        let current_loc = self.workspaces.element_location(&window).unwrap_or(restore.loc);
        let current_rect = Rectangle::new(current_loc, window.geometry().size);
        let anim_start = self.window_state_anim.current_rect(surface).unwrap_or(current_rect);

        window.set_fullscreen(false);
        window.configure_rect(restore);

        self.remap_tracked_window(window.clone(), restore.loc, true);
        self.window_state_anim.animate_default(surface, anim_start, restore);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        tracing::info!("Window left fullscreen");
        true
    }

    pub fn toggle_fullscreen_focused(&mut self, serial: Serial) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };
        let Some(surface) = window.get_wl_surface() else { return false };
        if self.is_fullscreen(&surface) {
            self.unfullscreen_surface(&surface, serial)
        } else {
            self.fullscreen_surface(&surface, serial)
        }
    }

    pub fn fullscreen_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.fullscreen_surface(surface, Serial::from(0))
    }

    pub fn unfullscreen_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.unfullscreen_surface(surface, Serial::from(0))
    }

    /// Wine fullscreen: Wine draws its own titlebar inside the window surface,
    /// so we configure the window taller and shift it up to hide the titlebar.
    pub fn wine_fullscreen(&mut self, surface: &WlSurface, _x11: &smithay::xwayland::X11Surface) {
        // First do normal fullscreen
        if !self.fullscreen_surface(surface, Serial::from(0)) {
            return;
        }

        // Now adjust: Wine's titlebar is ~19px. We detect it from the frame
        // geometry — Wine reports y as a negative value for the frame offset.
        // The titlebar height is larger than the frame shadow, so we use a
        // known offset. Wine Win10 titlebar is ~19 logical pixels.
        let titlebar_h = 19;

        let Some(window) = self.find_mapped_window(surface) else { return };
        let Some(output_geo) = self.output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
        else { return };

        // Map shifted up so titlebar goes off-screen
        let adjusted_loc = Point::from((output_geo.loc.x, output_geo.loc.y - titlebar_h));
        let padded_size = smithay::utils::Size::from((
            output_geo.size.w,
            output_geo.size.h + titlebar_h,
        ));
        window.configure_rect(Rectangle::new(adjusted_loc, padded_size));
        self.remap_tracked_window(window, adjusted_loc, true);

        tracing::info!(
            titlebar_h,
            "Wine fullscreen: shifted window up to hide titlebar"
        );
    }
}

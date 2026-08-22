//! Fullscreen state, animation, and the Wine-titlebar special case.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Point, Rectangle, Serial},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_state::FullscreenWindow;

impl Lantern {
    pub fn is_fullscreen(&self, surface: &WlSurface) -> bool {
        self.fullscreen_windows
            .iter()
            .any(|e| e.surface == *surface)
    }

    pub fn fullscreen_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        if self.is_fullscreen(surface) {
            return false;
        }

        let Some(window) = self.find_mapped_window(surface) else {
            // Not on screen — maybe iconified. Record the state so the
            // window comes back fullscreen instead of dropping the request.
            return self.fullscreen_minimized(surface);
        };

        // Get the raw output geometry (no exclusive zone subtraction)
        let Some(output_geo) = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
        else {
            return false;
        };

        // Save restore geometry
        let location = self
            .workspaces
            .element_location(&window)
            .unwrap_or_default();
        let restore = Rectangle::new(location, window.geometry().size);

        // If maximized, use the maximized restore instead
        let restore = if let Some(max_restore) = self.take_maximized_restore(surface) {
            max_restore
        } else {
            restore
        };

        // If snapped, use the snapped restore instead
        let restore = if let Some(idx) = self
            .snapped_windows
            .iter()
            .position(|e| e.surface == *surface)
        {
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
        self.animate_resize(surface, &window, anim_start, output_geo);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
            // If the cursor is parked on ANOTHER output, pull it into the
            // fullscreened window first: the refocus below works on the
            // cursor's current position, and pointer focus on the wrong
            // monitor means the game never receives wl_pointer.enter — its
            // pointer-lock constraint can't activate, so mouse input is dead
            // until alt-tab. Same-output fullscreens (e.g. a video player
            // under the cursor) are left alone.
            if let Some(pos) = self.seat.get_pointer().map(|p| p.current_location()) {
                if !output_geo.contains(pos.to_i32_round()) {
                    let center = smithay::utils::Point::from((
                        output_geo.loc.x as f64 + output_geo.size.w as f64 / 2.0,
                        output_geo.loc.y as f64 + output_geo.size.h as f64 / 2.0,
                    ));
                    self.warp_pointer_to(center);
                }
            }
            // Establish POINTER focus too — keyboard focus alone leaves Proton
            // games unclickable when they fullscreen under a stationary cursor.
            self.refocus_pointer_at_cursor();
        } else {
            self.schedule_client_render();
        }
        self.refresh_vrr();
        tracing::info!("Window entered fullscreen");
        true
    }

    pub fn unfullscreen_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        let Some(idx) = self
            .fullscreen_windows
            .iter()
            .position(|e| e.surface == *surface)
        else {
            return false;
        };
        let restore = self.fullscreen_windows.remove(idx).restore;

        // Iconified window leaving fullscreen: DXGI/SDL drop exclusive
        // fullscreen when they minimize. Nothing is on screen to animate or
        // focus, but the X11 state MUST still be acknowledged — Wine won't
        // sync the Win32 window again while a `_NET_WM_STATE` request is
        // unanswered, which leaves the game black after restore.
        if let Some(entry) = self
            .minimized_windows
            .iter_mut()
            .find(|e| e.surface == *surface)
        {
            entry.location = restore.loc;
            let window = entry.window.clone();
            window.set_fullscreen(false);
            window.configure_rect(restore);
            self.update_foreign_toplevel_states(surface);
            self.refresh_vrr();
            tracing::info!("Window left fullscreen (while minimized)");
            return true;
        }

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        let current_loc = self
            .workspaces
            .element_location(&window)
            .unwrap_or(restore.loc);
        let current_rect = Rectangle::new(current_loc, window.geometry().size);
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(current_rect);

        window.set_fullscreen(false);
        self.animate_resize(surface, &window, anim_start, restore);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        self.refresh_vrr();
        tracing::info!("Window left fullscreen");
        true
    }

    /// Fullscreen bookkeeping for a window that is currently minimized: the
    /// client re-entered fullscreen while parked (typically right after we
    /// deiconified it and it restored its Win32/SDL state, before our map
    /// landed). Records the target so the restore maps it fullscreen, acks
    /// the X11 state, and deliberately does NOT animate or steal focus.
    fn fullscreen_minimized(&mut self, surface: &WlSurface) -> bool {
        let Some(idx) = self
            .minimized_windows
            .iter()
            .position(|e| e.surface == *surface)
        else {
            return false;
        };
        let (window, location) = {
            let entry = &self.minimized_windows[idx];
            (entry.window.clone(), entry.location)
        };
        let Some(output_geo) = self
            .output_at_point(Point::from((location.x as f64, location.y as f64)))
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
        else {
            return false;
        };

        let restore = self
            .take_maximized_restore(surface)
            .unwrap_or_else(|| Rectangle::new(location, window.geometry().size));
        self.fullscreen_windows.push(FullscreenWindow {
            surface: surface.clone(),
            restore,
            target: output_geo,
        });
        self.minimized_windows[idx].location = output_geo.loc;

        window.set_fullscreen(true);
        window.configure_rect(output_geo);
        self.update_foreign_toplevel_states(surface);
        tracing::info!("Window entered fullscreen (while minimized)");
        true
    }

    pub fn toggle_fullscreen_focused(&mut self, serial: Serial) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };
        let Some(surface) = window.get_wl_surface() else {
            return false;
        };
        if self.is_fullscreen(&surface) {
            self.unfullscreen_surface(&surface, serial)
        } else {
            self.fullscreen_surface(&surface, serial)
        }
    }

    pub fn fullscreen_request_surface(&mut self, surface: &WlSurface) -> bool {
        // A client-requested fullscreen is an unambiguous "this is the window
        // the user is looking at" signal. Games under Proton/Wine often map a
        // throwaway helper window that steals focus, then fullscreen their real
        // window — so we MUST move keyboard + X11 input focus to the window
        // being fullscreened, otherwise the game receives no input on launch
        // (Cyberpunk 2077, Helldivers 2) until the user alt-tabs into it.
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        self.fullscreen_surface(surface, serial)
    }

    pub fn unfullscreen_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.unfullscreen_surface(surface, Serial::from(0))
    }

    /// Wine fullscreen: Wine draws its own titlebar inside the window surface,
    /// so we configure the window taller and shift it up to hide the titlebar.
    pub fn wine_fullscreen(&mut self, surface: &WlSurface, _x11: &smithay::xwayland::X11Surface) {
        // First do normal fullscreen. Use a fresh serial so the window being
        // fullscreened also grabs keyboard + X11 input focus — see the note in
        // fullscreen_request_surface. Wine games map a focus-stealing helper
        // window before fullscreening their real one.
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if !self.fullscreen_surface(surface, serial) {
            return;
        }

        // Now adjust: Wine's titlebar is ~19px. We detect it from the frame
        // geometry — Wine reports y as a negative value for the frame offset.
        // The titlebar height is larger than the frame shadow, so we use a
        // known offset. Wine Win10 titlebar is ~19 logical pixels.
        let titlebar_h = 19;

        let Some(window) = self.find_mapped_window(surface) else {
            return;
        };
        let Some(output_geo) = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
        else {
            return;
        };

        // Map shifted up so titlebar goes off-screen
        let adjusted_loc = Point::from((output_geo.loc.x, output_geo.loc.y - titlebar_h));
        let padded_size =
            smithay::utils::Size::from((output_geo.size.w, output_geo.size.h + titlebar_h));
        window.configure_rect(Rectangle::new(adjusted_loc, padded_size));
        self.remap_tracked_window(window, adjusted_loc, true);

        tracing::info!(
            titlebar_h,
            "Wine fullscreen: shifted window up to hide titlebar"
        );
    }
}

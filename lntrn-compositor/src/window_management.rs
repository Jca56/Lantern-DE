/// Window lifecycle: focus, minimize, maximize, restore, cycle, alt-tab.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Serial},
};

use crate::state::{FullscreenWindow, Lantern, MaximizedWindow, MinimizedWindow};
use crate::window_ext::WindowExt;

/// Action to take in response to an SSD button click.
pub enum SsdClickAction {
    Close(WlSurface),
    ToggleMaximize(WlSurface),
    Minimize(WlSurface),
    Move(Window),
}

impl Lantern {
    pub fn track_window(&mut self, window: &Window) {
        let Some(surface) = window.get_wl_surface() else { return };
        if !self.window_spawn_order.contains(&surface) {
            self.window_spawn_order.push(surface.clone());
        }
        self.remember_window_surface(&surface);
    }

    pub fn place_new_window(&mut self, _window: &Window) -> Point<i32, Logical> {
        let Some(output_geo) = self
            .space
            .outputs()
            .next()
            .and_then(|output| self.space.output_geometry(output))
        else {
            return (0, 0).into();
        };

        // Place windows in the current canvas viewport
        let viewport_x = self.canvas.offset.0 as i32;
        let viewport_y = self.canvas.offset.1 as i32;

        let existing_count = self.space.elements().count() as i32;
        let cascade_step = 36;
        let max_offset_x = (output_geo.size.w / 4).max(cascade_step);
        let max_offset_y = (output_geo.size.h / 4).max(cascade_step);
        let offset_x = (existing_count * cascade_step) % max_offset_x;
        let offset_y = (existing_count * cascade_step) % max_offset_y;

        Point::from((viewport_x + offset_x, viewport_y + offset_y))
    }

    pub fn map_new_window(&mut self, window: Window) {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let Some(surface) = window.get_wl_surface() else { return };

        // Check if this window should be claimed as the scratchpad
        let is_scratchpad = self.scratchpad_pending;
        if is_scratchpad {
            self.scratchpad_pending = false;
            self.scratchpad_surface = Some(surface.clone());
            tracing::info!("Claimed new window as scratchpad");
        }

        let location = if is_scratchpad {
            self.scratchpad_geometry()
                .map(|geo| geo.loc)
                .unwrap_or_else(|| self.place_new_window(&window))
        } else {
            self.place_new_window(&window)
        };

        tracing::info!(
            x = location.x,
            y = location.y,
            mapped_windows = self.space.elements().count(),
            scratchpad = is_scratchpad,
            "Mapping new toplevel window"
        );

        self.space.map_element(window.clone(), location, true);
        self.track_window(&window);

        // Configure scratchpad size
        if is_scratchpad {
            if let Some(geo) = self.scratchpad_geometry() {
                window.configure_size(geo.size);
            }
        }

        // Start open animation
        self.animations.start_open(&surface);

        // Announce to foreign-toplevel clients
        let title = window.get_title();
        let app_id = window.get_app_id();
        self.foreign_toplevel_state.new_toplevel(&surface, &title, &app_id);

        self.focus_window(&window, serial);
    }

    pub fn forget_window(&mut self, surface: &WlSurface) {
        self.window_spawn_order.retain(|entry| entry != surface);
        self.window_mru.retain(|entry| entry != surface);
        self.minimized_windows.retain(|entry| entry.surface != *surface);
        self.maximized_windows.retain(|entry| entry.surface != *surface);
        self.fullscreen_windows.retain(|entry| entry.surface != *surface);
        self.snapped_windows.retain(|entry| entry.surface != *surface);
        self.window_opacity.remove(surface);
        self.animations.remove(surface);
        self.ssd.remove(surface);
        self.foreign_toplevel_state.toplevel_closed(surface);

        // Clear scratchpad if this was it
        if self.scratchpad_surface.as_ref() == Some(surface) {
            self.scratchpad_surface = None;
            tracing::info!("Scratchpad window closed, clearing reference");
        }
    }

    /// Start a close animation on the focused window (Super+Q).
    /// Returns true if the animation was started. The actual `send_close()`
    /// happens when the animation finishes in the render loop.
    pub fn close_focused_animated(&mut self) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };
        let Some(surface) = window.get_wl_surface() else { return false };
        if self.animations.start_close(&surface) {
            tracing::info!("Close animation started for focused window");
            self.schedule_render();
            true
        } else {
            false
        }
    }

    /// Called when a close animation finishes. Sends the actual close request.
    pub fn finish_close_animation(&mut self, surface: &WlSurface) {
        if let Some(window) = self.find_mapped_window(surface) {
            tracing::info!("Close animation finished, sending close");
            window.request_close();
        }
    }

    pub fn get_window_opacity(&self, surface: &WlSurface) -> f32 {
        self.window_opacity.get(surface).copied().unwrap_or(1.0)
    }

    pub fn adjust_window_opacity(&mut self, surface: &WlSurface, delta: f32) {
        let current = self.get_window_opacity(surface);
        let new = (current + delta).clamp(0.1, 1.0);
        if (new - 1.0).abs() < f32::EPSILON {
            self.window_opacity.remove(surface);
        } else {
            self.window_opacity.insert(surface.clone(), new);
        }
    }

    pub fn focus_window(&mut self, window: &Window, serial: Serial) {
        let Some(surface) = window.get_wl_surface() else { return };
        self.space.raise_element(window, true);
        self.set_focus_surface(Some(surface), serial);
    }

    pub fn clear_focus(&mut self, serial: Serial) {
        self.set_focus_surface(None, serial);
    }

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

    pub fn minimize_focused(&mut self, serial: Serial) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };

        let Some(surface) = window.get_wl_surface() else { return false };
        self.minimize_surface(&surface, serial)
    }

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

        let all_surfaces: Vec<_> = self.window_spawn_order
            .iter()
            .filter(|s| self.find_any_window(s).is_some())
            .cloned()
            .collect();

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

    pub fn focus_next_window(&mut self, serial: Serial) -> bool {
        self.compact_window_mru();

        // Use spawn order so the list is stable and includes minimized windows
        let all_surfaces: Vec<_> = self.window_spawn_order
            .iter()
            .filter(|s| self.find_any_window(s).is_some())
            .cloned()
            .collect();

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

    pub fn maximize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.maximize_surface(surface, Serial::from(0))
    }

    pub fn unmaximize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.unmaximize_surface(surface, Serial::from(0))
    }

    pub fn minimize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.minimize_surface(surface, Serial::from(0))
    }

    pub fn hide_alt_tab_switcher(&mut self) {
        self.alt_tab_switcher.hide();
        self.schedule_render();
    }

    /// Close a window from the switcher overlay (close button click).
    /// Removes it from the switcher and sends close to the toplevel.
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

    pub(crate) fn set_focus_surface(&mut self, surface: Option<WlSurface>, serial: Serial) {
        tracing::info!(
            focused = surface.is_some(),
            mapped_windows = self.space.elements().count(),
            mru_len = self.window_mru.len(),
            "Updating keyboard focus"
        );

        let previous_focus = self.focused_surface.clone();
        // Update focused_surface BEFORE broadcasting so foreign-toplevel clients
        // see the correct activated state.
        self.focused_surface = surface.clone();

        let windows: Vec<_> = self.space.elements().cloned().collect();
        for candidate in &windows {
            let Some(candidate_surface) = candidate.get_wl_surface() else { continue };
            let is_focused = surface.as_ref().is_some_and(|focused| {
                &candidate_surface == focused
            });
            let was_focused = previous_focus.as_ref().is_some_and(|focused| {
                &candidate_surface == focused
            });

            if is_focused != was_focused {
                candidate.set_activated(is_focused);
                candidate.send_pending_configure();
                // Update foreign-toplevel activated state
                self.update_foreign_toplevel_states(&candidate_surface);
            }
        }

        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, surface.clone(), serial);

        if let Some(surface) = surface {
            self.remember_window_surface(&surface);
        }

        self.schedule_client_render();
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

        let Some(location) = self.space.element_location(&window) else {
            tracing::warn!("maximize_surface: no element location");
            return false;
        };
        let restore = Rectangle::new(location, window.geometry().size);
        self.maximized_windows.push(MaximizedWindow {
            surface: surface.clone(),
            restore,
        });

        let Some(output_geo) = self.window_output_geometry(&window) else {
            tracing::warn!("maximize_surface: no output geometry");
            self.maximized_windows.retain(|entry| entry.surface != *surface);
            return false;
        };
        tracing::info!("maximize_surface: configuring to {:?}", output_geo);

        window.set_maximized(true);
        window.configure_size(output_geo.size);

        self.space.map_element(window.clone(), output_geo.loc, true);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        true
    }

    pub(crate) fn unmaximize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        let Some(restore) = self.take_maximized_restore(surface) else {
            return false;
        };

        let Some(window) = self.find_mapped_window(surface) else {
            self.maximized_windows.push(MaximizedWindow {
                surface: surface.clone(),
                restore,
            });
            return false;
        };

        window.set_maximized(false);
        window.configure_size(restore.size);

        self.space.map_element(window.clone(), restore.loc, true);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        true
    }

    pub(crate) fn minimize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        if self.minimized_windows.iter().any(|entry| entry.surface == *surface) {
            return false;
        }

        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };

        let location = self.space.element_location(&window).unwrap_or_default();
        self.minimized_windows.push(MinimizedWindow {
            surface: surface.clone(),
            window: window.clone(),
            location,
        });
        window.set_activated(false);
        window.send_pending_configure();
        self.space.unmap_elem(&window);

        // Clear focus BEFORE broadcasting foreign-toplevel state so the
        // minimized window is no longer marked as activated. We must do
        // this before update_foreign_toplevel_states because the window
        // is already unmapped and clear_focus can't reach it via space.
        if serial != Serial::from(0) {
            self.clear_focus(serial);
        } else {
            self.focused_surface = None;
        }

        self.update_foreign_toplevel_states(surface);
        self.schedule_client_render();
        true
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
                entry.window.configure_size(output_geo.size);
                output_geo.loc
            } else {
                entry.location
            }
        } else {
            entry.location
        };

        self.space.map_element(entry.window.clone(), location, true);
        self.update_foreign_toplevel_states(&entry.surface);
        Some(entry.window)
    }

    pub fn focused_window(&self) -> Option<Window> {
        self.window_mru
            .iter()
            .find_map(|surface| self.find_mapped_window(surface))
            .or_else(|| self.space.elements().last().cloned())
    }

    pub(crate) fn remember_window_surface(&mut self, surface: &WlSurface) {
        self.window_mru.retain(|entry| entry != surface);
        self.window_mru.insert(0, surface.clone());
    }

    pub(crate) fn compact_window_mru(&mut self) {
        let retained: Vec<_> = self
            .window_mru
            .iter()
            .filter(|surface| self.find_any_window(surface).is_some())
            .cloned()
            .collect();
        self.window_mru = retained;
    }

    pub(crate) fn find_any_window(&self, surface: &WlSurface) -> Option<Window> {
        self.find_mapped_window(surface).or_else(|| {
            self.minimized_windows
                .iter()
                .find(|entry| entry.surface == *surface)
                .map(|entry| entry.window.clone())
        })
    }

    pub fn find_mapped_window(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|window| window.get_wl_surface().as_ref() == Some(surface))
            .cloned()
    }

    pub fn is_maximized(&self, surface: &WlSurface) -> bool {
        self.maximized_windows
            .iter()
            .any(|entry| entry.surface == *surface)
    }

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

    // --- Fullscreen ---

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
        let Some(output_geo) = self.space.outputs().next()
            .and_then(|output| self.space.output_geometry(output))
        else {
            return false;
        };

        // Save restore geometry
        let location = self.space.element_location(&window).unwrap_or_default();
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
        });

        window.set_fullscreen(true);
        window.configure_size(output_geo.size);

        self.space.map_element(window.clone(), output_geo.loc, true);
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

        window.set_fullscreen(false);
        window.configure_size(restore.size);

        self.space.map_element(window.clone(), restore.loc, true);
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
    pub fn wine_fullscreen(&mut self, surface: &WlSurface, x11: &smithay::xwayland::X11Surface) {
        // Get the Wine frame offset from X11 geometry (typically y=-4 for shadow)
        let x11_geo = x11.geometry();

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
        let Some(output_geo) = self.space.outputs().next()
            .and_then(|output| self.space.output_geometry(output))
        else { return };

        // Configure window to be taller (output height + titlebar)
        let padded_size = smithay::utils::Size::from((
            output_geo.size.w,
            output_geo.size.h + titlebar_h,
        ));
        window.configure_size(padded_size);

        // Map shifted up so titlebar goes off-screen
        let adjusted_loc = Point::from((output_geo.loc.x, output_geo.loc.y - titlebar_h));
        self.space.map_element(window, adjusted_loc, true);

        tracing::info!(
            titlebar_h,
            "Wine fullscreen: shifted window up to hide titlebar"
        );
    }

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

    pub(crate) fn window_output_geometry(&self, _window: &Window) -> Option<Rectangle<i32, Logical>> {
        let geo = self.space
            .outputs()
            .next()
            .and_then(|output| self.space.output_geometry(output))?;

        // Maximize fills the current viewport in canvas-space
        let viewport_x = self.canvas.offset.0 as i32;
        let viewport_y = self.canvas.offset.1 as i32;

        let mut result = Rectangle::new(
            Point::from((viewport_x, viewport_y)),
            geo.size,
        );

        // Subtract exclusive zones from layer surfaces (e.g. panel)
        let (top_excl, bottom_excl, left_excl, right_excl) = self.exclusive_zone_offsets();
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
        let maximized_surfaces: Vec<_> = self.maximized_windows
            .iter()
            .map(|e| e.surface.clone())
            .collect();
        for surface in &maximized_surfaces {
            if let Some(window) = self.find_mapped_window(surface) {
                if let Some(geo) = self.window_output_geometry(&window) {
                    window.configure_size(geo.size);
                    self.space.map_element(window, geo.loc, false);
                }
            }
        }

        // Reconfigure snapped windows too
        let snapped: Vec<_> = self.snapped_windows
            .iter()
            .map(|e| (e.surface.clone(), e.zone))
            .collect();
        for (surface, zone) in &snapped {
            if let Some(target) = self.snap_zone_geometry(*zone) {
                if let Some(window) = self.find_mapped_window(&surface) {
                    window.configure_size(target.size);
                    self.space.map_element(window, target.loc, false);
                }
            }
        }

        self.schedule_render();
    }

    // ── SSD interaction ─────────────────────────────────────────────────

    /// Update SSD hover state based on pointer position (in canvas-space).
    /// Returns true if any hover state changed (needs re-render).
    pub fn ssd_update_hover(&mut self, canvas_pos: smithay::utils::Point<f64, smithay::utils::Logical>) -> bool {
        let mut changed = false;

        // Collect SSD surfaces first to avoid borrow conflict
        let ssd_surfaces: Vec<WlSurface> = self.ssd.windows.keys().cloned().collect();

        for surface in &ssd_surfaces {
            let window = match self.find_mapped_window(surface) {
                Some(w) => w,
                None => continue,
            };
            // Skip fullscreen windows (no SSD shown)
            if self.is_fullscreen(surface) {
                continue;
            }
            let win_loc = self.space.element_location(&window).unwrap_or_default();
            let win_size = window.geometry().size;

            let new_hover = match crate::ssd::hit_test(canvas_pos, win_loc, win_size) {
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

        changed
    }

    /// Handle a click on SSD decorations. Returns true if the click was consumed.
    /// `canvas_pos` is the pointer position in canvas-space.
    pub fn ssd_handle_click(
        &mut self,
        canvas_pos: smithay::utils::Point<f64, smithay::utils::Logical>,
        serial: smithay::utils::Serial,
    ) -> Option<SsdClickAction> {
        let ssd_surfaces: Vec<WlSurface> = self.ssd.windows.keys().cloned().collect();

        // Check front-to-back (space elements are front-first)
        for window in self.space.elements().cloned().collect::<Vec<_>>() {
            let Some(surface) = window.get_wl_surface() else { continue };
            if !ssd_surfaces.contains(&surface) {
                continue;
            }
            if self.is_fullscreen(&surface) {
                continue;
            }
            let win_loc = self.space.element_location(&window).unwrap_or_default();
            let win_size = window.geometry().size;

            match crate::ssd::hit_test(canvas_pos, win_loc, win_size) {
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

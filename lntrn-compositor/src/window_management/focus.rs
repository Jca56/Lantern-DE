//! Focus management: setting/clearing focus, MRU tracking, window lookup.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Serial,
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

impl Lantern {
    pub fn focus_window(&mut self, window: &Window, serial: Serial) {
        let Some(surface) = window.get_wl_surface() else { return };
        self.space.raise_element(window, true);
        self.set_focus_surface(Some(surface), serial);
    }

    pub fn clear_focus(&mut self, serial: Serial) {
        self.set_focus_surface(None, serial);
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
}

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
        // Raise in BOTH the global self.space AND the window's per-workspace
        // Space so all Z-order consumers agree. Why both:
        //   - Rendering iterates the per-workspace Space's elements in
        //     bottom-to-top order — raising here is what makes the window
        //     visually appear on top.
        //   - Click hit-tests (`visible_element_under`) also use the
        //     per-workspace Space.
        //   - Pointer motion hit-tests (`surface_under`) use self.space.
        // Without raising in both, the renderer would draw window-A on top
        // while motion/hover events would still fall through to window-B
        // (whichever was last raised in self.space).
        self.space.raise_element(window, true);
        let workspace_loc = self.workspaces.window_workspace(&surface);
        if let Some((output_name, ws_id)) = workspace_loc {
            if let Some(space) = self.workspace_space_mut(&output_name, ws_id) {
                space.raise_element(window, true);
            }
        }
        // Keep XWayland's stacking order in sync with the compositor. Without
        // this, X11 clients with multi-window UIs (Steam, Wine apps) deliver
        // pointer events through whichever window happens to be top of the X
        // server's internal stack — which can be a stale, invisible one — so
        // clicks on the visible window are silently dropped.
        if let Some(x11) = window.x11_surface().cloned() {
            if let Some(xwm) = self.xwayland_state.wm.as_mut() {
                if let Err(err) = xwm.raise_window(&x11) {
                    tracing::warn!("X11Wm::raise_window failed: {err}");
                }
            }
        }
        self.set_focus_surface(Some(surface), serial);
    }

    pub fn clear_focus(&mut self, serial: Serial) {
        self.set_focus_surface(None, serial);
    }

    /// Re-evaluate pointer focus at the cursor's CURRENT location and deliver a
    /// synthetic `wl_pointer.motion`. Setting keyboard focus alone is not enough
    /// for click-to-interact: `wl_pointer.enter` only fires on real motion, so a
    /// window that maps/fullscreens UNDER a stationary cursor never gets pointer
    /// focus and silently drops clicks. Proton games (Helldivers, Cyberpunk) hit
    /// this: they fullscreen their real window while the cursor sits still over
    /// it, so without this nudge they stay unclickable until you wiggle the mouse
    /// or alt-tab. Mirrors the OR-window motion synthesis in handlers/xwayland.rs.
    pub fn refocus_pointer_at_cursor(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let pos = pointer.current_location();
        let under = self.surface_under(pos);
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location: pos,
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time: 0,
            },
        );
        pointer.frame(self);
    }

    pub(crate) fn set_focus_surface(&mut self, surface: Option<WlSurface>, serial: Serial) {
        // While the session is locked, the lock surface owns keyboard focus.
        // Refuse every other focus change so typed passwords reach the locker.
        if self.is_locked() {
            return;
        }
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

        // Build the keyboard focus target. For X11 surfaces we use the
        // X11 variant so Smithay's KeyboardTarget impl also invokes the
        // X11 server's SetInputFocus — native X11 games (SDL2 / XInput2 /
        // XKBlib) won't receive key events without it.
        let focus_target = surface.as_ref().map(|wls| {
            let x11 = self
                .space
                .elements()
                .find(|w| w.get_wl_surface().as_ref() == Some(wls))
                .and_then(|w| w.x11_surface().cloned());
            match x11 {
                Some(x11s) => crate::keyboard_focus::KeyboardFocusTarget::X11(x11s),
                None => crate::keyboard_focus::KeyboardFocusTarget::Wayland(wls.clone()),
            }
        });

        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, focus_target, serial);

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
        // Look across every per-workspace Space first — windows on hidden
        // workspaces still need to be findable for focus/animation/state
        // tracking. Fall back to the legacy global Space so scratchpad and
        // other un-workspaced windows are still discoverable.
        if let Some(w) = self.find_window_anywhere(surface) {
            return Some(w);
        }
        self.space
            .elements()
            .find(|window| window.get_wl_surface().as_ref() == Some(surface))
            .cloned()
    }
}

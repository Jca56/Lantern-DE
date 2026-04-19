/// XWayland handler: X11 window lifecycle, clipboard sync, override-redirect.

use smithay::{
    delegate_xwayland_shell,
    desktop::Window,
    input::pointer::{Focus, GrabStartData as PointerGrabStartData},
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::xwayland_shell::XWaylandShellHandler,
    wayland::selection::SelectionTarget,
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmHandler, XwmId},
        X11Surface, X11Wm,
    },
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::os::fd::OwnedFd;

use crate::{
    grabs::{resize_grab::ResizeEdge, MoveSurfaceGrab, ResizeSurfaceGrab},
    window_ext::WindowExt,
    Lantern,
};

impl XwmHandler for Lantern {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwayland_state.wm.as_mut().expect("X11Wm not initialized")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(
            class = window.class(),
            title = window.title(),
            "New X11 window"
        );
        // Nothing to do yet — wait for map_window_request
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(
            class = window.class(),
            "New X11 override-redirect window"
        );
        // Nothing to do — wait for mapped_override_redirect_window
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        // Skip windows that have already been destroyed: Proton/Wine can race
        // map ↔ destroy during startup, and reparenting a dead window spams
        // X11 ReparentWindow errors that propagate into the game's Xlib.
        if !window.alive() {
            tracing::debug!(class = window.class(), "Ignoring X11 map request for dead window");
            return;
        }

        tracing::info!(
            class = window.class(),
            title = window.title(),
            "X11 map request"
        );

        // Grant the mapping
        if let Err(err) = window.set_mapped(true) {
            tracing::error!("Failed to set X11 window mapped: {}", err);
            return;
        }

        // Configure to the requested geometry
        let geo = window.geometry();
        if let Err(err) = window.configure(geo) {
            tracing::warn!("Failed to configure X11 window: {}", err);
        }

        // Wrap in Smithay's Window type
        let win = Window::new_x11_window(window.clone());

        if should_add_ssd(&window) {
            if let Some(wl_surface) = window.wl_surface() {
                self.ssd.add(wl_surface);
            }
        }

        // The Wayland surface may not be associated yet (race condition).
        // If get_wl_surface() returns None, defer mapping until surface_associated().
        if win.get_wl_surface().is_some() {
            self.map_new_window(win);
        } else {
            tracing::info!(
                class = window.class(),
                "X11 window pending surface association"
            );
            self.pending_x11_windows.push(win);
        }
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let geo = window.geometry();
        tracing::info!(
            class = window.class(),
            x = geo.loc.x,
            y = geo.loc.y,
            w = geo.size.w,
            h = geo.size.h,
            "Override-redirect window mapped"
        );

        let win = Window::new_x11_window(window);
        // OR windows position themselves — map at their requested location
        self.space.map_element(win.clone(), geo.loc, false);
        self.override_redirect_windows.push(win);
        self.schedule_render();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(class = window.class(), "X11 window unmapped");

        // Remove from pending list if it never got mapped
        self.pending_x11_windows.retain(|w| {
            w.x11_surface().map(|x| x != &window).unwrap_or(true)
        });

        // Check override-redirect list first
        if let Some(idx) = self.override_redirect_windows.iter().position(|w| {
            w.x11_surface().map(|x| x == &window).unwrap_or(false)
        }) {
            let win = self.override_redirect_windows.remove(idx);
            self.space.unmap_elem(&win);
            self.schedule_render();
            return;
        }

        // Regular managed window
        if let Some(wl_surface) = window.wl_surface() {
            if let Some(win) = self.find_mapped_window(&wl_surface) {
                self.space.unmap_elem(&win);
            }
            self.forget_window(&wl_surface);
        }

        self.schedule_render();
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(class = window.class(), "X11 window destroyed");

        // Remove from pending list if it never got mapped
        self.pending_x11_windows.retain(|w| {
            w.x11_surface().map(|x| x != &window).unwrap_or(true)
        });

        // Clean up override-redirect
        if let Some(idx) = self.override_redirect_windows.iter().position(|w| {
            w.x11_surface().map(|x| x == &window).unwrap_or(false)
        }) {
            let win = self.override_redirect_windows.remove(idx);
            self.space.unmap_elem(&win);
        }

        // Clean up managed windows
        if let Some(wl_surface) = window.wl_surface() {
            if let Some(win) = self.find_mapped_window(&wl_surface) {
                self.space.unmap_elem(&win);
            }
            self.forget_window(&wl_surface);
        }

        self.schedule_render();
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // Configuring a dead window triggers X11 errors that propagate into
        // the game's Xlib and can abort Proton/Wine.
        if !window.alive() {
            return;
        }
        // Don't let clients resize/reposition themselves out of a
        // compositor-managed state (maximized or fullscreen).
        if let Some(wl_surface) = window.wl_surface() {
            if self.is_maximized(&wl_surface) || self.is_fullscreen(&wl_surface) {
                tracing::debug!(
                    class = window.class(),
                    "Ignoring X11 configure request: window is in managed state"
                );
                return;
            }
        }

        let mut geo = window.geometry();
        if let Some(x) = x {
            geo.loc.x = x;
        }
        if let Some(y) = y {
            geo.loc.y = y;
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        tracing::debug!(
            class = window.class(),
            x = geo.loc.x, y = geo.loc.y,
            w = geo.size.w, h = geo.size.h,
            "X11 configure request"
        );
        let _ = window.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
        // Update override-redirect window positions in the space
        if window.is_override_redirect() {
            if let Some(win) = self.override_redirect_windows.iter().find(|w| {
                w.x11_surface().map(|x| x == &window).unwrap_or(false)
            }) {
                self.space.map_element(win.clone(), geometry.loc, false);
                self.schedule_render();
            }
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(class = window.class(), "X11 maximize request");
        if let Some(wl_surface) = window.wl_surface() {
            self.maximize_request_surface(&wl_surface);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(class = window.class(), "X11 unmaximize request");
        if let Some(wl_surface) = window.wl_surface() {
            self.unmaximize_request_surface(&wl_surface);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if !window.alive() {
            tracing::debug!("Ignoring X11 fullscreen request for dead window");
            return;
        }
        let geo = window.geometry();
        tracing::info!(
            class = window.class(),
            title = window.title(),
            x = geo.loc.x, y = geo.loc.y,
            w = geo.size.w, h = geo.size.h,
            decorated = window.is_decorated(),
            "X11 fullscreen request"
        );
        if let Some(wl_surface) = window.wl_surface() {
            if is_wine_window(&window) {
                // Wine draws its own titlebar inside the window surface.
                // Tell Wine to use the full output size + titlebar, then map
                // shifted up so the titlebar goes off-screen.
                self.wine_fullscreen(&wl_surface, &window);
            } else {
                self.fullscreen_request_surface(&wl_surface);
            }
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if !window.alive() {
            tracing::debug!("Ignoring X11 unfullscreen request for dead window");
            return;
        }
        let geo = window.geometry();
        tracing::info!(
            class = window.class(),
            title = window.title(),
            x = geo.loc.x, y = geo.loc.y,
            w = geo.size.w, h = geo.size.h,
            decorated = window.is_decorated(),
            "X11 unfullscreen request"
        );
        if let Some(wl_surface) = window.wl_surface() {
            self.unfullscreen_request_surface(&wl_surface);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!(class = window.class(), "X11 minimize request");
        if let Some(wl_surface) = window.wl_surface() {
            self.minimize_request_surface(&wl_surface);
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        button: u32,
        resize_edge: X11ResizeEdge,
    ) {
        let Some(wl_surface) = window.wl_surface() else { return };
        let Some(win) = self.find_mapped_window(&wl_surface) else { return };

        let seat = self.seat.clone();
        let pointer = match seat.get_pointer() {
            Some(p) => p,
            None => return,
        };

        let start_data = PointerGrabStartData {
            focus: self.surface_under(pointer.current_location())
                .map(|(s, loc)| (s, loc.to_i32_round())),
            button,
            location: pointer.current_location(),
        };

        let initial_window_location = self.space.element_location(&win).unwrap_or_default();
        let initial_window_size = win.geometry().size;

        let edges = x11_resize_edge_to_ours(resize_edge);

        let grab = ResizeSurfaceGrab::start(
            start_data,
            win,
            edges,
            Rectangle::new(initial_window_location, initial_window_size),
        );

        let serial = SERIAL_COUNTER.next_serial();
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        let Some(wl_surface) = window.wl_surface() else { return };
        let Some(win) = self.find_mapped_window(&wl_surface) else { return };

        let seat = self.seat.clone();
        let pointer = match seat.get_pointer() {
            Some(p) => p,
            None => return,
        };

        let start_data = PointerGrabStartData {
            focus: self.surface_under(pointer.current_location())
                .map(|(s, loc)| (s, loc.to_i32_round())),
            button,
            location: pointer.current_location(),
        };

        let initial_window_location = self.space.element_location(&win).unwrap_or_default();
        let was_snapped = self.is_snapped(&wl_surface);
        let was_maximized = self.is_maximized(&wl_surface);
        let was_tiled = self.workspaces.contains(&wl_surface);

        let grab = MoveSurfaceGrab {
            start_data,
            window: win,
            initial_window_location,
            was_snapped,
            was_maximized,
            was_tiled,
            restored_this_drag: false,
            has_moved: false,
        };

        let serial = SERIAL_COUNTER.next_serial();
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    // --- Clipboard / Selection ---

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        // Allow X11 clients to access Wayland clipboard
        true
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        _selection: SelectionTarget,
        _mime_type: String,
        _fd: OwnedFd,
    ) {
        // TODO: Read from Wayland clipboard and write to fd
        // This requires reading from DataDeviceState's current selection
        tracing::debug!("X11 selection send requested (not yet implemented)");
    }

    fn new_selection(
        &mut self,
        _xwm: XwmId,
        _selection: SelectionTarget,
        _mime_types: Vec<String>,
    ) {
        // TODO: Set X11 selection as Wayland clipboard source
        tracing::debug!("X11 new selection (not yet implemented)");
    }

    fn cleared_selection(&mut self, _xwm: XwmId, _selection: SelectionTarget) {
        tracing::debug!("X11 selection cleared");
    }
}

impl XWaylandShellHandler for Lantern {
    fn xwayland_shell_state(&mut self) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm_id: XwmId,
        wl_surface: WlSurface,
        surface: X11Surface,
    ) {
        tracing::info!(
            class = surface.class(),
            "X11 surface associated with Wayland surface"
        );

        // If the window is already mapped in the space, add SSD now
        // (it may have been mapped before the surface was associated)
        if self.find_mapped_window(&wl_surface).is_some() {
            if should_add_ssd(&surface) {
                self.ssd.add(wl_surface);
            }
            return;
        }

        // Check if this window was waiting for its surface association
        if let Some(idx) = self.pending_x11_windows.iter().position(|w| {
            w.x11_surface().map(|x| x == &surface).unwrap_or(false)
        }) {
            let win = self.pending_x11_windows.remove(idx);
            tracing::info!(
                class = surface.class(),
                "Mapping deferred X11 window"
            );
            if should_add_ssd(&surface) {
                self.ssd.add(wl_surface);
            }
            self.map_new_window(win);
        }
    }
}

delegate_xwayland_shell!(Lantern);

/// Check if an X11 window should get server-side decorations.
/// Respects motif hints (is_decorated) from the client.
/// Wine windows are always skipped — Wine manages its own decorations.
fn should_add_ssd(window: &X11Surface) -> bool {
    if window.is_override_redirect() {
        return false;
    }
    if is_wine_window(window) {
        return false;
    }
    !window.is_decorated()
}

/// Detect Wine windows by class name (typically ends in .exe).
fn is_wine_window(window: &X11Surface) -> bool {
    let class = window.class().to_lowercase();
    class.ends_with(".exe") || class.contains("wine")
}

/// Convert Smithay's X11 ResizeEdge to our internal ResizeEdge bitflags.
fn x11_resize_edge_to_ours(edge: X11ResizeEdge) -> ResizeEdge {
    let mut result = ResizeEdge::empty();
    match edge {
        X11ResizeEdge::Top => result |= ResizeEdge::TOP,
        X11ResizeEdge::Bottom => result |= ResizeEdge::BOTTOM,
        X11ResizeEdge::Left => result |= ResizeEdge::LEFT,
        X11ResizeEdge::Right => result |= ResizeEdge::RIGHT,
        X11ResizeEdge::TopLeft => result |= ResizeEdge::TOP | ResizeEdge::LEFT,
        X11ResizeEdge::TopRight => result |= ResizeEdge::TOP | ResizeEdge::RIGHT,
        X11ResizeEdge::BottomLeft => result |= ResizeEdge::BOTTOM | ResizeEdge::LEFT,
        X11ResizeEdge::BottomRight => result |= ResizeEdge::BOTTOM | ResizeEdge::RIGHT,
    }
    result
}

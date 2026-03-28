/// Unified window operations for both Wayland (XDG) and X11 windows.
///
/// Smithay's `Window` wraps either a `ToplevelSurface` or an `X11Surface`.
/// Most compositor code used `.toplevel().unwrap()` which panics for X11.
/// This trait provides safe accessors that dispatch on the underlying type.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};
use smithay::wayland::seat::WaylandFocus;

pub trait WindowExt {
    /// Get the WlSurface (works for both Wayland and X11).
    /// Returns None only if an X11 surface isn't yet associated.
    fn get_wl_surface(&self) -> Option<WlSurface>;

    /// Window title (XDG: from XdgToplevelSurfaceData, X11: WM_NAME).
    fn get_title(&self) -> String;

    /// App identifier (XDG: app_id, X11: WM_CLASS).
    fn get_app_id(&self) -> String;

    /// Request the window to close.
    fn request_close(&self);

    /// Configure the window's size.
    /// Wayland: sets pending state + sends configure.
    /// X11: sends X11 ConfigureWindow.
    fn configure_size(&self, size: Size<i32, Logical>);

    /// Flush any pending configure to the client. No-op for X11.
    fn send_pending_configure(&self);

    /// Set the maximized state on the window.
    fn set_maximized(&self, maximized: bool);

    /// Set the fullscreen state on the window.
    fn set_fullscreen(&self, fullscreen: bool);
}

impl WindowExt for Window {
    fn get_wl_surface(&self) -> Option<WlSurface> {
        WaylandFocus::wl_surface(self).map(|cow| cow.into_owned())
    }

    fn get_title(&self) -> String {
        if let Some(toplevel) = self.toplevel() {
            smithay::wayland::compositor::with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok())
                    .and_then(|attrs| attrs.title.clone())
                    .unwrap_or_default()
            })
        } else if let Some(x11) = self.x11_surface() {
            x11.title()
        } else {
            String::new()
        }
    }

    fn get_app_id(&self) -> String {
        if let Some(toplevel) = self.toplevel() {
            smithay::wayland::compositor::with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok())
                    .and_then(|attrs| attrs.app_id.clone())
                    .unwrap_or_default()
            })
        } else if let Some(x11) = self.x11_surface() {
            x11.class()
        } else {
            String::new()
        }
    }

    fn request_close(&self) {
        if let Some(toplevel) = self.toplevel() {
            toplevel.send_close();
        } else if let Some(x11) = self.x11_surface() {
            let _ = x11.close();
        }
    }

    fn configure_size(&self, size: Size<i32, Logical>) {
        if let Some(toplevel) = self.toplevel() {
            toplevel.with_pending_state(|state| {
                state.size = Some(size);
            });
            toplevel.send_pending_configure();
        } else if let Some(x11) = self.x11_surface() {
            let geo = x11.geometry();
            let rect = smithay::utils::Rectangle::new(geo.loc, size);
            let _ = x11.configure(rect);
        }
    }

    fn send_pending_configure(&self) {
        if let Some(toplevel) = self.toplevel() {
            toplevel.send_pending_configure();
        }
        // X11 configures are immediate — no-op
    }

    fn set_maximized(&self, maximized: bool) {
        if let Some(toplevel) = self.toplevel() {
            toplevel.with_pending_state(|state| {
                if maximized {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                }
            });
        } else if let Some(x11) = self.x11_surface() {
            let _ = x11.set_maximized(maximized);
        }
    }

    fn set_fullscreen(&self, fullscreen: bool) {
        if let Some(toplevel) = self.toplevel() {
            toplevel.with_pending_state(|state| {
                if fullscreen {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
                    );
                }
            });
        } else if let Some(x11) = self.x11_surface() {
            let _ = x11.set_fullscreen(fullscreen);
        }
    }
}

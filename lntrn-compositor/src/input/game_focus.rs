//! Games and the keyboard: a game lets go of the mouse when it loses the
//! keyboard (Alt+Tab, a click on another monitor) and takes it back when it
//! gets the keyboard again.
//!
//! ── Pointer constraints follow the keyboard ──
//! A game's pointer lock (mouse-look) or confine holds only while the window
//! it belongs to has the keyboard. Before, a lock held whenever the cursor sat
//! over the game: Alt+Tab moved the keyboard away, but the mouse stayed
//! trapped on the game's monitor. KDE and sway gate constraints the same way.
//!
//! ── The focusing click on a fullscreen window is eaten ──
//! Clicking an unfocused fullscreen window (the game you Alt+Tabbed out of)
//! gives it the keyboard, and with it the lock, but the click itself never
//! reaches it: in a game that click is an attack or a menu pick.

use std::borrow::Cow;

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        compositor::get_parent, pointer_constraints::with_pointer_constraint,
        seat::WaylandFocus,
    },
    xwayland::X11Surface,
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

/// The surface a subsurface hangs off (the surface itself for a toplevel).
fn root_surface(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

impl Lantern {
    // ── Pointer constraints follow the keyboard ──

    /// The keyboard focus's surface. Never call it from inside a keyboard
    /// `set_focus` (`SeatHandler::focus_changed` runs while the keyboard is
    /// locked): it would deadlock. That path queues [`Self::queue_pointer_constraint_sync`].
    pub(crate) fn keyboard_focus_surface(&self) -> Option<WlSurface> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        focus.wl_surface().map(Cow::into_owned)
    }

    /// May a pointer constraint on `surface` hold while `focus` has the
    /// keyboard? Only when both belong to the same window. Call it outside
    /// `with_pointer_constraint`: that holds the surface's state lock, and
    /// walking to the root surface takes it again.
    pub(crate) fn pointer_constraint_allowed(
        &self,
        surface: &WlSurface,
        focus: Option<&WlSurface>,
    ) -> bool {
        let Some(focus) = focus else {
            return false;
        };
        if surface == focus {
            return true;
        }
        let root = root_surface(surface);
        if root == root_surface(focus) {
            return true;
        }
        // Every X11 window is the one XWayland client, so tell games apart by
        // process: a game's helper window (override-redirect) may hold the
        // lock while its main window has the keyboard.
        match (self.x11_surface_of(&root), self.x11_surface_of(focus)) {
            (Some(a), Some(b)) => a.pid().is_some() && a.pid() == b.pid(),
            _ => false,
        }
    }

    fn x11_surface_of(&self, surface: &WlSurface) -> Option<X11Surface> {
        self.space
            .elements()
            .chain(self.override_redirect_windows.iter())
            .filter_map(|w| w.x11_surface())
            .find(|x11| x11.wl_surface().as_ref() == Some(surface))
            .cloned()
    }

    /// Re-check the pointer constraint once the event loop is idle. Keyboard
    /// focus changes call this: they can happen while the keyboard is locked
    /// (`focus_changed`) or inside a pointer grab (a window drag handing the
    /// window to another monitor), where touching either handle deadlocks.
    pub(crate) fn queue_pointer_constraint_sync(&mut self) {
        if self.constraint_sync_queued {
            return;
        }
        self.constraint_sync_queued = true;
        self.loop_handle.insert_idle(|state: &mut Lantern| {
            state.constraint_sync_queued = false;
            state.sync_pointer_constraint();
        });
    }

    /// Bring the pointer constraint under the cursor in line with the
    /// keyboard: let go when its window lost the keyboard, engage when it has
    /// it and the cursor sits inside the constraint's region. Proton games
    /// request their lock mid-fullscreen-dance, before focus lands, so the
    /// engage half matters as much as the release.
    pub fn sync_pointer_constraint(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let focus = self.keyboard_focus_surface();
        let pos = pointer.current_location();
        let under = self.surface_under(pos);

        // An active constraint lives on the pointer-focused surface; release
        // it there, and on the surface under the cursor if that differs.
        let mut candidates: Vec<WlSurface> = pointer.current_focus().into_iter().collect();
        if let Some((surface, _)) = under.as_ref() {
            if !candidates.contains(surface) {
                candidates.push(surface.clone());
            }
        }
        for surface in &candidates {
            if !self.pointer_constraint_allowed(surface, focus.as_ref()) {
                with_pointer_constraint(surface, &pointer, |constraint| {
                    if let Some(constraint) = constraint.filter(|c| c.is_active()) {
                        constraint.deactivate();
                    }
                });
            }
        }

        let Some((surface, surface_loc)) = under else {
            return;
        };
        if !self.pointer_constraint_allowed(&surface, focus.as_ref()) {
            return;
        }
        with_pointer_constraint(&surface, &pointer, |constraint| {
            let Some(constraint) = constraint else { return };
            if constraint.is_active() {
                return;
            }
            // Region is surface-local, matching the motion-path check.
            let point = (pos - surface_loc).to_i32_round();
            if constraint.region().map_or(true, |r| r.contains(point)) {
                constraint.activate();
            }
        });
    }

    // ── The focusing click on a fullscreen window is eaten ──

    /// A press on `window` should only focus it: it's fullscreen and doesn't
    /// have the keyboard yet. Ask before focusing it.
    pub(crate) fn is_focusing_click_on_fullscreen(&self, window: &Window) -> bool {
        let Some(surface) = window.get_wl_surface() else {
            return false;
        };
        self.is_fullscreen(&surface) && self.keyboard_focus_surface().as_ref() != Some(&surface)
    }

    /// Swallow the release of a press that was eaten as a focusing click.
    /// True when `button` is that release: it must not reach the client.
    pub(crate) fn take_eaten_release(&mut self, button: u32) -> bool {
        if self.eaten_click_button == Some(button) {
            self.eaten_click_button = None;
            return true;
        }
        false
    }
}

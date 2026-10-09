//! Modal overlays: Overlay-layer surfaces holding Exclusive keyboard
//! interactivity — the screenshot selection UI, the screen recorder's region
//! picker, the Command Center while it's open.
//!
//! A fullscreen window covers every other layer surface on its output: the
//! bar, notifications and the OSD must not draw over a game or a video. A
//! modal overlay is the exception. The user summoned it and it already owns
//! the keyboard, so it is drawn above the fullscreen window, takes the
//! pointer there (out of a game's pointer lock, if need be), and keeps the
//! output composited rather than on direct scanout.

use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        shell::wlr_layer::{KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceCachedState},
    },
};

use crate::state::Lantern;

/// True if a layer surface with this state is a modal overlay.
pub(crate) fn is_modal(cached: &LayerSurfaceCachedState) -> bool {
    cached.layer == Layer::Overlay
        && cached.keyboard_interactivity == KeyboardInteractivity::Exclusive
}

fn layer_is_modal(ls: &LayerSurface) -> bool {
    ls.alive()
        && with_states(ls.wl_surface(), |states| {
            let cached = *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current();
            is_modal(&cached)
        })
}

impl Lantern {
    /// True while any modal overlay is up. Hot corners must not fire
    /// underneath one: dragging a screenshot selection into a corner would
    /// otherwise pop the window switcher or show-desktop mid-capture.
    pub(crate) fn modal_overlay_active(&self) -> bool {
        self.layer_surfaces.iter().any(layer_is_modal)
    }

    /// True while a modal overlay is up on `output`.
    pub(crate) fn output_has_modal_overlay(&self, output: &Output) -> bool {
        self.layer_surfaces
            .iter()
            .any(|ls| layer_is_modal(ls) && self.layer_surface_on_output(ls, output))
    }

    /// Hand pointer focus to whatever is under the cursor now, if something
    /// else holds it. Pointer focus otherwise only follows real motion, and a
    /// pointer-locked game never produces any: a modal overlay that maps over
    /// it would stay unreachable, and the game would not get its lock back
    /// once the overlay is gone. Moving focus off the game releases its lock
    /// (smithay deactivates the constraint on leave) and brings the cursor
    /// back; moving it onto the game re-engages the lock.
    pub(crate) fn sync_pointer_focus(&mut self) {
        // The lock screen and the window switcher keep the pointer away from
        // clients on purpose.
        if self.is_locked() || self.alt_tab_switcher.is_visible() {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // A held button pins focus to the surface it went down on; the next
        // call after its release picks the change up.
        if pointer.is_grabbed() {
            return;
        }
        let under = self.surface_under(pointer.current_location());
        if under.as_ref().map(|(surface, _)| surface) == pointer.current_focus().as_ref() {
            return;
        }
        // The motion path looks up pointer constraints on this cached hit; a
        // stale one would re-lock the pointer to the surface it just left.
        self.last_pointer_under = None;
        self.refocus_pointer_at_cursor();
    }

    /// A layer surface is going away. If it held the keyboard (a modal
    /// overlay closing), hand focus back to the top-most window — as the
    /// commit handler does when a layer surface drops its grab — so the
    /// window underneath, a game especially, is live again without a click.
    pub(crate) fn release_layer_focus(&mut self, surface: &WlSurface) {
        // While locked the lock surface owns the keyboard; leave it be.
        if self.is_locked() {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let had_focus = keyboard
            .current_focus()
            .is_some_and(|focus| focus.wl_surface().as_deref() == Some(surface));
        if had_focus {
            if let Some(window) = self.focused_window() {
                self.focus_window(&window, SERIAL_COUNTER.next_serial());
            }
        }
        self.sync_pointer_focus();
    }
}

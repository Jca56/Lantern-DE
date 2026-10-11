//! wp_fractional_scale: telling each surface the scale of the output it is
//! on.
//!
//! Smithay sends `preferred_scale` only when it is set, and sets it nowhere
//! itself, so every point where a surface gets or changes its output has to
//! say so. With one output there is one answer. With a 4K monitor at 1.3
//! beside a 1080p one at 1.0 a surface that is told the wrong one renders a
//! buffer for the other monitor's density and is resampled onto this one:
//! soft text on the 1080p monitor, a blurry upscale on the 4K one.

use smithay::delegate_fractional_scale;
use smithay::desktop::{find_popup_root_surface, Window};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{get_parent, with_states};
use smithay::wayland::fractional_scale::{with_fractional_scale, FractionalScaleHandler};

use crate::window_ext::WindowExt;
use crate::Lantern;

fn set_preferred(surface: &WlSurface, scale: f64) {
    // Dedupes, and is a no-op for a surface whose client never bound the
    // protocol, so it is safe to call on everything.
    with_states(surface, |states| {
        with_fractional_scale(states, |f| f.set_preferred_scale(scale));
    });
}

impl Lantern {
    /// The output a surface's content is shown at the scale of: the output
    /// that owns its window (or holds its layer surface), found through the
    /// subsurface and popup chain.
    fn scale_output_for(&self, surface: &WlSurface) -> Option<Output> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(popup) = self.popups.find_popup(&root) {
            if let Ok(parent) = find_popup_root_surface(&popup) {
                root = parent;
            }
        }
        self.owner_output(&root)
            .or_else(|| self.layer_surface_outputs.get(&root).cloned())
    }

    /// Tell a window's whole surface tree, popups included, the scale of the
    /// output that owns it. Call when a window is mapped and whenever it
    /// changes owner.
    pub(crate) fn send_window_scale(&self, window: &Window) {
        let Some(surface) = window.get_wl_surface() else {
            return;
        };
        let Some(scale) = self
            .owner_output(&surface)
            .or_else(|| self.output_for_window(window))
            .map(|o| o.current_scale().fractional_scale())
        else {
            return;
        };
        // Subsurfaces and popups included. `with_surfaces` hands out each
        // surface's state already locked: set through that, never through
        // `with_states` again.
        window.with_surfaces(|_, states| {
            with_fractional_scale(states, |f| f.set_preferred_scale(scale));
        });
    }
}

impl FractionalScaleHandler for Lantern {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // A surface that already belongs somewhere takes that output's
        // scale. A brand new toplevel belongs nowhere yet: it will open on
        // the output under the pointer, so that is the best first answer
        // (it used to be the primary's, whichever monitor the window opened
        // on). `send_window_scale` corrects it when the window is mapped.
        let output = self
            .scale_output_for(&surface)
            .or_else(|| {
                let pos = self.seat.get_pointer()?.current_location();
                self.output_at_point(pos)
            })
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let scale = output
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or_else(crate::output_scale);
        set_preferred(&surface, scale);
    }
}

delegate_fractional_scale!(Lantern);

/// Push the current per-output fractional scale to every mapped surface.
/// Live scale changes (Settings slider, Gaming Mode) and layout changes
/// need this sweep; a window changing output on its own is handled where
/// that happens (`send_window_scale`).
pub fn refresh_fractional_scales(state: &Lantern) {
    let fallback = state
        .workspaces
        .outputs_iter()
        .next()
        .map(|o| o.current_scale().fractional_scale())
        .unwrap_or_else(crate::output_scale);
    // The global Space holds every mapped window (all workspaces, active or
    // not, plus scratchpad and override-redirect).
    for window in state.space.elements() {
        state.send_window_scale(window);
    }
    // Minimized windows live nowhere; they remember their output.
    for entry in &state.minimized_windows {
        let scale = state
            .workspaces
            .output_by_name(&entry.output)
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(fallback);
        entry.window.with_surfaces(|_, states| {
            with_fractional_scale(states, |f| f.set_preferred_scale(scale));
        });
    }
    for ls in &state.layer_surfaces {
        let scale = state
            .layer_surface_outputs
            .get(ls.wl_surface())
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(fallback);
        set_preferred(ls.wl_surface(), scale);
    }
}

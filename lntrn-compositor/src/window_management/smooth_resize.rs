//! Dispatcher for window-state resize animations. Trusted Lantern clients
//! get the smooth-scale path (`animate_smooth`) — compositor stretches the
//! existing committed buffer to fit the interpolated rect throughout the
//! animation, then sends a single configure at the end. Untrusted clients
//! (Wine, generic Wayland) keep the existing crossfade path.
//!
//! See `docs/native-smooth-resize-plan.md`.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

impl Lantern {
    /// Animate a window from `src` to `dst` (both compositor-logical
    /// coords). For trusted surfaces uses the smooth-scale path; for
    /// untrusted surfaces falls back to the existing immediate-configure
    /// + crossfade path.
    ///
    /// Callers that previously did `configure_rect(dst) + remap +
    /// animate_default(src, dst)` should call this instead — it handles
    /// the configure/remap/animate trio uniformly for both paths.
    pub fn animate_resize(
        &mut self,
        surface: &WlSurface,
        window: &Window,
        src: Rectangle<i32, Logical>,
        dst: Rectangle<i32, Logical>,
    ) {
        let trusted = crate::security::is_trusted_surface(surface);
        tracing::info!(
            trusted,
            src_size = ?src.size,
            dst_size = ?dst.size,
            "animate_resize"
        );
        if trusted {
            // Smooth path: scale the existing buffer throughout the
            // animation; defer the configure until anim end. Remap to the
            // new location so positioning is current. The remap's output
            // handoff measures against the DESTINATION size — the live
            // geometry is still the old (e.g. maximized) size until the
            // client acks, and its center can land on the wrong monitor.
            self.remap_tracked_window_sized(window.clone(), dst.loc, dst.size, true);
            self.window_state_anim.animate_smooth(surface, src, dst);
        } else {
            // Crossfade path: configure the client to the new size
            // immediately and let the existing snapshot crossfade hide the
            // content reflow.
            window.configure_rect(dst);
            self.remap_tracked_window_sized(window.clone(), dst.loc, dst.size, true);
            self.window_state_anim.animate_default(surface, src, dst);
        }
    }
}

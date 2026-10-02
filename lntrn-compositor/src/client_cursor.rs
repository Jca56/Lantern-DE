//! Magnification for cursors that X11 clients supply as raw bitmaps.
//!
//! A themed cursor follows `[input].cursor_size` because the client loads it
//! from our Xcursor theme at that size. A game that ships its own artwork
//! (Unity, most Wine/Proton titles) hands XWayland a fixed bitmap instead —
//! typically 32px — and no theme or size setting ever reaches it. On a 4K
//! panel that is a speck next to the Lantern cursor.
//!
//! So the compositor scales such a cursor up to the configured size itself,
//! the way Windows' pointer-size setting does. Only XWayland cursors are
//! touched: native Wayland clients size their own and stay pixel-exact.

use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    xwayland::XWaylandClientData,
};

/// A cursor this close to the configured size is a themed one that landed on
/// the theme's nearest packed size — leave it unscaled and sharp.
const DEAD_BAND: f64 = 1.15;

/// Ceiling on the blow-up, so a deliberately tiny cursor (a crosshair dot,
/// a 1px "hidden" cursor) can't turn into a screen-covering blob.
const MAX_FACTOR: f64 = 4.0;

/// How much to enlarge the client cursor `surface` so its longer side matches
/// `cursor_size` (logical px). 1.0 = draw it as supplied.
pub fn magnification(surface: &WlSurface, cursor_size: u32) -> f64 {
    let from_x11 = surface
        .client()
        .is_some_and(|client| client.get_data::<XWaylandClientData>().is_some());
    if !from_x11 {
        return 1.0;
    }
    let Some(size) = with_renderer_surface_state(surface, |rss| rss.surface_size()).flatten()
    else {
        return 1.0;
    };
    let longest = size.w.max(size.h);
    if longest <= 0 {
        return 1.0;
    }
    // Same floor as the Lantern cursor's own rasterizer (cursor.rs).
    let target = cursor_size.max(32) as f64;
    let factor = target / longest as f64;
    if factor < DEAD_BAND {
        1.0
    } else {
        factor.min(MAX_FACTOR)
    }
}

//! Bar-wide theme state for border/bevel colors. Keeps popup and menu
//! modules decoupled from the theme setting without threading a `lantern`
//! bool through every draw signature.

use std::sync::atomic::{AtomicBool, Ordering};

use lntrn_render::Color;

static LANTERN: AtomicBool = AtomicBool::new(false);

pub fn set_lantern(v: bool) {
    LANTERN.store(v, Ordering::Relaxed);
}

pub fn is_lantern() -> bool {
    LANTERN.load(Ordering::Relaxed)
}

/// Border stroke color used by popups, modals, and menu panels.
pub fn popup_border() -> Color {
    if is_lantern() {
        // Match lntrn-terminal's Lantern window edge (amber/gold).
        Color::from_rgba8(230, 160, 50, 255)
    } else {
        Color::BLACK
    }
}

/// Inner-shadow bevel color for the bar itself. `opacity` scales the alpha
/// so the bevel tracks the user's bar opacity setting.
pub fn bar_bevel(opacity: f32) -> Color {
    if is_lantern() {
        // Amber glow matching the popup/menu border color.
        Color::from_rgba8(230, 160, 50, 255).with_alpha(0.45 * opacity)
    } else {
        Color::rgba(0.25, 0.25, 0.25, 0.30 * opacity)
    }
}

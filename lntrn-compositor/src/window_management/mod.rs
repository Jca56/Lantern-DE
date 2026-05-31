//! Window lifecycle: focus, minimize, maximize, fullscreen, restore,
//! cycle, alt-tab, SSD interactions. Each domain lives in its own
//! file as an extra `impl Lantern` block.

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::desktop::Window;

mod alt_tab;
mod focus;
mod fullscreen;
mod half_pose;
mod lifecycle;
mod maximize;
mod minimize;
mod smart_snap;
mod smooth_resize;
mod solo_tile;
mod ssd;
mod window_swap;

pub use half_pose::PoseSlot;

/// A cardinal direction for the keyboard window-management scheme
/// (snap / in-place resize / swap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowDir {
    Left,
    Right,
    Up,
    Down,
}

/// Action to take in response to an SSD button click.
pub enum SsdClickAction {
    Close(WlSurface),
    ToggleMaximize(WlSurface),
    Minimize(WlSurface),
    Move(Window),
}

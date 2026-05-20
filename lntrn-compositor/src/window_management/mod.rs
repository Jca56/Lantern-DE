//! Window lifecycle: focus, minimize, maximize, fullscreen, restore,
//! cycle, alt-tab, SSD interactions. Each domain lives in its own
//! file as an extra `impl Lantern` block.

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::desktop::Window;

mod alt_tab;
mod axis_resize;
mod focus;
mod fullscreen;
mod half_pose;
mod lifecycle;
mod maximize;
mod minimize;
mod smooth_resize;
mod solo_tile;
mod ssd;
mod zone_move;

pub use axis_resize::ResizeAction;
pub use half_pose::{CornerDir, PoseSlot};
pub use zone_move::{ArrowDir, MoveZone};

/// Action to take in response to an SSD button click.
pub enum SsdClickAction {
    Close(WlSurface),
    ToggleMaximize(WlSurface),
    Minimize(WlSurface),
    Move(Window),
}

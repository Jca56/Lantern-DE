//! Window-state bookkeeping entries.
//!
//! Small records the compositor stores while a window is held in a
//! non-normal layout state — minimized, fullscreen, maximized, or puffed
//! up to the solo-tile rect. Each remembers what's needed to render the
//! window in that state and to restore it afterwards. The live lists that
//! hold them (`minimized_windows`, `fullscreen_windows`, …) and the logic
//! that drives them live on `Lantern` in [`crate::state`] and
//! `crate::window_management`.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};

/// A window the user has minimized. Kept off-screen until restored; the
/// stored `location` is where it returns to.
#[derive(Clone)]
pub struct MinimizedWindow {
    pub surface: WlSurface,
    pub window: Window,
    pub location: Point<i32, Logical>,
}

/// A window currently fullscreened. `restore` is the pre-fullscreen rect.
#[derive(Clone)]
pub struct FullscreenWindow {
    pub surface: WlSurface,
    pub restore: Rectangle<i32, Logical>,
    /// Output geometry at the time of fullscreen — used as the render fallback
    /// after the state animation finishes but before the client has acked the
    /// new size. Without it, the render would briefly fall back to the
    /// stale `window.geometry().size` and the window would visually snap.
    pub target: Rectangle<i32, Logical>,
}

/// A window currently maximized. `restore` is the pre-maximize rect.
#[derive(Clone)]
pub struct MaximizedWindow {
    pub surface: WlSurface,
    pub restore: Rectangle<i32, Logical>,
    /// Output geometry at the time of maximize — see [`FullscreenWindow::target`].
    pub target: Rectangle<i32, Logical>,
}

/// A window currently puffed up to the "solo tile" rect (output minus
/// exclusive zones, inset by `SINGLE_WINDOW_OUTER_GAP`). The Super+Up /
/// Super+Down ladder drives this state. The entry persists across a
/// subsequent maximize so unmaximize returns the window to its
/// solo-tile rect (one rung down the ladder).
#[derive(Clone)]
pub struct SoloTiledWindow {
    pub surface: WlSurface,
    pub window: Window,
    /// Rect to restore to when un-solo'd (the "Normal" geometry).
    pub restore: Rectangle<i32, Logical>,
    pub target: Rectangle<i32, Logical>,
}

// ── Drag-and-drop module ─────────────────────────────────────────────────────
// Handles outgoing X11 XDND protocol for dragging files out of the window.

mod overlay;
mod x11_source;

pub use overlay::DragIcon;
pub use x11_source::{DndResult, start_drag_out};

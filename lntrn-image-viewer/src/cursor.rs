//! Window-edge resize bands and the pointer shapes that go with them, plus
//! the canvas editor's hover shapes.

use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;

use crate::canvas::input::CursorHint;

/// Which edge (or corner) band of the window the pointer is in, if any.
pub fn edge_resize(cx: f32, cy: f32, w: f32, h: f32, border: f32) -> Option<ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (_, true, true, _) => Some(ResizeEdge::TopRight),
        (true, _, _, true) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Left),
        (_, true, _, _) => Some(ResizeEdge::Right),
        (_, _, true, _) => Some(ResizeEdge::Top),
        (_, _, _, true) => Some(ResizeEdge::Bottom),
        _ => None,
    }
}

/// Map a resize edge to the matching directional cursor shape.
pub fn resize_edge_shape(edge: ResizeEdge) -> Shape {
    match edge {
        ResizeEdge::Top => Shape::NResize,
        ResizeEdge::Bottom => Shape::SResize,
        ResizeEdge::Left => Shape::WResize,
        ResizeEdge::Right => Shape::EResize,
        ResizeEdge::TopLeft => Shape::NwResize,
        ResizeEdge::TopRight => Shape::NeResize,
        ResizeEdge::BottomLeft => Shape::SwResize,
        ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

/// Canvas-mode pointer shapes: sidebar grip, item grab, and handle resizes.
pub fn canvas_shape(hint: CursorHint) -> Shape {
    use crate::canvas::editor::ResizeHandle as H;
    match hint {
        CursorHint::Default => Shape::Default,
        CursorHint::ColResize => Shape::ColResize,
        CursorHint::Grab => Shape::Grab,
        CursorHint::Grabbing => Shape::Grabbing,
        CursorHint::Resize(h) => match h {
            H::TopLeft | H::BottomRight => Shape::NwseResize,
            H::TopRight | H::BottomLeft => Shape::NeswResize,
            H::Top | H::Bottom => Shape::NsResize,
            H::Left | H::Right => Shape::EwResize,
        },
    }
}

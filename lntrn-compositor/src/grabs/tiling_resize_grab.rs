//! Pointer grab that resizes BSP tile splits in real time.
//!
//! Two construction modes:
//! - **Edge mode** (`for_edge`): user grabbed an outer edge of a tiled leaf.
//!   We walk up from the leaf to the nearest matching ancestor split per axis
//!   (one for L/R edges, one for T/B edges); a corner drag yields two anchors.
//! - **Seam mode** (`for_seam`): user grabbed the gap between two tiles.
//!   Single anchor — the directly grabbed Split.
//!
//! On each motion event we convert the pointer delta into a ratio delta along
//! each anchor's axis (`delta_logical / parent_extent_logical`) and apply it
//! immediately via `apply_tiling_layout_immediate` so the layout tracks the
//! cursor without animation lag.

use crate::{grabs::resize_grab::ResizeEdge, Lantern};
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, GestureHoldBeginEvent,
        GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
        GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData, MotionEvent,
        PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::tiling::SplitDirection;
use crate::window_ext::WindowExt;

/// One tile-resize axis. A grab carries up to two of these (H + V for a corner).
#[derive(Debug, Clone)]
pub struct ResizeAnchor {
    pub split_idx: usize,
    /// Axis whose pointer-delta drives the ratio.
    pub axis: SplitDirection,
    /// Logical extent of the parent split rect along `axis` at grab start.
    pub parent_extent: i32,
    pub start_ratio: f32,
}

pub struct TilingResizeGrab {
    start_data: PointerGrabStartData<Lantern>,
    grab_button: u32,
    cursor_icon: CursorIcon,
    output_name: String,
    anchors: Vec<ResizeAnchor>,
}

impl TilingResizeGrab {
    /// Build a grab from an outer-edge press on a tiled window.
    /// Returns None if the surface isn't in a tiling tree, or no matching
    /// ancestor split exists in the requested direction(s).
    pub fn for_edge(
        start_data: PointerGrabStartData<Lantern>,
        window: &Window,
        edges: ResizeEdge,
        data: &Lantern,
    ) -> Option<Self> {
        let surface = window.get_wl_surface()?;
        let output_name = data.workspaces.output_for_tiled_surface(&surface)?;
        let area = data.tiling_area_for_surface(&surface)?;
        let tree = data.workspaces.active_tiling_tree(&output_name)?;

        let mut anchors = Vec::with_capacity(2);

        // Horizontal axis (LEFT/RIGHT edge): leaf must be in the side OPPOSITE
        // the edge. Right-edge → leaf is in left subtree; left-edge → right.
        if edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            let want_left_side = edges.intersects(ResizeEdge::RIGHT);
            if let Some(idx) = tree.find_ancestor_split(&surface, SplitDirection::Horizontal, want_left_side) {
                if let (Some(parent_rect), Some(start_ratio)) =
                    (tree.node_rect(idx, area), tree.split_ratio(idx))
                {
                    anchors.push(ResizeAnchor {
                        split_idx: idx,
                        axis: SplitDirection::Horizontal,
                        parent_extent: parent_rect.size.w.max(1),
                        start_ratio,
                    });
                }
            }
        }
        // Vertical axis (TOP/BOTTOM): bottom-edge → leaf is in top subtree.
        if edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
            let want_left_side = edges.intersects(ResizeEdge::BOTTOM);
            if let Some(idx) = tree.find_ancestor_split(&surface, SplitDirection::Vertical, want_left_side) {
                if let (Some(parent_rect), Some(start_ratio)) =
                    (tree.node_rect(idx, area), tree.split_ratio(idx))
                {
                    anchors.push(ResizeAnchor {
                        split_idx: idx,
                        axis: SplitDirection::Vertical,
                        parent_extent: parent_rect.size.h.max(1),
                        start_ratio,
                    });
                }
            }
        }

        if anchors.is_empty() {
            return None;
        }

        let cursor_icon = crate::grabs::resize_grab::ResizeSurfaceGrab::cursor_icon_for_edges(edges);
        let grab_button = start_data.button;
        Some(Self { start_data, grab_button, cursor_icon, output_name, anchors })
    }

    /// Build a grab from a seam press: pointer is in the gap between two
    /// tiles. `split_idx` and `parent_rect` come from `TilingState::seam_at`.
    pub fn for_seam(
        start_data: PointerGrabStartData<Lantern>,
        output_name: String,
        split_idx: usize,
        parent_rect: Rectangle<i32, Logical>,
        axis: SplitDirection,
        start_ratio: f32,
    ) -> Self {
        let parent_extent = match axis {
            SplitDirection::Horizontal => parent_rect.size.w,
            SplitDirection::Vertical => parent_rect.size.h,
        }
        .max(1);
        let cursor_icon = match axis {
            SplitDirection::Horizontal => CursorIcon::EwResize,
            SplitDirection::Vertical => CursorIcon::NsResize,
        };
        let grab_button = start_data.button;
        Self {
            start_data,
            grab_button,
            cursor_icon,
            output_name,
            anchors: vec![ResizeAnchor { split_idx, axis, parent_extent, start_ratio }],
        }
    }
}

impl PointerGrab<Lantern> for TilingResizeGrab {
    fn motion(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        data.cursor.set_status(CursorImageStatus::Named(self.cursor_icon));

        let delta = event.location - self.start_data.location;
        for anchor in &self.anchors {
            let along = match anchor.axis {
                SplitDirection::Horizontal => delta.x,
                SplitDirection::Vertical => delta.y,
            };
            let new_ratio = anchor.start_ratio + (along as f32 / anchor.parent_extent as f32);
            data.workspaces
                .set_split_ratio_on_output(&self.output_name, anchor.split_idx, new_ratio);
        }
        data.apply_tiling_layout_immediate();
    }

    fn relative_motion(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&self.grab_button) {
            handle.unset_grab(self, data, event.serial, event.time, true);
            data.cursor
                .set_status(CursorImageStatus::Named(CursorIcon::Default));
            data.apply_tiling_layout_immediate();
        }
    }

    fn axis(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, details: AxisFrame) {
        handle.axis(data, details)
    }
    fn frame(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>) {
        handle.frame(data);
    }
    fn gesture_swipe_begin(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GestureSwipeBeginEvent) {
        handle.gesture_swipe_begin(data, event)
    }
    fn gesture_swipe_update(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GestureSwipeUpdateEvent) {
        handle.gesture_swipe_update(data, event)
    }
    fn gesture_swipe_end(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GestureSwipeEndEvent) {
        handle.gesture_swipe_end(data, event)
    }
    fn gesture_pinch_begin(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GesturePinchBeginEvent) {
        handle.gesture_pinch_begin(data, event)
    }
    fn gesture_pinch_update(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GesturePinchUpdateEvent) {
        handle.gesture_pinch_update(data, event)
    }
    fn gesture_pinch_end(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GesturePinchEndEvent) {
        handle.gesture_pinch_end(data, event)
    }
    fn gesture_hold_begin(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GestureHoldBeginEvent) {
        handle.gesture_hold_begin(data, event)
    }
    fn gesture_hold_end(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>, event: &GestureHoldEndEvent) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Lantern> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Lantern) {}
}

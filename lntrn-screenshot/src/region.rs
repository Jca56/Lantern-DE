//! Setting the region: dragging it out, moving it, sizing it by its
//! handles, and picking a window to take its place.

use crate::selection::{DragMode, HandleEdge, Selection};
use crate::{SelectionUi, UiMode};

impl SelectionUi {
    /// Topmost window under the cursor = last match in bottom→top order.
    pub(crate) fn update_hover_window(&mut self, cx: f32, cy: f32) {
        self.hover_window = self.windows.iter().rposition(|w| w.contains(cx, cy));
    }

    pub(crate) fn on_pick_window_click(&mut self, cx: f32, cy: f32) {
        self.update_hover_window(cx, cy);
        if let Some(idx) = self.hover_window {
            let w = &self.windows[idx];
            // Clamp to the captured area so an off-screen window edge can't
            // produce a selection outside the image.
            let x = w.x.max(0.0);
            let y = w.y.max(0.0);
            let right = (w.x + w.w).min(self.capture_width as f32);
            let bottom = (w.y + w.h).min(self.capture_height as f32);
            self.selection = Some(Selection::from_normalized(x, y, right - x, bottom - y));
        }
        // Whether or not we hit a window, leave pick mode — a stray click in
        // empty space drops back to normal rather than trapping the user.
        self.mode = UiMode::Normal;
    }

    pub(crate) fn on_cursor_moved(&mut self, cx: f32, cy: f32) {
        match self.drag_mode {
            DragMode::New { start_x, start_y } => {
                self.selection = Some(Selection {
                    x: start_x,
                    y: start_y,
                    w: cx - start_x,
                    h: cy - start_y,
                });
            }
            DragMode::Handle { edge, orig } => {
                let (ox, oy, ow, oh) = orig;
                let (nx, ny, nw, nh) = match edge {
                    HandleEdge::TopLeft => (cx, cy, ox + ow - cx, oy + oh - cy),
                    HandleEdge::Top => (ox, cy, ow, oy + oh - cy),
                    HandleEdge::TopRight => (ox, cy, cx - ox, oy + oh - cy),
                    HandleEdge::Right => (ox, oy, cx - ox, oh),
                    HandleEdge::BottomRight => (ox, oy, cx - ox, cy - oy),
                    HandleEdge::Bottom => (ox, oy, ow, cy - oy),
                    HandleEdge::BottomLeft => (cx, oy, ox + ow - cx, cy - oy),
                    HandleEdge::Left => (cx, oy, ox + ow - cx, oh),
                };
                self.selection = Some(Selection::from_normalized(nx, ny, nw, nh));
            }
            DragMode::Move { offset_x, offset_y } => {
                if let Some(ref sel) = self.selection {
                    let (_, _, w, h) = sel.normalized();
                    self.selection = Some(Selection::from_normalized(
                        cx - offset_x,
                        cy - offset_y,
                        w,
                        h,
                    ));
                }
            }
            DragMode::None => {}
        }
    }

    pub(crate) fn on_left_pressed(&mut self, cx: f32, cy: f32) {
        if let Some(ref sel) = self.selection {
            if let Some(edge) = sel.hit_handle(cx, cy) {
                let orig = sel.normalized();
                self.drag_mode = DragMode::Handle { edge, orig };
                return;
            }
            if sel.contains(cx, cy) {
                let (sx, sy, _, _) = sel.normalized();
                self.drag_mode = DragMode::Move {
                    offset_x: cx - sx,
                    offset_y: cy - sy,
                };
                return;
            }
        }
        self.drag_mode = DragMode::New {
            start_x: cx,
            start_y: cy,
        };
        self.selection = None;
    }

    pub(crate) fn on_left_released(&mut self) {
        if let Some(ref sel) = self.selection {
            let (x, y, w, h) = sel.normalized();
            if w > 2.0 && h > 2.0 {
                self.selection = Some(Selection::from_normalized(x, y, w, h));
            } else {
                self.selection = None;
            }
        }
        self.drag_mode = DragMode::None;
    }
}

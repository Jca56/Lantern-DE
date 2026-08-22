//! Selection rectangle + drag-handle hit testing for the screenshot UI.
//!
//! All coordinates are in *physical* pixels — the wayland layer reports
//! pointer positions in logical (surface-local) units, and `main.rs`
//! scales those by the fractional output scale before feeding them here.

pub const HANDLE_SIZE: f32 = 16.0;
pub const HANDLE_HIT: f32 = 22.0;

#[derive(Clone)]
pub struct Selection {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum HandleEdge {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

#[derive(Clone, Copy, PartialEq)]
pub enum DragMode {
    None,
    New {
        start_x: f32,
        start_y: f32,
    },
    Handle {
        edge: HandleEdge,
        orig: (f32, f32, f32, f32),
    },
    Move {
        offset_x: f32,
        offset_y: f32,
    },
}

impl Selection {
    pub fn normalized(&self) -> (f32, f32, f32, f32) {
        let (x, w) = if self.w < 0.0 {
            (self.x + self.w, -self.w)
        } else {
            (self.x, self.w)
        };
        let (y, h) = if self.h < 0.0 {
            (self.y + self.h, -self.h)
        } else {
            (self.y, self.h)
        };
        (x, y, w, h)
    }

    pub fn from_normalized(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn hit_handle(&self, cx: f32, cy: f32) -> Option<HandleEdge> {
        let (x, y, w, h) = self.normalized();
        let half = HANDLE_HIT;
        let on_left = (cx - x).abs() < half;
        let on_right = (cx - (x + w)).abs() < half;
        let on_top = (cy - y).abs() < half;
        let on_bottom = (cy - (y + h)).abs() < half;
        let in_x = cx >= x - half && cx <= x + w + half;
        let in_y = cy >= y - half && cy <= y + h + half;

        if on_top && on_left {
            return Some(HandleEdge::TopLeft);
        }
        if on_top && on_right {
            return Some(HandleEdge::TopRight);
        }
        if on_bottom && on_left {
            return Some(HandleEdge::BottomLeft);
        }
        if on_bottom && on_right {
            return Some(HandleEdge::BottomRight);
        }
        if on_top && in_x {
            return Some(HandleEdge::Top);
        }
        if on_bottom && in_x {
            return Some(HandleEdge::Bottom);
        }
        if on_left && in_y {
            return Some(HandleEdge::Left);
        }
        if on_right && in_y {
            return Some(HandleEdge::Right);
        }
        None
    }

    pub fn contains(&self, cx: f32, cy: f32) -> bool {
        let (x, y, w, h) = self.normalized();
        cx >= x && cx <= x + w && cy >= y && cy <= y + h
    }
}

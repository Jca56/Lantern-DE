//! The area being chosen, as geometry: dragging one out, its edges and
//! corners taken to resize it, moving it whole. Physical pixels of the
//! picker's surface, which covers its monitor.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::CursorIcon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Edge {
    pub const ALL: [Edge; 8] = [Edge::TopLeft, Edge::Top, Edge::TopRight, Edge::Right, Edge::BottomRight, Edge::Bottom, Edge::BottomLeft, Edge::Left];

    /// Where its handle sits on `r`.
    pub fn handle(self, r: Rect) -> Vec2 {
        let (l, c, rt) = (r.min.x, r.center().x, r.max.x);
        let (t, m, b) = (r.min.y, r.center().y, r.max.y);
        match self {
            Edge::TopLeft => Vec2::new(l, t),
            Edge::Top => Vec2::new(c, t),
            Edge::TopRight => Vec2::new(rt, t),
            Edge::Right => Vec2::new(rt, m),
            Edge::BottomRight => Vec2::new(rt, b),
            Edge::Bottom => Vec2::new(c, b),
            Edge::BottomLeft => Vec2::new(l, b),
            Edge::Left => Vec2::new(l, m),
        }
    }

    pub fn cursor(self) -> CursorIcon {
        match self {
            Edge::TopLeft | Edge::BottomRight => CursorIcon::NwseResize,
            Edge::TopRight | Edge::BottomLeft => CursorIcon::NeswResize,
            Edge::Top | Edge::Bottom => CursorIcon::NsResize,
            Edge::Left | Edge::Right => CursorIcon::EwResize,
        }
    }
}

/// The edge or corner of `r` within `reach` of `p`, corners first.
pub fn edge_at(r: Rect, p: Vec2, reach: f64) -> Option<Edge> {
    let near = |a: f64, b: f64| (a - b).abs() <= reach;
    let (left, right, top, bottom) = (near(p.x, r.min.x), near(p.x, r.max.x), near(p.y, r.min.y), near(p.y, r.max.y));
    let across = p.x >= r.min.x - reach && p.x <= r.max.x + reach;
    let along = p.y >= r.min.y - reach && p.y <= r.max.y + reach;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(Edge::TopLeft),
        (_, true, true, _) => Some(Edge::TopRight),
        (true, _, _, true) => Some(Edge::BottomLeft),
        (_, true, _, true) => Some(Edge::BottomRight),
        (_, _, true, _) if across => Some(Edge::Top),
        (_, _, _, true) if across => Some(Edge::Bottom),
        (true, ..) if along => Some(Edge::Left),
        (_, true, ..) if along => Some(Edge::Right),
        _ => None,
    }
}

/// The rectangle with corners at `a` and `b`, whichever way round.
pub fn span(a: Vec2, b: Vec2) -> Rect {
    Rect::new(Vec2::new(a.x.min(b.x), a.y.min(b.y)), Vec2::new(a.x.max(b.x), a.y.max(b.y)))
}

/// `start` with `edge` dragged to `p`; dragged past the opposite side,
/// it turns over rather than going inside out.
pub fn resize(start: Rect, edge: Edge, p: Vec2) -> Rect {
    let (mut a, mut b) = (start.min, start.max);
    match edge {
        Edge::TopLeft => a = p,
        Edge::Top => a.y = p.y,
        Edge::TopRight => (b.x, a.y) = (p.x, p.y),
        Edge::Right => b.x = p.x,
        Edge::BottomRight => b = p,
        Edge::Bottom => b.y = p.y,
        Edge::BottomLeft => (a.x, b.y) = (p.x, p.y),
        Edge::Left => a.x = p.x,
    }
    span(a, b)
}

/// `r` moved to have its top left at `to`, kept inside `within`.
pub fn moved(r: Rect, to: Vec2, within: Rect) -> Rect {
    let size = r.size();
    let x = to.x.clamp(within.min.x, (within.max.x - size.x).max(within.min.x));
    let y = to.y.clamp(within.min.y, (within.max.y - size.y).max(within.min.y));
    Rect::from_min_size(Vec2::new(x, y), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Rect {
        Rect::from_xywh(100.0, 100.0, 200.0, 100.0)
    }

    #[test]
    fn corners_win_over_edges_and_the_middle_is_no_edge() {
        assert_eq!(edge_at(r(), Vec2::new(102.0, 98.0), 10.0), Some(Edge::TopLeft));
        assert_eq!(edge_at(r(), Vec2::new(200.0, 195.0), 10.0), Some(Edge::Bottom));
        assert_eq!(edge_at(r(), Vec2::new(305.0, 150.0), 10.0), Some(Edge::Right));
        assert_eq!(edge_at(r(), Vec2::new(200.0, 150.0), 10.0), None);
        assert_eq!(edge_at(r(), Vec2::new(500.0, 100.0), 10.0), None);
    }

    #[test]
    fn a_dragged_edge_turns_over_past_the_far_side() {
        assert_eq!(resize(r(), Edge::Right, Vec2::new(400.0, 0.0)), Rect::from_xywh(100.0, 100.0, 300.0, 100.0));
        assert_eq!(resize(r(), Edge::Left, Vec2::new(350.0, 0.0)), Rect::from_xywh(300.0, 100.0, 50.0, 100.0));
        assert_eq!(resize(r(), Edge::TopRight, Vec2::new(250.0, 50.0)), Rect::from_xywh(100.0, 50.0, 150.0, 150.0));
    }

    #[test]
    fn a_moved_area_stays_on_its_screen() {
        let screen = Rect::from_xywh(0.0, 0.0, 1000.0, 500.0);
        assert_eq!(moved(r(), Vec2::new(900.0, -50.0), screen), Rect::from_xywh(800.0, 0.0, 200.0, 100.0));
    }
}

//! The sidebar's pictures, drawn from lines and shapes so they are sharp
//! at any scale and take whatever colour the row is in. Each fills most
//! of the rect it is given; `w` is the stroke width.

use lntrn_app::lntrn_render::DrawList;
use lntrn_math::{Color, Rect, Vec2};

/// A point at `(x, y)` of the rect's half-size from its centre.
fn at(rect: Rect, x: f64, y: f64) -> Vec2 {
    let s = rect.width().min(rect.height()) * 0.5;
    rect.center() + Vec2::new(x * s, y * s)
}

fn half(rect: Rect) -> f64 {
    rect.width().min(rect.height()) * 0.5
}

/// A dashboard: four tiles, one of them lit.
pub fn overview(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let r = half(rect) * 0.16;
    for (x, y) in [(-0.85, -0.85), (0.1, -0.85), (-0.85, 0.1)] {
        d.stroke_rect(Rect::new(at(rect, x, y), at(rect, x + 0.75, y + 0.75)), w, r, color);
    }
    d.rounded_rect(Rect::new(at(rect, 0.1, 0.1), at(rect, 0.85, 0.85)), r, color);
}

/// A list: three rows, each with its bullet.
pub fn processes(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    for y in [-0.6, 0.0, 0.6] {
        d.circle(at(rect, -0.72, y), w * 0.9, color);
        d.line(at(rect, -0.35, y), at(rect, 0.85, y), w, color);
    }
}

/// A chip: a square with a core in it and legs on every side.
pub fn cpu(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.stroke_rect(Rect::new(at(rect, -0.6, -0.6), at(rect, 0.6, 0.6)), w, s * 0.16, color);
    d.rounded_rect(Rect::new(at(rect, -0.24, -0.24), at(rect, 0.24, 0.24)), s * 0.08, color);
    for t in [-0.3, 0.3] {
        d.line(at(rect, t, -0.95), at(rect, t, -0.6), w, color);
        d.line(at(rect, t, 0.6), at(rect, t, 0.95), w, color);
        d.line(at(rect, -0.95, t), at(rect, -0.6, t), w, color);
        d.line(at(rect, 0.6, t), at(rect, 0.95, t), w, color);
    }
}

/// A memory module: a board with chips on it and contacts under it.
pub fn memory(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.stroke_rect(Rect::new(at(rect, -0.92, -0.5), at(rect, 0.92, 0.35)), w, s * 0.12, color);
    for x in [-0.55, 0.0, 0.55] {
        d.line(at(rect, x, -0.22), at(rect, x, 0.08), w * 1.6, color);
        d.line(at(rect, x, 0.35), at(rect, x, 0.72), w, color);
    }
}

/// A graphics card: a board with a fan, and the bracket at its end.
pub fn gpu(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.line(at(rect, -0.92, -0.8), at(rect, -0.92, 0.8), w, color);
    d.stroke_rect(Rect::new(at(rect, -0.68, -0.5), at(rect, 0.92, 0.42)), w, s * 0.14, color);
    d.ring(at(rect, 0.14, -0.04), s * 0.27, w, color);
    d.line(at(rect, -0.4, 0.42), at(rect, -0.4, 0.75), w, color);
    d.line(at(rect, -0.4, 0.75), at(rect, 0.35, 0.75), w, color);
}

/// A drive: a box with a shelf, and its light.
pub fn disks(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    let body = Rect::new(at(rect, -0.85, -0.7), at(rect, 0.85, 0.7));
    d.stroke_rect(body, w, s * 0.2, color);
    d.hline(body.min.x + w, body.max.x - w, at(rect, 0.0, 0.18).y.round(), w, color);
    d.circle(at(rect, 0.5, 0.45), w * 0.9, color);
}

/// Traffic both ways: an arrow up beside an arrow down.
pub fn network(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    d.line(at(rect, -0.4, 0.8), at(rect, -0.4, -0.7), w, color);
    d.polyline(&[at(rect, -0.8, -0.3), at(rect, -0.4, -0.75), at(rect, 0.0, -0.3)], w, color, false);
    d.line(at(rect, 0.4, -0.8), at(rect, 0.4, 0.7), w, color);
    d.polyline(&[at(rect, 0.0, 0.3), at(rect, 0.4, 0.75), at(rect, 0.8, 0.3)], w, color, false);
}

/// A thermometer: a stem into a bulb, part way up.
pub fn sensors(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    let stem = Rect::new(at(rect, -0.2, -0.92), at(rect, 0.2, 0.3));
    d.stroke_rect(stem, w, stem.width() * 0.5, color);
    d.circle(at(rect, 0.0, 0.55), s * 0.38, color);
    d.line(at(rect, 0.0, -0.3), at(rect, 0.0, 0.4), w * 1.4, color);
}

/// About this machine: an `i` in a ring.
pub fn system(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    d.ring(rect.center(), half(rect) * 0.85, w, color);
    d.circle(at(rect, 0.0, -0.42), w * 0.85, color);
    d.line(at(rect, 0.0, -0.1), at(rect, 0.0, 0.48), w * 1.2, color);
}

/// Which way a folded row opens: a chevron pointing right when shut and
/// down when open.
pub fn chevron(d: &mut DrawList, rect: Rect, color: Color, w: f64, open: bool) {
    let tips = if open { [at(rect, -0.6, -0.3), at(rect, 0.0, 0.35), at(rect, 0.6, -0.3)] } else { [at(rect, -0.3, -0.6), at(rect, 0.35, 0.0), at(rect, -0.3, 0.6)] };
    d.polyline(&tips, w, color, false);
}

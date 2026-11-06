//! The sidebar's pictures, drawn from lines and shapes so they are sharp
//! at any scale and take whatever colour the row is in. Each fills most
//! of the rect it is given; `w` is the stroke width.

use std::f64::consts::{PI, TAU};

use lntrn_app::lntrn_render::DrawList;
use lntrn_math::{Color, Rect, Vec2};

/// A point at `(x, y)` of the rect's half-size from its centre.
fn at(rect: Rect, x: f64, y: f64) -> Vec2 {
    let s = half(rect);
    rect.center() + Vec2::new(x * s, y * s)
}

fn half(rect: Rect) -> f64 {
    rect.width().min(rect.height()) * 0.5
}

/// Points along an arc of the circle `r` (in halves of the rect) around
/// `(cx, cy)`, from angle `from` to `to`, radians clockwise from the right.
fn arc(rect: Rect, (cx, cy): (f64, f64), r: f64, from: f64, to: f64) -> Vec<Vec2> {
    (0..=16).map(|i| from + (to - from) * f64::from(i) / 16.0).map(|a| at(rect, cx + r * a.cos(), cy + r * a.sin())).collect()
}

/// A gauge: an arc with its needle well over to the right.
pub fn overview(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    d.polyline(&arc(rect, (0.0, 0.25), 0.85, PI, TAU), w, color, false);
    d.line(at(rect, 0.0, 0.25), at(rect, 0.5, -0.3), w, color);
    d.circle(at(rect, 0.0, 0.25), w * 1.4, color);
    d.line(at(rect, -0.85, 0.7), at(rect, 0.85, 0.7), w, color);
}

/// A graphics card: a board with a fan on it and its bracket.
pub fn graphics(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.stroke_rect(Rect::new(at(rect, -0.7, -0.55), at(rect, 0.95, 0.45)), w, s * 0.12, color);
    d.polyline(&arc(rect, (0.12, -0.05), 0.32, 0.0, TAU), w, color, true);
    d.circle(at(rect, 0.12, -0.05), w, color);
    d.line(at(rect, -0.95, -0.85), at(rect, -0.95, 0.85), w, color);
    d.line(at(rect, -0.95, -0.85), at(rect, -0.7, -0.85), w, color);
    d.line(at(rect, -0.45, 0.45), at(rect, -0.45, 0.75), w, color);
    d.line(at(rect, 0.05, 0.45), at(rect, 0.05, 0.75), w, color);
}

/// A fan: three blades swept around a hub, in a ring.
pub fn fan(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    d.polyline(&arc(rect, (0.0, 0.0), 0.9, 0.0, TAU), w, color, true);
    for blade in 0..3 {
        let a = f64::from(blade) * TAU / 3.0;
        // Out from the hub, curving round as it goes.
        let sweep: Vec<Vec2> = (0..=6).map(|i| f64::from(i) / 6.0).map(|t| (0.18 + t * 0.52, a + t * 0.9)).map(|(r, a)| at(rect, r * a.cos(), r * a.sin())).collect();
        d.polyline(&sweep, w * 1.6, color, false);
    }
    d.circle(rect.center(), half(rect) * 0.18, color);
}

/// A chip: a square with a core in it and legs on every side.
pub fn processor(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.stroke_rect(Rect::new(at(rect, -0.6, -0.6), at(rect, 0.6, 0.6)), w, s * 0.16, color);
    d.rounded_rect(Rect::new(at(rect, -0.24, -0.24), at(rect, 0.24, 0.24)), s * 0.08, color);
    for t in [-0.3, 0.3] {
        d.line(at(rect, t, -0.6), at(rect, t, -0.92), w, color);
        d.line(at(rect, t, 0.6), at(rect, t, 0.92), w, color);
        d.line(at(rect, -0.6, t), at(rect, -0.92, t), w, color);
        d.line(at(rect, 0.6, t), at(rect, 0.92, t), w, color);
    }
}

/// Saved settings: three sliders, each set somewhere else.
pub fn profiles(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    for (y, knob) in [(-0.6, 0.35), (0.0, -0.4), (0.6, 0.1)] {
        d.line(at(rect, -0.85, y), at(rect, 0.85, y), w, color);
        d.circle(at(rect, knob, y), half(rect) * 0.17, color);
    }
}

/// More: three dots in a row.
pub fn more(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    for x in [-0.6, 0.0, 0.6] {
        d.circle(at(rect, x, 0.0), w * 1.3, color);
    }
}

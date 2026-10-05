//! The sidebar's pictures, drawn from lines and shapes so they are sharp
//! at any scale and take whatever colour the row is in. Each fills most
//! of the rect it is given; `w` is the stroke width.

use std::f64::consts::PI;

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

/// A framed picture: a sun over a hill.
pub fn wallpaper(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.stroke_rect(Rect::new(at(rect, -0.9, -0.7), at(rect, 0.9, 0.7)), w, s * 0.22, color);
    d.circle(at(rect, -0.38, -0.2), s * 0.15, color);
    d.polyline(&[at(rect, -0.62, 0.42), at(rect, -0.1, -0.02), at(rect, 0.18, 0.24), at(rect, 0.4, 0.06), at(rect, 0.64, 0.4)], w, color, false);
}

/// A disc, half of it filled: light and dark.
pub fn appearance(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let (c, r) = (rect.center(), half(rect) * 0.8);
    d.pie(c, r - w * 0.5, -PI * 0.5, PI * 0.5, color);
    d.ring(c, r, w, color);
}

/// A window: a frame with its title bar ruled off.
pub fn windows(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    let frame = Rect::new(at(rect, -0.9, -0.72), at(rect, 0.9, 0.72));
    d.stroke_rect(frame, w, s * 0.22, color);
    d.hline(frame.min.x + w, frame.max.x - w, (frame.min.y + s * 0.42).round(), w, color);
}

/// Two panes, the front one see-through.
pub fn effects(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    d.stroke_rect(Rect::new(at(rect, -0.9, -0.8), at(rect, 0.35, 0.3)), w, s * 0.2, color);
    let front = Rect::new(at(rect, -0.35, -0.3), at(rect, 0.9, 0.8));
    d.rounded_rect(front, s * 0.2, color.fade(0.35));
    d.stroke_rect(front, w, s * 0.2, color);
}

/// A ball with speed lines behind it.
pub fn animations(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    d.circle(at(rect, 0.42, 0.0), half(rect) * 0.42, color);
    d.line(at(rect, -0.9, 0.0), at(rect, -0.2, 0.0), w, color);
    d.line(at(rect, -0.7, -0.4), at(rect, -0.25, -0.4), w, color);
    d.line(at(rect, -0.7, 0.4), at(rect, -0.25, 0.4), w, color);
}

/// A mouse seen from above, with its wheel.
pub fn mouse(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let body = Rect::new(at(rect, -0.55, -0.9), at(rect, 0.55, 0.9));
    d.stroke_rect(body, w, body.width() * 0.5, color);
    d.line(at(rect, 0.0, -0.55), at(rect, 0.0, -0.2), w, color);
}

/// A bell.
pub fn notifications(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let s = half(rect);
    let (dome, r) = (at(rect, 0.0, -0.12), s * 0.52);
    let mut outline = vec![at(rect, -0.8, 0.5), at(rect, -0.52, 0.22)];
    // The dome: the top half of a circle, left round to right.
    outline.extend((0..=12).map(|i| dome + Vec2::from_angle(PI + PI * i as f64 / 12.0) * r));
    outline.extend([at(rect, 0.52, 0.22), at(rect, 0.8, 0.5)]);
    d.polyline(&outline, w, color, true);
    d.circle(at(rect, 0.0, 0.76), s * 0.13, color);
}

/// The power mark: a broken ring with a stroke through the gap.
pub fn power(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let gap = 0.7;
    d.arc(at(rect, 0.0, 0.08), half(rect) * 0.72, -PI * 0.5 + gap, PI * 1.5 - gap, w, color);
    d.line(at(rect, 0.0, -0.9), at(rect, 0.0, -0.05), w, color);
}

/// A terminal: a prompt and the line being typed, in a window.
pub fn terminal(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    d.stroke_rect(Rect::new(at(rect, -0.9, -0.72), at(rect, 0.9, 0.72)), w, half(rect) * 0.22, color);
    d.polyline(&[at(rect, -0.5, -0.28), at(rect, -0.14, 0.02), at(rect, -0.5, 0.32)], w, color, false);
    d.line(at(rect, 0.08, 0.34), at(rect, 0.5, 0.34), w, color);
}

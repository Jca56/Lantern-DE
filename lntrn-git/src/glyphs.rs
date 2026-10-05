//! The app's pictures, drawn from lines so they are sharp at any scale
//! and take whatever colour they are given. Each fills most of the rect
//! it gets; `w` is the stroke width.

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

type Draw = lntrn_ui::IconFn;

/// A point at `(x, y)` of the rect's half-size from its centre.
fn at(rect: Rect, x: f64, y: f64) -> Vec2 {
    let s = rect.width().min(rect.height()) * 0.5;
    rect.center() + Vec2::new(x * s, y * s)
}

/// Add: stage a file, make something new.
pub const PLUS: Draw = |d, rect, color, w| {
    d.line(at(rect, -0.8, 0.0), at(rect, 0.8, 0.0), w, color);
    d.line(at(rect, 0.0, -0.8), at(rect, 0.0, 0.8), w, color);
};

/// Take away: unstage a file.
pub const MINUS: Draw = |d, rect, color, w| {
    d.line(at(rect, -0.8, 0.0), at(rect, 0.8, 0.0), w, color);
};

/// Go into: open a submodule.
pub const INTO: Draw = |d, rect, color, w| {
    d.polyline(&[at(rect, -0.35, -0.8), at(rect, 0.45, 0.0), at(rect, -0.35, 0.8)], w, color, false);
};

/// Go back out.
pub const BACK: Draw = |d, rect, color, w| {
    d.polyline(&[at(rect, 0.35, -0.8), at(rect, -0.45, 0.0), at(rect, 0.35, 0.8)], w, color, false);
};

/// Bring down: clone.
pub const DOWNLOAD: Draw = |d, rect, color, w| {
    d.line(at(rect, 0.0, -0.9), at(rect, 0.0, 0.3), w, color);
    d.polyline(&[at(rect, -0.5, -0.2), at(rect, 0.0, 0.35), at(rect, 0.5, -0.2)], w, color, false);
    d.polyline(&[at(rect, -0.85, 0.35), at(rect, -0.85, 0.85), at(rect, 0.85, 0.85), at(rect, 0.85, 0.35)], w, color, false);
};

/// The six colours the history graph's lanes cycle through.
pub const LANES: [Color; 6] = [look::INFO, look::GOOD, look::WARN, Color::hex(0xD98BF2), look::BAD, Color::hex(0x73E6D9)];

pub fn lane_color(lane: usize) -> Color {
    LANES[lane % LANES.len()]
}

/// A tick in a ring: nothing left to do.
pub fn all_clear(ui: &mut Ui, center: Vec2, radius: f64, color: Color) {
    let w = ui.m.px(3.0);
    ui.draw.ring(center, radius, w, color);
    let s = radius * 0.45;
    ui.draw.polyline(&[center + Vec2::new(-s, 0.0), center + Vec2::new(-s * 0.25, s * 0.7), center + Vec2::new(s, -s * 0.7)], w, color, false);
}

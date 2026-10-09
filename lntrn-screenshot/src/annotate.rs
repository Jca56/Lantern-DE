//! Annotations: the marks drawn on a screenshot with the pen, the
//! highlighter, the arrow, the box, the circle and the text tool.
//!
//! A mark lives in image pixels, so it stays put on the picture however
//! the region is moved or sized. Marks are drawn by the same code for the
//! live preview and for the saved image (see `ink`), so what shows is what
//! is saved.

pub mod bar;
pub mod ink;
mod input;

use lntrn_render::{Color, Painter, TextRenderer};

/// The colours on offer, as sRGB.
pub const PALETTE: [[u8; 3]; 6] = [
    [0xff, 0x3b, 0x30], // red
    [0xff, 0x9b, 0x42], // orange
    [0xff, 0xd6, 0x0a], // yellow
    [0x34, 0xc7, 0x59], // green
    [0x2f, 0x8c, 0xff], // blue
    [0xff, 0xff, 0xff], // white
];
/// How many thicknesses there are: small, medium, large.
pub const SIZES: usize = 3;

// Per thickness, in logical px (multiplied by the output scale).
const STROKE_WIDTH: [f32; SIZES] = [3.0, 6.0, 11.0];
const MARKER_WIDTH: [f32; SIZES] = [16.0, 26.0, 40.0];
const FONT_SIZE: [f32; SIZES] = [24.0, 36.0, 54.0];
/// How see-through the highlighter is.
const MARKER_ALPHA: f32 = 0.42;
/// A pen point is kept once the cursor has moved this far from the last.
const PEN_STEP: f32 = 1.5;
/// A dragged shape shorter than this is a slip, not a mark.
const SMALLEST: f32 = 3.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Marker,
    Arrow,
    Rect,
    Ellipse,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Pen,
        Tool::Marker,
        Tool::Arrow,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Text,
    ];
}

type Point = (f32, f32);

pub enum Shape {
    /// Freehand: the points the pen passed through.
    Pen(Vec<Point>),
    /// A straight see-through band, for highlighting.
    Marker { from: Point, to: Point },
    Arrow { from: Point, to: Point },
    /// The outline of the box between two opposite corners.
    Rect { from: Point, to: Point },
    /// The outline of the ellipse inside that box.
    Ellipse { from: Point, to: Point },
    /// One line of typed text; `at` is its top-left.
    Text { at: Point, text: String },
}

pub struct Mark {
    pub shape: Shape,
    pub rgb: [u8; 3],
    /// The stroke's width, or the font size for text, in image pixels.
    pub weight: f32,
}

impl Mark {
    /// A mark begun at `at` with `tool`.
    pub fn begin(tool: Tool, at: Point, color: usize, size: usize, scale: f32) -> Mark {
        let s = scale.max(1.0);
        let (shape, weight) = match tool {
            Tool::Pen => (Shape::Pen(vec![at]), STROKE_WIDTH[size]),
            Tool::Marker => (Shape::Marker { from: at, to: at }, MARKER_WIDTH[size]),
            Tool::Arrow => (Shape::Arrow { from: at, to: at }, STROKE_WIDTH[size]),
            Tool::Rect => (Shape::Rect { from: at, to: at }, STROKE_WIDTH[size]),
            Tool::Ellipse => (Shape::Ellipse { from: at, to: at }, STROKE_WIDTH[size]),
            // The click lands on the middle of the line's left end.
            Tool::Text => {
                let font = FONT_SIZE[size] * s;
                let at = (at.0, at.1 - font * 0.6);
                return Mark {
                    shape: Shape::Text {
                        at,
                        text: String::new(),
                    },
                    rgb: PALETTE[color],
                    weight: font,
                };
            }
        };
        Mark {
            shape,
            rgb: PALETTE[color],
            weight: weight * s,
        }
    }

    /// The drag has reached `to`.
    pub fn drag_to(&mut self, to: Point) {
        match &mut self.shape {
            Shape::Pen(points) => {
                let last = points[points.len() - 1];
                if (to.0 - last.0).hypot(to.1 - last.1) >= PEN_STEP {
                    points.push(to);
                }
            }
            Shape::Marker { to: end, .. }
            | Shape::Arrow { to: end, .. }
            | Shape::Rect { to: end, .. }
            | Shape::Ellipse { to: end, .. } => *end = to,
            Shape::Text { .. } => {}
        }
    }

    /// Whether there is anything to it: a dragged shape that never left
    /// where it started, or text with nothing typed, is dropped.
    pub fn is_empty(&self) -> bool {
        match &self.shape {
            Shape::Pen(_) => false,
            Shape::Marker { from, to }
            | Shape::Arrow { from, to }
            | Shape::Rect { from, to }
            | Shape::Ellipse { from, to } => (to.0 - from.0).hypot(to.1 - from.1) < SMALLEST,
            Shape::Text { text, .. } => text.trim().is_empty(),
        }
    }

    pub fn color(&self) -> Color {
        Color::from_rgb8(self.rgb[0], self.rgb[1], self.rgb[2])
    }

    /// Draw the mark's strokes. Text has none: see [`Mark::queue_text`].
    pub fn paint(&self, painter: &mut Painter) {
        let (color, w) = (self.color(), self.weight);
        match &self.shape {
            Shape::Pen(points) if points.len() == 1 => {
                painter.circle_filled(points[0].0, points[0].1, w * 0.5, color);
            }
            Shape::Pen(points) => painter.polyline_round(points, w, color),
            // One band, so its see-through colour never doubles up.
            Shape::Marker { from, to } => {
                painter.line(from.0, from.1, to.0, to.1, w, color.with_alpha(MARKER_ALPHA));
            }
            Shape::Arrow { from, to } => {
                painter.polyline_round(&[*from, *to], w, color);
                painter.polyline_round(&arrow_head(*from, *to, w), w, color);
            }
            Shape::Rect { from, to } => {
                let corners = [*from, (to.0, from.1), *to, (from.0, to.1), *from];
                painter.polyline_round(&corners, w, color);
            }
            Shape::Ellipse { from, to } => {
                painter.polyline_round(&ellipse_outline(*from, *to), w, color);
            }
            Shape::Text { .. } => {}
        }
    }

    /// Queue the mark's text, if it is text. Returns how wide it is, so
    /// the caret can sit after it.
    pub fn queue_text(&self, text: &mut TextRenderer, screen_w: u32, screen_h: u32) -> f32 {
        let Shape::Text { at, text: words } = &self.shape else {
            return 0.0;
        };
        if words.is_empty() {
            return 0.0;
        }
        let width = text.measure_width(words, self.weight);
        // Slack past the measured width, or the last glyph is clipped.
        let room = width + self.weight;
        text.queue(
            words,
            self.weight,
            at.0,
            at.1,
            self.color(),
            room,
            screen_w,
            screen_h,
        );
        width
    }
}

/// The two barbs of an arrow's head, as one stroke through its tip.
fn arrow_head(from: Point, to: Point, width: f32) -> [Point; 3] {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy).max(0.001);
    let (ux, uy) = (dx / len, dy / len);
    // Barbs no longer than the shaft itself, for a short arrow.
    let barb = (width * 4.5).max(14.0).min(len);
    let (sin, cos) = (28.0_f32).to_radians().sin_cos();
    let barb_at = |sin: f32| {
        let (bx, by) = (-(ux * cos - uy * sin), -(ux * sin + uy * cos));
        (to.0 + bx * barb, to.1 + by * barb)
    };
    [barb_at(sin), to, barb_at(-sin)]
}

/// Points round the ellipse inside the box `from`..`to`, closed.
fn ellipse_outline(from: Point, to: Point) -> Vec<Point> {
    let (cx, cy) = ((from.0 + to.0) * 0.5, (from.1 + to.1) * 0.5);
    let (rx, ry) = ((to.0 - from.0).abs() * 0.5, (to.1 - from.1).abs() * 0.5);
    // Enough segments that the largest stays a few pixels long.
    let steps = ((rx + ry) * 0.5).clamp(24.0, 200.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_that_never_moved_is_empty_and_a_pen_dot_is_not() {
        let mut arrow = Mark::begin(Tool::Arrow, (10.0, 10.0), 0, 1, 1.0);
        assert!(arrow.is_empty());
        arrow.drag_to((40.0, 30.0));
        assert!(!arrow.is_empty());
        assert!(!Mark::begin(Tool::Pen, (10.0, 10.0), 0, 1, 1.0).is_empty());
        assert!(Mark::begin(Tool::Text, (10.0, 10.0), 0, 1, 1.0).is_empty());
    }

    #[test]
    fn the_pen_drops_points_that_barely_moved() {
        let mut pen = Mark::begin(Tool::Pen, (0.0, 0.0), 0, 0, 1.0);
        pen.drag_to((0.5, 0.5));
        pen.drag_to((3.0, 0.0));
        let Shape::Pen(points) = &pen.shape else {
            panic!("a pen mark")
        };
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn an_arrows_barbs_trail_back_from_its_tip() {
        let [a, tip, b] = arrow_head((0.0, 0.0), (100.0, 0.0), 4.0);
        assert_eq!(tip, (100.0, 0.0));
        assert!(a.0 < 100.0 && b.0 < 100.0, "both behind the tip");
        assert!((a.1 + b.1).abs() < 1e-3 && a.1 != 0.0, "one each side");
    }

    #[test]
    fn an_ellipse_outline_is_closed_and_stays_in_its_box() {
        let points = ellipse_outline((10.0, 20.0), (110.0, 60.0));
        let (first, last) = (points[0], points[points.len() - 1]);
        assert!((first.0 - last.0).abs() < 1e-2 && (first.1 - last.1).abs() < 1e-2);
        assert!(points
            .iter()
            .all(|p| (9.9..=110.1).contains(&p.0) && (19.9..=60.1).contains(&p.1)));
    }
}

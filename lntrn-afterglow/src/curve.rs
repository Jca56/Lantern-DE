//! The fan curve's editor: heat along the bottom, speed up the side,
//! the curve through its points. Drag a point to move it, double-click
//! the graph to add one where you clicked. Points keep their order (each
//! cooler than the next) and the curve only climbs, so a point is held
//! between its neighbours. Speeds the fans can't run at are shaded off,
//! and the card's heat and speed right now sit on the graph as a dot.

use lntrn_afterglowd::tune::{CURVE_CELSIUS, MAX_POINTS, Point};
use lntrn_kit::{look, small_style};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, Sense, Ui, WidgetId};

/// A point's dot, and the reach of its handle.
const DOT_R: f64 = 11.0;
const HANDLE_R: f64 = 24.0;
/// The room for the side's labels and the bottom's.
const LEFT: f64 = 70.0;
const BELOW: f64 = 40.0;

/// Where the plot's numbers go on screen, and back.
struct Plot {
    rect: Rect,
}

impl Plot {
    fn span() -> (f64, f64) {
        (f64::from(CURVE_CELSIUS.0), f64::from(CURVE_CELSIUS.1))
    }

    fn to_screen(&self, celsius: f64, percent: f64) -> Vec2 {
        let (lo, hi) = Self::span();
        let r = self.rect;
        Vec2::new(r.min.x + (celsius - lo) / (hi - lo) * r.width(), r.max.y - percent / 100.0 * r.height())
    }

    fn point(&self, p: Point) -> Vec2 {
        self.to_screen(f64::from(p.celsius), f64::from(p.percent))
    }

    /// The heat and speed at `at`, in whole degrees and percent.
    fn from_screen(&self, at: Vec2) -> (i64, i64) {
        let (lo, hi) = Self::span();
        let r = self.rect;
        let celsius = lo + (at.x - r.min.x) / r.width().max(1.0) * (hi - lo);
        let percent = (r.max.y - at.y) / r.height().max(1.0) * 100.0;
        (celsius.round() as i64, percent.round() as i64)
    }
}

/// `want` for point `i`, held where it may go: cooler than the next,
/// hotter than the one before, no slower than the one before and no
/// faster than the next, and inside the fans' `range`.
pub fn place(points: &[Point], i: usize, want: (i64, i64), (lo, hi): (u32, u32)) -> Point {
    let (c_lo, c_hi) = (i64::from(CURVE_CELSIUS.0), i64::from(CURVE_CELSIUS.1));
    let before = i.checked_sub(1).and_then(|b| points.get(b));
    let after = points.get(i + 1);
    let celsius = want.0.clamp(before.map_or(c_lo, |p| i64::from(p.celsius) + 1), after.map_or(c_hi, |p| i64::from(p.celsius) - 1));
    let percent = want.1.clamp(before.map_or(i64::from(lo), |p| i64::from(p.percent)), after.map_or(i64::from(hi), |p| i64::from(p.percent)));
    Point::new(celsius.clamp(c_lo, c_hi) as u32, percent.clamp(i64::from(lo), i64::from(hi)) as u32)
}

/// A new point at `want`, among the others in order. `None` when the
/// curve is full or a point already sits at that heat.
pub fn add(points: &mut Vec<Point>, want: (i64, i64), range: (u32, u32)) -> Option<usize> {
    if points.len() >= MAX_POINTS || points.iter().any(|p| i64::from(p.celsius) == want.0) {
        return None;
    }
    let at = points.iter().position(|p| i64::from(p.celsius) > want.0).unwrap_or(points.len());
    points.insert(at, Point::new(0, 0));
    points[at] = place(points, at, want, range);
    Some(at)
}

/// The editor filling `rect`. `picked` is the point last pressed, for the
/// page's Remove button. `now` is the card's heat and fan speed. `true`
/// when a point moved or was added.
pub fn editor(ui: &mut Ui, id: WidgetId, rect: Rect, points: &mut Vec<Point>, picked: &mut Option<usize>, range: (u32, u32), now: Option<(f64, f64)>) -> bool {
    let m = ui.m;
    let small = small_style(ui);
    let plot = Plot { rect: Rect::new(Vec2::new(rect.min.x + m.px(LEFT), rect.min.y + m.px(14.0)), Vec2::new(rect.max.x - m.px(18.0), rect.max.y - m.px(BELOW))) };
    let accent = ui.theme.accent;
    let mut changed = false;

    // The points first: a press on one is theirs before the graph's.
    let mut hovered = None;
    for i in 0..points.len() {
        let at = plot.point(points[i]);
        let r = ui.interact(id.with_index(i), Rect::from_center_size(at, Vec2::splat(m.px(HANDLE_R) * 2.0)), Sense::GRAB);
        if r.hovered || r.held {
            hovered = Some(i);
            ui.state.cursor_icon = if r.held { CursorIcon::Grabbing } else { CursorIcon::Pointer };
        }
        if r.pressed {
            *picked = Some(i);
        }
        if r.dragging {
            let moved = place(points, i, plot.from_screen(ui.state.pointer), range);
            if moved != points[i] {
                points[i] = moved;
                changed = true;
            }
        }
    }
    let r = ui.interact(id.with("plot"), plot.rect, Sense::CLICK);
    if r.double_clicked && hovered.is_none() {
        if let Some(i) = add(points, plot.from_screen(ui.state.pointer), range) {
            *picked = Some(i);
            changed = true;
        }
    } else if r.pressed && hovered.is_none() {
        *picked = None;
    }

    // The ground: a well, its rules and their numbers.
    let d = &mut *ui.draw;
    d.rounded_rect(rect, m.px(12.0), look::well());
    d.stroke_rect(rect, m.px(1.0), m.px(12.0), look::line());
    let (c_lo, c_hi) = CURVE_CELSIUS;
    for c in (c_lo..=c_hi).step_by(10) {
        let x = plot.to_screen(f64::from(c), 0.0).x;
        ui.draw.vline(x, plot.rect.min.y, plot.rect.max.y, m.px(1.0), look::line());
        let label = Rect::from_center_size(Vec2::new(x, plot.rect.max.y + m.px(BELOW) * 0.5), Vec2::new(m.px(60.0), m.px(BELOW)));
        ui.text_centered(&format!("{c}°"), &small, label, look::text_dim());
    }
    for p in (0..=100).step_by(25) {
        let y = plot.to_screen(f64::from(c_lo), f64::from(p)).y;
        ui.draw.hline(plot.rect.min.x, plot.rect.max.x, y, m.px(1.0), look::line());
        let label = Rect::new(Vec2::new(rect.min.x, y - m.px(14.0)), Vec2::new(plot.rect.min.x - m.px(10.0), y + m.px(14.0)));
        ui.text_right(&format!("{p}%"), &small, label, look::text_dim());
    }
    // Below the fans' least: off limits.
    let floor = plot.to_screen(f64::from(c_lo), f64::from(range.0)).y;
    ui.draw.rect(Rect::new(Vec2::new(plot.rect.min.x, floor), plot.rect.max), look::wash(0.05));

    // The curve, held flat past its first and last points, and shaded under.
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        let mut line = vec![plot.to_screen(f64::from(c_lo), f64::from(first.percent))];
        line.extend(points.iter().map(|p| plot.point(*p)));
        line.push(plot.to_screen(f64::from(c_hi), f64::from(last.percent)));
        let shade = accent.fade(0.14);
        for pair in line.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (a0, b0) = (Vec2::new(a.x, plot.rect.max.y), Vec2::new(b.x, plot.rect.max.y));
            ui.draw.triangle(a, b, b0, shade);
            ui.draw.triangle(a, b0, a0, shade);
        }
        ui.draw.polyline(&line, m.px(3.0), accent, false);
    }

    // Where the card is now.
    if let Some((celsius, percent)) = now {
        let at = plot.to_screen(celsius.clamp(f64::from(c_lo), f64::from(c_hi)), percent.clamp(0.0, 100.0));
        ui.draw.vline(at.x, plot.rect.min.y, plot.rect.max.y, m.px(2.0), look::warn().fade(0.5));
        ui.draw.circle(at, m.px(8.0), look::warn());
        ui.draw.circle(at, m.px(4.0), look::well());
    }

    for (i, p) in points.iter().enumerate() {
        let at = plot.point(*p);
        let hot = hovered == Some(i);
        let r = m.px(if hot { DOT_R + 3.0 } else { DOT_R });
        if *picked == Some(i) {
            ui.draw.circle(at, r + m.px(6.0), accent.fade(0.3));
        }
        ui.draw.circle(at, r, look::text());
        ui.draw.circle(at, r - m.px(4.0), if *picked == Some(i) { accent } else { look::card() });
        if hot {
            let tip = format!("{}°  {}%", p.celsius, p.percent);
            let w = ui.measure(&tip, &small) + m.px(20.0);
            let h = f64::from(small.line_height()) + m.px(10.0);
            let mut bubble = Rect::from_min_size(Vec2::new(at.x - w * 0.5, at.y - r - h - m.px(10.0)), Vec2::new(w, h));
            if bubble.min.y < rect.min.y {
                bubble = Rect::from_min_size(Vec2::new(bubble.min.x, at.y + r + m.px(10.0)), bubble.size());
            }
            ui.draw.rounded_rect(bubble, m.px(8.0), Color::BLACK.fade(0.8));
            ui.text_centered(&tip, &small, bubble, look::text());
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGE: (u32, u32) = (30, 100);

    fn curve() -> Vec<Point> {
        vec![Point::new(40, 30), Point::new(60, 50), Point::new(80, 100)]
    }

    #[test]
    fn a_dragged_point_is_held_between_its_neighbours() {
        let p = curve();
        assert_eq!(place(&p, 1, (65, 70)), Point::new(65, 70));
        assert_eq!(place(&p, 1, (90, 70)), Point::new(79, 70), "not past the next one's heat");
        assert_eq!(place(&p, 1, (50, 10)), Point::new(50, 30), "no slower than the one before");
        assert_eq!(place(&p, 1, (50, 110)), Point::new(50, 100), "no faster than the next");
        assert_eq!(place(&p, 0, (0, 0)), Point::new(20, 30), "the coolest heat, the fans' least");
        assert_eq!(place(&p, 2, (200, 200)), Point::new(100, 100));
    }

    fn place(points: &[Point], i: usize, want: (i64, i64)) -> Point {
        super::place(points, i, want, RANGE)
    }

    #[test]
    fn a_point_is_added_in_order_and_not_where_one_is() {
        let mut p = curve();
        assert_eq!(add(&mut p, (50, 45), RANGE), Some(1));
        assert_eq!(p[1], Point::new(50, 45));
        assert_eq!(add(&mut p, (70, 20), RANGE), Some(3));
        assert_eq!(p[3], Point::new(70, 50), "held between 50 and 100");
        assert_eq!(add(&mut p, (60, 60), RANGE), None, "a point is at 60 already");
        assert_eq!(add(&mut p, (95, 100), RANGE), Some(5));
        while p.len() < MAX_POINTS {
            let c = 21 + p.len() as i64;
            add(&mut p, (c, 30), RANGE).unwrap();
        }
        assert_eq!(add(&mut p, (99, 100), RANGE), None, "full");
    }
}

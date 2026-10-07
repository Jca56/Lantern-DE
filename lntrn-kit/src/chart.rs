//! Charts: a graph of what something has been doing, a meter of how full
//! it is now, and the tile a dashboard shows both on. The numbers are the
//! caller's; nothing here knows what they measure.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, Sense, Ui, WidgetId};

use crate::layout::{small_style, title_style};
use crate::look;

const RADIUS: f64 = 12.0;
/// Room between a graph's frame and what is plotted in it.
const INSET: f64 = 10.0;
const LINE_W: f64 = 2.5;
/// How tall a meter's bar is, whatever the rect it is centred in.
pub const METER_H: f64 = 12.0;
/// Seconds a meter takes to reach a new value.
const GLIDE: f64 = 0.25;

/// One line of a [`graph`]: its samples, oldest first, and its colour.
#[derive(Clone, Copy, Debug)]
pub struct Series<'a> {
    pub values: &'a [f32],
    pub color: Color,
    /// Shade the area under the line.
    pub fill: bool,
}

impl<'a> Series<'a> {
    pub fn line(values: &'a [f32], color: Color) -> Self {
        Self { values, color, fill: false }
    }

    pub fn area(values: &'a [f32], color: Color) -> Self {
        Self { values, color, fill: true }
    }
}

/// What a [`graph`] plots and how.
#[derive(Clone, Copy, Debug)]
pub struct Graph<'a> {
    pub series: &'a [Series<'a>],
    /// The value at the top of the graph; samples above it are held there.
    pub max: f64,
    /// How many samples span the width. Fewer are drawn from the right
    /// edge in: a history that is still filling.
    pub slots: usize,
    /// What the top of the graph stands for ("100%"), written small in
    /// its corner. Empty: nothing.
    pub top: &'a str,
}

/// The accent while there is room, amber when it is getting full, red
/// when it nearly is.
pub fn heat(frac: f64, accent: Color) -> Color {
    if frac >= 0.9 {
        look::BAD
    } else if frac >= 0.75 {
        look::WARN
    } else {
        accent
    }
}

/// How far apart samples sit when `slots` of them span `width`.
fn step(width: f64, slots: usize) -> f64 {
    width / (slots.max(2) - 1) as f64
}

/// Where each of `values` goes in `plot`: the newest on its right edge,
/// the rest a [`step`] apart leftwards, no more than `slots` of them.
fn points(values: &[f32], plot: Rect, max: f64, slots: usize) -> Vec<Vec2> {
    let shown = &values[values.len().saturating_sub(slots.max(2))..];
    let dx = step(plot.width(), slots);
    let max = if max > 0.0 { max } else { 1.0 };
    shown
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let back = (shown.len() - 1 - i) as f64;
            let t = (f64::from(*v) / max).clamp(0.0, 1.0);
            Vec2::new(plot.max.x - back * dx, plot.max.y - t * plot.height())
        })
        .collect()
}

/// A series drawn in `plot`: its shade first, then its line.
fn trace(ui: &mut Ui, plot: Rect, s: &Series, max: f64, slots: usize, width: f64) {
    let pts = points(s.values, plot, max, slots);
    if s.fill {
        let shade = s.color.fade(0.16);
        for pair in pts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (a0, b0) = (Vec2::new(a.x, plot.max.y), Vec2::new(b.x, plot.max.y));
            ui.draw.triangle(a, b, b0, shade);
            ui.draw.triangle(a, b0, a0, shade);
        }
    }
    match pts.as_slice() {
        [] => {}
        [only] => ui.draw.circle(*only, width, s.color),
        many => ui.draw.polyline(many, width, s.color, false),
    }
}

/// A graph filling `rect`: a sunk panel ruled at each quarter, with every
/// series traced over it. Under the pointer a line marks one moment and
/// each series wears a dot there; what comes back is how many samples ago
/// that moment was (`0`: the newest), for the caller to say its values.
pub fn graph(ui: &mut Ui, id: WidgetId, rect: Rect, g: &Graph) -> Option<usize> {
    let m = ui.m;
    let radius = m.px(RADIUS);
    ui.draw.rounded_rect(rect, radius, look::WELL);
    ui.draw.stroke_rect(rect, m.px(1.0), radius, look::LINE);
    let plot = rect.shrink(m.px(INSET));
    for quarter in 1..4 {
        let y = (plot.min.y + plot.height() * f64::from(quarter) / 4.0).round();
        ui.draw.hline(plot.min.x, plot.max.x, y, m.px(1.0), look::LINE.fade(0.55));
    }
    if !g.top.is_empty() {
        let small = small_style(ui);
        ui.text_at(g.top, &small, Vec2::new(plot.min.x + m.px(4.0), plot.min.y), plot.width(), look::TEXT_DIM.fade(0.75));
    }

    ui.draw.push_clip(rect);
    for s in g.series {
        trace(ui, plot, s, g.max, g.slots, m.px(LINE_W));
    }
    ui.draw.pop_clip();

    let r = ui.interact(id, rect, Sense::NONE);
    if !r.hovered {
        return None;
    }
    let dx = step(plot.width(), g.slots);
    let back = ((plot.max.x - ui.state.pointer.x) / dx).round().max(0.0) as usize;
    let longest = g.series.iter().map(|s| s.values.len().min(g.slots.max(2))).max().unwrap_or(0);
    if back >= longest {
        return None;
    }
    let x = (plot.max.x - back as f64 * dx).round();
    ui.draw.vline(x, plot.min.y, plot.max.y, m.px(1.0), look::TEXT_DIM.fade(0.6));
    for s in g.series {
        let pts = points(s.values, plot, g.max, g.slots);
        if let Some(p) = pts.len().checked_sub(back + 1).and_then(|i| pts.get(i)) {
            ui.draw.circle(*p, m.px(6.0), look::WELL);
            ui.draw.circle(*p, m.px(4.0), s.color);
        }
    }
    Some(back)
}

/// A little graph with no frame or rules, for a tile or a row: one
/// series, shaded, across all of `rect`.
pub fn spark(ui: &mut Ui, rect: Rect, values: &[f32], color: Color, max: f64, slots: usize) {
    ui.draw.push_clip(rect);
    trace(ui, rect, &Series::area(values, color), max, slots, ui.m.px(2.0));
    ui.draw.pop_clip();
}

/// The bar of a meter in `rect`: [`METER_H`] tall, on its middle.
fn bar(ui: &Ui, rect: Rect) -> Rect {
    let h = ui.m.px(METER_H).min(rect.height());
    Rect::from_min_size(Vec2::new(rect.min.x, (rect.center().y - h * 0.5).round()), Vec2::new(rect.width(), h))
}

/// The left `frac` of `track`, never shorter than it is tall so its
/// round ends stay round.
fn filled(track: Rect, frac: f64) -> Rect {
    let w = (track.width() * frac.clamp(0.0, 1.0)).max(track.height()).min(track.width());
    Rect::from_min_size(track.min, Vec2::new(w, track.height()))
}

/// How full something is: a round-ended track across the middle of
/// `rect`, filled with `color` to `frac` (0 to 1). The fill glides to a
/// new value instead of jumping.
pub fn meter(ui: &mut Ui, id: WidgetId, rect: Rect, frac: f64, color: Color) {
    let track = bar(ui, rect);
    let round = track.height() * 0.5;
    ui.draw.rounded_rect(track, round, look::TRACK);
    let t = ui.animate(id.with("level"), frac.clamp(0.0, 1.0), GLIDE);
    if t > 0.002 {
        ui.draw.rounded_rect(filled(track, t), round, color);
    }
}

/// A meter split into parts laid end to end, each `(share, colour)` with
/// the shares adding up to at most 1: what is used, what is only kept
/// handy, and the empty track for the rest.
pub fn meter_parts(ui: &mut Ui, rect: Rect, parts: &[(f64, Color)]) {
    let track = bar(ui, rect);
    let round = track.height() * 0.5;
    ui.draw.rounded_rect(track, round, look::TRACK);
    // Where each part ends; drawn last to first, so each lies over the
    // start of the one after it.
    let mut end = 0.0;
    let ends: Vec<(f64, Color)> = parts
        .iter()
        .map(|(share, color)| {
            end += share.max(0.0);
            (end.min(1.0), *color)
        })
        .collect();
    for (end, color) in ends.into_iter().rev() {
        if end > 0.002 {
            ui.draw.rounded_rect(filled(track, end), round, color);
        }
    }
}

/// A dot of `color` then `label`, small, from `at` (the dot's left, the
/// text's top). Returns how wide the two came to, for placing the next.
pub fn legend(ui: &mut Ui, at: Vec2, color: Color, label: &str) -> f64 {
    let m = ui.m;
    let style = small_style(ui);
    let dot = m.px(6.0);
    let mid = at.y + f64::from(style.line_height()) * 0.5;
    ui.draw.circle(Vec2::new(at.x + dot, mid.round()), dot, color);
    let text_x = at.x + dot * 2.0 + m.px(8.0);
    let w = ui.measure(label, &style);
    ui.text_at(label, &style, Vec2::new(text_x, at.y), w + m.px(4.0), look::TEXT_DIM);
    text_x + w - at.x
}

/// What a [`tile`] says.
#[derive(Clone, Copy, Debug)]
pub struct Tile<'a> {
    /// What is measured: "CPU".
    pub title: &'a str,
    /// The reading, large: "37%".
    pub value: &'a str,
    /// A line under it: "4.2 GHz".
    pub note: &'a str,
    /// The reading's history, oldest first, graphed along the bottom.
    pub values: &'a [f32],
    pub max: f64,
    pub slots: usize,
    pub color: Color,
}

/// A dashboard tile filling `rect`: a card with a title, a large
/// reading, a note and the reading's history along its bottom. `true`
/// when it was clicked: it is a way in to the page about it.
pub fn tile(ui: &mut Ui, id: WidgetId, rect: Rect, t: &Tile) -> bool {
    let m = ui.m;
    let mut r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let radius = m.px(15.0);
    ui.draw.rounded_rect(rect, radius, if r.hovered { look::CARD.scale_rgb(1.25) } else { look::CARD });
    ui.draw.stroke_rect(rect, m.px(1.0), radius, if r.hovered { t.color } else { look::LINE });

    let pad = m.px(20.0);
    let (small, big) = (small_style(ui), title_style(ui));
    let (small_h, big_h) = (f64::from(small.line_height()), f64::from(big.line_height()));
    let left = rect.min.x + pad;
    let width = (rect.width() - pad * 2.0).max(1.0);
    let mut y = rect.min.y + pad * 0.8;
    let dot = m.px(6.0);
    ui.draw.circle(Vec2::new(left + dot, (y + small_h * 0.5).round()), dot, t.color);
    let title = Rect::from_min_size(Vec2::new(left + dot * 2.0 + m.px(8.0), y), Vec2::new(width - dot * 2.0 - m.px(8.0), small_h));
    ui.text_in_rect(&t.title.to_uppercase(), &small.clone().bold(), title, look::TEXT_DIM);
    y += small_h + m.px(4.0);
    ui.text_in_rect(t.value, &big, Rect::from_min_size(Vec2::new(left, y), Vec2::new(width, big_h)), look::TEXT);
    y += big_h;
    ui.text_in_rect(t.note, &small, Rect::from_min_size(Vec2::new(left, y), Vec2::new(width, small_h)), look::TEXT_DIM);
    y += small_h + m.px(8.0);

    // The history takes what is left, clear of the rounded corners.
    let floor = rect.max.y - pad * 0.6;
    if floor - y > m.px(16.0) {
        spark(ui, Rect::new(Vec2::new(left, y), Vec2::new(rect.max.x - pad, floor)), t.values, t.color, t.max, t.slots);
    }
    ui.focus_ring(id, rect);
    r.clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot() -> Rect {
        Rect::from_xywh(100.0, 50.0, 90.0, 40.0)
    }

    #[test]
    fn the_newest_sample_sits_on_the_right_edge_and_the_rest_step_back() {
        // Ten slots across 90: nine steps of 10.
        let pts = points(&[0.0, 50.0, 100.0], plot(), 100.0, 10);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[2], Vec2::new(190.0, 50.0), "newest: right edge, full height");
        assert_eq!(pts[1], Vec2::new(180.0, 70.0));
        assert_eq!(pts[0], Vec2::new(170.0, 90.0), "oldest: leftmost, on the floor");
    }

    #[test]
    fn a_full_history_spans_the_width_and_a_longer_one_drops_its_oldest() {
        let values: Vec<f32> = (0..14).map(|i| i as f32).collect();
        let pts = points(&values, plot(), 13.0, 10);
        assert_eq!(pts.len(), 10);
        assert_eq!(pts[0].x, 100.0);
        assert_eq!(pts[9].x, 190.0);
        // The last ten of fourteen: 4 is the oldest still shown.
        assert!((pts[0].y - (90.0 - 4.0 / 13.0 * 40.0)).abs() < 1e-9);
    }

    #[test]
    fn samples_past_the_top_are_held_there_and_no_scale_is_survived() {
        let pts = points(&[250.0, -3.0], plot(), 100.0, 4);
        assert_eq!((pts[0].y, pts[1].y), (50.0, 90.0));
        assert!(points(&[1.0], plot(), 0.0, 0).iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        assert!(points(&[], plot(), 100.0, 10).is_empty());
    }

    #[test]
    fn a_fill_keeps_its_round_ends_and_heat_warns_when_it_is_nearly_full() {
        let track = Rect::from_xywh(0.0, 0.0, 200.0, 12.0);
        assert_eq!(filled(track, 0.5).width(), 100.0);
        assert_eq!(filled(track, 0.01).width(), 12.0, "never thinner than it is tall");
        assert_eq!(filled(track, 7.0).width(), 200.0);
        assert_eq!(heat(0.2, look::GOLD), look::GOLD);
        assert_eq!(heat(0.8, look::GOLD), look::WARN);
        assert_eq!(heat(0.95, look::GOLD), look::BAD);
    }
}

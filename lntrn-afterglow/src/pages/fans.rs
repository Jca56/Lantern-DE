//! The graphics card's fans: their speed and the card's heat over the
//! last two minutes, then how they're driven: by the card, at one speed,
//! or along a curve of Afterglow's that the helper follows.

use lntrn_afterglowd::tune::{Fan, Point};
use lntrn_kit::chart::{Graph, Series};
use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{caption, card, note};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::Ui;

use super::{FANS, HEAT, SLOTS, SPAN, at, graph, legend, tuning};
use crate::app::App;
use crate::curve;
use crate::format;
use crate::sample::Frame;

/// How tall the curve's editor is, in logical pixels.
const EDITOR_H: f64 = 420.0;
const MODES: [&str; 3] = ["Automatic", "Fixed", "Curve"];

/// Ready-made curves, as `(°C, %)`; a speed below what the fans can do
/// is lifted to it.
pub const PRESETS: [(&str, [(u32, u32); 4]); 3] = [("Quiet", [(40, 0), (60, 40), (75, 60), (85, 100)]), ("Balanced", [(35, 0), (55, 45), (70, 70), (80, 100)]), ("Cool", [(30, 40), (50, 60), (65, 85), (75, 100)])];

pub fn preset(points: &[(u32, u32)], (lo, hi): (u32, u32)) -> Vec<Point> {
    points.iter().map(|&(c, p)| Point::new(c, p.clamp(lo, hi))).collect()
}

/// What the fan modes remember while another is picked, so trying one
/// and going back loses nothing.
#[derive(Clone, Debug, Default)]
pub struct Remembered {
    pub fixed: Option<u32>,
    pub curve: Option<Vec<Point>>,
    /// The curve's point last pressed.
    pub picked: Option<usize>,
}

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui) {
    let now = frame.gpu.as_ref().and_then(|g| Some((f64::from(g.now.celsius?), f64::from(*g.now.fans.first()?))));
    if let Some(g) = &frame.gpu {
        let shown = Graph { series: &[Series::line(&g.celsius, HEAT), Series::area(&g.fan, FANS)], max: 100.0, slots: SLOTS, top: "100" };
        let said = now.map(|(c, p)| format!("{}, {}", format::celsius(c), format::percent(p))).unwrap_or_default();
        graph(ui, "fans", &format!("Heat and fan speed, {SPAN}"), &said, &shown, |back| format!("{}, {}", format::celsius(at(&g.celsius, back)), format::percent(at(&g.fan, back))));
        legend(ui, &[(HEAT, "Heat, °C"), (FANS, "Fan speed, %")]);
    }
    if let Some(trouble) = frame.status().and_then(|s| s.trouble.as_deref()) {
        note(ui, trouble);
    }
    let fan_now = frame.status().and_then(|s| s.fan_percent);
    let memory = &mut app.fans;
    let Some((limits, draft)) = tuning(&mut app.draft, frame, ui) else { return };
    let (Some(l), Some(g)) = (&limits.gpu, draft.gpu.as_mut()) else {
        note(ui, "There is no NVIDIA card here.");
        return;
    };
    let Some(range) = l.fan_range else {
        note(ui, "This card's fans can't be set from here.");
        return;
    };

    caption(ui, "How they're driven");
    card(ui, "mode", |c| {
        let mut mode = match g.fan {
            Fan::Auto => 0,
            Fan::Fixed(_) => 1,
            Fan::Curve(_) => 2,
        };
        let hint = match mode {
            0 => "The card's own curve: quiet when cool, louder as it heats.",
            1 => "One speed, whatever the heat.",
            _ => "Your curve, followed by the helper once a second.",
        };
        if c.segmented("Mode", hint, &mut mode, &MODES) {
            let speed_now = now.map(|(_, p)| p as u32);
            g.fan = match mode {
                0 => Fan::Auto,
                1 => Fan::Fixed(memory.fixed.or(speed_now).unwrap_or(50).clamp(range.0, range.1)),
                _ => Fan::Curve(memory.curve.clone().unwrap_or_else(|| preset(&PRESETS[1].1, range))),
            };
        }
        match &mut g.fan {
            Fan::Auto => {}
            Fan::Fixed(p) => {
                let mut v = i64::from(*p);
                if c.slider_int("Speed", "", &mut v, (range.0.into(), range.1.into()), 1, |v| format::percent(v)) {
                    *p = v as u32;
                }
            }
            Fan::Curve(points) => {
                let m = c.ui.m;
                let id = c.ui.id("curve");
                c.block(m.px(EDITOR_H), |ui, rect| {
                    let inner = Rect::new(rect.min + Vec2::new(m.px(20.0), m.px(16.0)), rect.max - Vec2::new(m.px(20.0), m.px(16.0)));
                    let live = now.map(|(c, p)| (c, fan_now.map_or(p, f64::from)));
                    curve::editor(ui, id, inner, points, &mut memory.picked, range, live);
                });
                let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
                if let Some(i) = buttons(c, "Start from", "Then drag its points, or double-click the graph to add one.", &names) {
                    *points = preset(&PRESETS[i].1, range);
                    memory.picked = None;
                }
                let picked = memory.picked.filter(|i| *i < points.len());
                let can_remove = picked.is_some() && points.len() > 2;
                let hint = match picked {
                    Some(i) => format!("The one at {}°, {}%.", points[i].celsius, points[i].percent),
                    None => "Click a point to pick it.".to_owned(),
                };
                if button(c, "Remove the picked point", &hint, "Remove", can_remove) {
                    if let Some(i) = picked {
                        points.remove(i);
                    }
                    memory.picked = None;
                }
            }
        }
        // What each mode was last set to, so leaving it loses nothing.
        match &g.fan {
            Fan::Fixed(p) => memory.fixed = Some(*p),
            Fan::Curve(points) => memory.curve = Some(points.clone()),
            Fan::Auto => {}
        }
    });
    note(ui, "The curve eases down only once the card is a few degrees cooler, so the fans don't hunt. Near the heat where the card slows itself they run flat out, whatever the curve says.");
}

/// A row of `labels` as buttons at the right of a card row: the index of
/// the one pressed.
fn buttons(c: &mut lntrn_kit::Card, label: &str, hint: &str, labels: &[&str]) -> Option<usize> {
    let gap = c.ui.m.px(10.0);
    let widths: Vec<f64> = labels.iter().map(|l| controls::button_width(c.ui, l)).collect();
    let total = widths.iter().sum::<f64>() + gap * (labels.len().saturating_sub(1)) as f64;
    let id = c.ui.id(label);
    c.row(label, hint, total, |ui, slot| {
        let h = ui.m.px(controls::BUTTON_H);
        let mut x = slot.min.x;
        let mut pressed = None;
        for (i, (l, w)) in labels.iter().zip(&widths).enumerate() {
            let rect = Rect::from_min_size(Vec2::new(x, (slot.center().y - h * 0.5).round()), Vec2::new(*w, h));
            if controls::button(ui, id.with_index(i), rect, l) {
                pressed = Some(i);
            }
            x += w + gap;
        }
        pressed
    })
}

/// One button at the right of a card row, dim and dead unless `enabled`.
fn button(c: &mut lntrn_kit::Card, label: &str, hint: &str, text: &str, enabled: bool) -> bool {
    let w = controls::button_width(c.ui, text);
    let id = c.ui.id(label);
    c.row(label, hint, w, |ui, slot| {
        let h = ui.m.px(controls::BUTTON_H);
        let rect = Rect::from_min_size(Vec2::new(slot.min.x, (slot.center().y - h * 0.5).round()), Vec2::new(slot.width(), h));
        controls::button_of(ui, id, rect, text, Kind::Plain, enabled)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smoke::fixture;

    #[test]
    fn every_preset_is_a_curve_the_helper_takes() {
        let l = fixture::limits();
        let range = l.gpu.as_ref().unwrap().fan_range.unwrap();
        for (name, points) in PRESETS {
            let mut t = l.stock.clone();
            t.gpu.as_mut().unwrap().fan = Fan::Curve(preset(&points, range));
            assert_eq!(l.check(&t), Ok(()), "{name}");
        }
    }
}

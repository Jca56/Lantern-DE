//! The kit's own controls, each drawn into a rect it is handed: a pill
//! switch, a thin slider with a knob, a segmented control and a button.
//! All take the keyboard: Tab reaches them, Enter or Space clicks, the
//! arrows step.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, KeyStep, Sense, Ui, WidgetId};

use crate::look;

pub const SWITCH_W: f64 = 66.0;
const SWITCH_H: f64 = 36.0;
/// A slider's slot: its track, then its value.
pub const SLIDER_W: f64 = 400.0;
const VALUE_W: f64 = 115.0;
const KNOB_R: f64 = 13.0;
const SEGMENT_H: f64 = 48.0;
pub const BUTTON_H: f64 = 48.0;
const RADIUS: f64 = 12.0;

/// A pill switch at the right end of `slot`. A click anywhere in `hit`
/// (the row) flips it, and the row lights a little under the pointer to
/// say so. Draw the row's text after this, over that light.
pub fn switch(ui: &mut Ui, id: WidgetId, hit: Rect, slot: Rect, value: &mut bool) -> bool {
    let m = ui.m;
    let size = Vec2::new(m.px(SWITCH_W), m.px(SWITCH_H));
    let pill = Rect::from_min_size(Vec2::new(slot.max.x - size.x, (slot.center().y - size.y * 0.5).round()), size);
    let mut r = ui.interact(id, hit, Sense::CLICK);
    ui.focusable(id, pill);
    ui.key_click(id, &mut r);
    if r.clicked {
        *value = !*value;
    }
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
        let glow = Rect::new(Vec2::new(hit.min.x + m.px(8.0), hit.min.y), Vec2::new(hit.max.x - m.px(8.0), hit.max.y));
        ui.draw.rounded_rect(glow, m.px(10.0), Color::WHITE.fade(0.04));
    }
    let t = ui.animate(id.with("on"), if *value { 1.0 } else { 0.0 }, 0.12);
    let accent = ui.theme.accent;
    ui.draw.rounded_rect(pill, size.y * 0.5, look::TRACK.lerp(accent, t));
    let knob = Vec2::new(pill.min.x + size.y * 0.5 + (size.x - size.y) * t, pill.center().y);
    ui.draw.circle(knob, size.y * 0.5 - m.px(4.0), look::TEXT.lerp(look::on(accent), t));
    // The ring follows the pill's shape, not the theme's corner.
    if ui.state.focus == Some(id) && ui.state.focus_visible {
        let w = m.px(2.0);
        ui.draw.stroke_rect(pill.expand(w * 2.0), w, size.y * 0.5 + w * 2.0, ui.theme.focus);
    }
    r.clicked
}

/// `v` on the nearest `step` from `min`, inside the range, without the
/// float dust a sum of steps leaves.
fn snap(v: f64, (min, max): (f64, f64), step: f64) -> f64 {
    let v = if step > 0.0 { min + ((v - min) / step).round() * step } else { v };
    ((v * 1e6).round() / 1e6).clamp(min, max)
}

/// A slider across `slot`: a thin track with a knob, then the value as
/// `show` writes it. Drag or click along the row's whole height; the
/// arrows step it when it has the keyboard.
pub fn slider(ui: &mut Ui, id: WidgetId, slot: Rect, value: &mut f64, range: (f64, f64), step: f64, show: &dyn Fn(f64) -> String) -> bool {
    let m = ui.m;
    let (min, max) = range;
    let knob_r = m.px(KNOB_R);
    let hit = Rect::new(slot.min, Vec2::new(slot.max.x - m.px(VALUE_W), slot.max.y));
    let (x0, x1) = (hit.min.x + knob_r, hit.max.x - knob_r);
    let cy = slot.center().y.round();
    let band = Rect::new(Vec2::new(hit.min.x, cy - knob_r), Vec2::new(hit.max.x, cy + knob_r));
    let r = ui.interact(id, hit, Sense::DRAG);
    let focused = ui.focusable(id, band);
    if r.hovered || r.held {
        ui.state.cursor_icon = CursorIcon::EwResize;
    }
    let mut changed = false;
    if max > min {
        let mut v = *value;
        if r.dragging {
            let t = ((ui.state.pointer.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
            v = snap(min + t * (max - min), range, step);
        } else if focused {
            v = match ui.key_step(id) {
                KeyStep::By(n) => snap(v + n as f64 * step, range, step),
                KeyStep::Min => min,
                KeyStep::Max => max,
                KeyStep::None => v,
            };
        }
        if v != *value {
            *value = v;
            changed = true;
        }
    }

    let t = if max > min { ((*value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
    let kx = (x0 + (x1 - x0) * t).round();
    let half = m.px(3.0);
    let accent = ui.theme.accent;
    ui.draw.rounded_rect(Rect::new(Vec2::new(x0 - half, cy - half), Vec2::new(x1 + half, cy + half)), half, look::TRACK);
    ui.draw.rounded_rect(Rect::new(Vec2::new(x0 - half, cy - half), Vec2::new(kx, cy + half)), half, accent);
    let hot = r.hovered || r.held;
    let knob = Vec2::new(kx, cy);
    if hot {
        ui.draw.circle(knob, knob_r + m.px(6.0), accent.fade(0.25));
    }
    ui.draw.shadow(Rect::from_center_size(knob + Vec2::new(0.0, m.px(2.0)), Vec2::splat(knob_r * 2.0)), knob_r, m.px(6.0), Color::BLACK.fade(0.5));
    ui.draw.circle(knob, knob_r, look::TEXT);
    ui.draw.circle(knob, knob_r - m.px(4.0), if r.held { accent } else { look::CARD });

    let style = ui.text_style();
    ui.text_right(&show(*value), &style, Rect::new(Vec2::new(hit.max.x, slot.min.y), slot.max), look::TEXT);
    ui.focus_ring(id, band);
    changed
}

/// How wide [`segmented`] wants to be: every segment as wide as the
/// widest label needs.
pub fn segmented_width(ui: &mut Ui, options: &[&str]) -> f64 {
    let style = ui.text_style();
    let widest = options.iter().map(|o| ui.measure(o, &style)).fold(0.0, f64::max);
    (widest + ui.m.px(30.0)).max(ui.m.px(90.0)) * options.len() as f64
}

/// One of `options` in a trough at the right end of `slot`, the picked
/// one on an accent thumb that slides between them. One Tab stop; the
/// arrows move the pick.
pub fn segmented(ui: &mut Ui, id: WidgetId, slot: Rect, selected: &mut usize, options: &[&str]) -> bool {
    let n = options.len();
    if n == 0 {
        return false;
    }
    let m = ui.m;
    let h = m.px(SEGMENT_H);
    let w = segmented_width(ui, options).min(slot.width());
    let rect = Rect::from_min_size(Vec2::new(slot.max.x - w, (slot.center().y - h * 0.5).round()), Vec2::new(w, h));
    let part = w / n as f64;
    let mut changed = false;
    if ui.focusable(id, rect) {
        let last = n as i64 - 1;
        let next = match ui.key_step(id) {
            KeyStep::By(by) => (*selected as i64 + by as i64).clamp(0, last) as usize,
            KeyStep::Min => 0,
            KeyStep::Max => n - 1,
            KeyStep::None => *selected,
        };
        if next != *selected {
            *selected = next;
            changed = true;
        }
    }
    let segment = |i: usize| Rect::from_min_size(Vec2::new(rect.min.x + part * i as f64, rect.min.y), Vec2::new(part, h));
    let mut hovered = None;
    for i in 0..n {
        let r = ui.interact(id.with_index(i), segment(i), Sense::CLICK);
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
            hovered = Some(i);
        }
        if r.pressed {
            ui.state.focus = Some(id);
        }
        if r.clicked && *selected != i {
            *selected = i;
            changed = true;
        }
    }

    let radius = m.px(RADIUS);
    let inset = m.px(4.0);
    ui.draw.rounded_rect(rect, radius, look::WELL);
    ui.draw.stroke_rect(rect, m.px(1.0), radius, look::LINE);
    if let Some(i) = hovered.filter(|i| *i != *selected) {
        ui.draw.rounded_rect(segment(i).shrink(inset), radius - inset, Color::WHITE.fade(0.06));
    }
    let x = ui.animate(id.with("thumb"), *selected as f64, 0.12);
    let thumb = Rect::from_min_size(Vec2::new(rect.min.x + part * x, rect.min.y), Vec2::new(part, h)).shrink(inset);
    ui.draw.rounded_rect(thumb, radius - inset, ui.theme.accent);
    let style = ui.text_style();
    for (i, option) in options.iter().enumerate() {
        let ink = if i == *selected { ui.theme.accent_text } else { look::TEXT };
        ui.text_centered(option, &style, segment(i), ink);
    }
    ui.focus_ring(id, rect);
    changed
}

/// How wide a [`button`] saying `text` wants to be.
pub fn button_width(ui: &mut Ui, text: &str) -> f64 {
    let style = ui.text_style();
    ui.measure(text, &style) + ui.m.px(50.0)
}

/// A button filling `rect`. `true` when pressed.
pub fn button(ui: &mut Ui, id: WidgetId, rect: Rect, text: &str) -> bool {
    let m = ui.m;
    let mut r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let face = if r.held {
        look::BUTTON.scale_rgb(0.8)
    } else if r.hovered {
        look::BUTTON.scale_rgb(1.35)
    } else {
        look::BUTTON
    };
    let radius = m.px(RADIUS);
    ui.draw.rounded_rect(rect, radius, face);
    ui.draw.stroke_rect(rect, m.px(1.0), radius, if r.hovered { ui.theme.accent } else { look::TRACK });
    let style = ui.text_style();
    ui.text_centered(text, &style, rect, look::TEXT);
    ui.focus_ring(id, rect);
    r.clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_lands_on_steps_without_float_dust() {
        assert_eq!(snap(0.14, (0.0, 1.0), 0.05), 0.15);
        assert_eq!(snap(0.7000001, (0.1, 1.0), 0.05), 0.7);
        assert_eq!(snap(9.0, (0.0, 1.0), 0.05), 1.0);
        assert_eq!(snap(-3.0, (-1.0, 1.0), 0.05), -1.0);
        // No step: only clamped.
        assert_eq!(snap(0.123, (0.0, 1.0), 0.0), 0.123);
    }
}

//! What the popups are built of, at the launcher's sizes: the panel, a
//! column laid down its inside, switches, sliders, rows, small buttons,
//! a scrolling list (its rows as tall as each needs), and text cut short
//! with an ellipsis where it would run past its room.

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Response, Sense, Ui, WidgetId};

use crate::bar::Look;

/// Inside the panel's edge, and its corners, logical (the launcher's).
pub const PAD: f64 = 18.0;
pub const RADIUS: f64 = 16.0;
/// Rows, logical: the title's, a list's, a line of text's.
pub const HEAD_H: f64 = 48.0;
pub const ROW_H: f64 = 56.0;
pub const LINE_H: f64 = 30.0;
/// Text, logical: the title, names, and what is beside them at the 18
/// floor.
pub const TITLE: f64 = 22.0;
pub const NAME: f64 = 20.0;
pub const DIM: f64 = 18.0;
/// A list shows this many rows, and scrolls past them.
pub const MAX_ROWS: usize = 6;

pub fn title_style(ui: &Ui) -> TextStyle {
    TextStyle::new(ui.m.px(TITLE) as f32).bold()
}

pub fn name_style(ui: &Ui) -> TextStyle {
    TextStyle::new(ui.m.px(NAME) as f32)
}

pub fn dim_style(ui: &Ui) -> TextStyle {
    TextStyle::new(ui.m.px(DIM) as f32)
}

/// The panel: the launcher's, a shadow under a plate with a hairline.
pub fn panel(ui: &mut Ui, rect: Rect, look: Look) {
    let m = ui.m;
    if look.glass {
        crate::glass::pane(rect, m.px(RADIUS));
    } else {
        ui.draw.shadow(rect, m.px(RADIUS), m.px(24.0), Color::BLACK.fade(0.45));
    }
    ui.draw.rounded_rect(rect, m.px(RADIUS), look.plate);
    ui.draw.stroke_rect(rect, m.px(1.0), m.px(RADIUS), look.edge);
}

/// Rows laid one under another down the panel's inside.
pub struct Column {
    pub left: f64,
    pub right: f64,
    pub y: f64,
}

impl Column {
    pub fn new(inner: Rect) -> Column {
        Column { left: inner.min.x, right: inner.max.x, y: inner.min.y }
    }

    /// The next `h` pixels of the column.
    pub fn take(&mut self, h: f64) -> Rect {
        let r = Rect::new(Vec2::new(self.left, self.y), Vec2::new(self.right, self.y + h));
        self.y += h;
        r
    }
}

/// `text` as much of it fits in `w` pixels, an ellipsis ending what's cut.
pub fn fit(ui: &mut Ui, text: &str, style: &TextStyle, w: f64) -> String {
    if ui.measure(text, style) <= w {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let try_it: String = chars[..mid].iter().collect::<String>() + "…";
        if ui.measure(&try_it, style) <= w { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_owned() + "…"
}

/// One line of `text` from the left of `rect`, cut to fit it.
pub fn line(ui: &mut Ui, rect: Rect, text: &str, style: &TextStyle, color: Color) {
    let shown = fit(ui, text, style, rect.width());
    ui.text_in_rect(&shown, style, rect, color);
}

/// The popup's title along the top row, with `on` a switch at its right
/// end. Returns whether the switch was flipped.
pub fn header(ui: &mut Ui, row: Rect, title: &str, on: Option<bool>) -> bool {
    let style = title_style(ui);
    ui.text_in_rect(title, &style, row, ui.theme.text);
    let Some(on) = on else { return false };
    let m = ui.m;
    let size = Vec2::new(m.px(56.0), m.px(32.0));
    let rect = Rect::from_min_size(Vec2::new(row.max.x - size.x, (row.center().y - size.y * 0.5).round()), size);
    switch(ui, ui.id("switch").with(title), rect, on)
}

/// A switch in `rect`: a pill, the knob at its right and the pill lit
/// while on. Returns whether it was clicked.
pub fn switch(ui: &mut Ui, id: WidgetId, rect: Rect, on: bool) -> bool {
    let m = ui.m;
    let r = ui.interact(id, rect.expand(m.px(6.0)), Sense::CLICK);
    let t = ui.animate(id.with("knob"), if on { 1.0 } else { 0.0 }, 0.15);
    let accent = ui.theme.accent;
    let track = look::track().lerp(accent, t);
    let track = if r.hovered { track.lerp(Color::WHITE, 0.08) } else { track };
    ui.draw.rounded_rect(rect, rect.height() * 0.5, track);
    let knob_r = rect.height() * 0.5 - m.px(4.0);
    let left = rect.min.x + rect.height() * 0.5;
    let x = left + (rect.width() - rect.height()) * t;
    ui.draw.circle(Vec2::new(x, rect.center().y), knob_r, Color::WHITE.lerp(ui.theme.text, 1.0 - t));
    r.clicked
}

/// A slider across `rect`, `value` of `max`, with a tick at `mark`;
/// pressed or dragged anywhere in `rect`, it follows the pointer. Returns
/// the value it was moved to.
pub fn slider(ui: &mut Ui, id: WidgetId, rect: Rect, value: f64, max: f64, mark: Option<f64>) -> Option<f64> {
    let m = ui.m;
    let knob = m.px(11.0);
    let track = Rect::new(Vec2::new(rect.min.x + knob, (rect.center().y - m.px(4.0)).round()), Vec2::new(rect.max.x - knob, (rect.center().y + m.px(4.0)).round()));
    let r = ui.interact(id, rect, Sense::GRAB);
    let moved = (r.pressed || r.dragging).then(|| ((ui.state.pointer.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0) * max);
    let shown = moved.unwrap_or(value).clamp(0.0, max) / max.max(f64::EPSILON);
    let accent = ui.theme.accent;
    ui.draw.rounded_rect(track, track.height() * 0.5, look::track());
    let x = track.min.x + track.width() * shown;
    ui.draw.rounded_rect(Rect::new(track.min, Vec2::new(x.max(track.min.x + track.height()), track.max.y)), track.height() * 0.5, accent);
    if let Some(mark) = mark {
        let mx = (track.min.x + track.width() * (mark / max)).round();
        ui.draw.rect(Rect::new(Vec2::new(mx - m.px(1.0), track.min.y - m.px(5.0)), Vec2::new(mx + m.px(1.0), track.max.y + m.px(5.0))), ui.theme.text_dim);
    }
    let lit = r.hovered || r.held;
    ui.draw.circle(Vec2::new(x, track.center().y), if lit { knob * 1.1 } else { knob }, ui.theme.text);
    moved
}

/// A row of a list in `rect`, lit under the pointer, and when `current`.
pub fn row(ui: &mut Ui, id: WidgetId, rect: Rect, current: bool) -> Response {
    let r = ui.interact(id, rect, Sense::CLICK);
    let m = ui.m;
    if current {
        ui.draw.rounded_rect(rect, m.px(10.0), ui.theme.accent.fade(0.20));
    } else if r.hovered {
        ui.draw.rounded_rect(rect, m.px(10.0), Color::WHITE.fade(0.06));
    }
    r
}

/// A small button with `label`, its right end at `right`, centred on
/// `cy`. Returns whether it was clicked, and its rect.
pub fn button(ui: &mut Ui, id: WidgetId, right: f64, cy: f64, label: &str) -> (bool, Rect) {
    button_with(ui, id, right, cy, label, Some(ui.theme.text))
}

/// [`button`] with its label in `ink`, or dim and deaf to clicks when
/// `None`.
pub fn button_with(ui: &mut Ui, id: WidgetId, right: f64, cy: f64, label: &str, ink: Option<Color>) -> (bool, Rect) {
    let m = ui.m;
    let style = dim_style(ui);
    let w = ui.measure(label, &style) + m.px(28.0);
    let rect = Rect::new(Vec2::new(right - w, (cy - m.px(18.0)).round()), Vec2::new(right, (cy + m.px(18.0)).round()));
    let r = ui.interact(id, rect, Sense::CLICK);
    let face = match ink {
        None => 0.04,
        Some(_) if r.held => 0.16,
        Some(_) if r.hovered => 0.12,
        Some(_) => 0.07,
    };
    ui.draw.rounded_rect(rect, rect.height() * 0.5, Color::WHITE.fade(face));
    ui.text_centered(label, &style, rect, ink.unwrap_or(ui.theme.text_dim.fade(0.6)));
    (r.clicked && ink.is_some(), rect)
}

/// `count` rows of `row_h` in `rect`, scrolled by the wheel over it from
/// `scroll`, each drawn by `each(ui, index, its rect)`.
pub fn list(ui: &mut Ui, rect: Rect, count: usize, row_h: f64, scroll: &mut f64, each: impl FnMut(&mut Ui, usize, Rect)) {
    list_rows(ui, rect, &vec![row_h; count], scroll, each);
}

/// [`list`] with each row as tall as `heights` says.
pub fn list_rows(ui: &mut Ui, rect: Rect, heights: &[f64], scroll: &mut f64, mut each: impl FnMut(&mut Ui, usize, Rect)) {
    let total: f64 = heights.iter().sum();
    let max = (total - rect.height()).max(0.0);
    if rect.contains(ui.state.pointer) && ui.state.wheel.y != 0.0 {
        *scroll -= ui.state.wheel.y;
        ui.state.wheel = Vec2::ZERO;
    }
    *scroll = scroll.clamp(0.0, max);
    ui.draw.push_clip(rect);
    let mut top = rect.min.y - *scroll;
    for (i, &h) in heights.iter().enumerate() {
        let r = Rect::from_min_size(Vec2::new(rect.min.x, top), Vec2::new(rect.width(), h));
        top += h;
        if r.max.y > rect.min.y && r.min.y < rect.max.y {
            each(ui, i, r);
        }
    }
    ui.draw.pop_clip();
    if max > 0.0 {
        let m = ui.m;
        let thumb = (rect.height() * rect.height() / total).max(m.px(32.0));
        let y = rect.min.y + (rect.height() - thumb) * (*scroll / max);
        ui.draw.rounded_rect(Rect::from_min_size(Vec2::new(rect.max.x - m.px(4.0), y), Vec2::new(m.px(4.0), thumb)), m.px(2.0), ui.theme.text_dim.fade(0.5));
    }
}

/// How tall a list of `count` rows shows, pixels: no more than
/// [`MAX_ROWS`] of them.
pub fn list_h(ui: &Ui, count: usize) -> f64 {
    ui.m.px(ROW_H) * count.min(MAX_ROWS) as f64
}

/// How far `scroll` must go for the row `at` of `heights` to show whole
/// in a list `shown` tall.
pub fn scrolled_to(heights: &[f64], at: usize, shown: f64, scroll: f64) -> f64 {
    let top: f64 = heights[..at.min(heights.len())].iter().sum();
    let bottom = top + heights.get(at).copied().unwrap_or(0.0);
    scroll.max(bottom - shown).min(top)
}


#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_kit::desktop::Desktop;
    use lntrn_ui::testing::Harness;

    fn harness() -> Harness {
        let mut h = Harness::new(800.0, 600.0);
        h.theme = crate::bar::theme(&Desktop::default());
        h
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis_to_fit() {
        let mut h = harness();
        h.frame(|ui| {
            let style = name_style(ui);
            let long = "Starship/Matisse HD Audio Controller Analog Stereo";
            let cut = fit(ui, long, &style, 200.0);
            assert!(cut.ends_with('…') && ui.measure(&cut, &style) <= 200.0, "{cut}");
            assert_eq!(fit(ui, "Buds", &style, 200.0), "Buds");
        });
    }

    #[test]
    fn a_switch_flips_on_a_click() {
        let mut h = harness();
        let rect = Rect::from_min_size(Vec2::new(100.0, 100.0), Vec2::new(56.0, 32.0));
        let mut flips = 0;
        h.settle(3, |ui| _ = switch(ui, WidgetId::ROOT.with("s"), rect, false));
        h.click_at(rect.center(), |ui| flips += usize::from(switch(ui, WidgetId::ROOT.with("s"), rect, false)));
        assert_eq!(flips, 1);
    }

    #[test]
    fn a_slider_follows_the_pointer_from_a_press_anywhere_on_it() {
        let mut h = harness();
        let rect = Rect::from_min_size(Vec2::new(100.0, 100.0), Vec2::new(322.0, 56.0));
        let mut last = None;
        // Its track runs from 111 to 411: three quarters along is 336.
        h.drag(Vec2::new(150.0, 128.0), Vec2::new(336.0, 140.0), 4, |ui| {
            if let Some(v) = slider(ui, WidgetId::ROOT.with("v"), rect, 0.3, 1.2, Some(1.0)) {
                last = Some(v);
            }
        });
        let v = last.expect("moved");
        assert!((v - 0.9).abs() < 0.01, "{v}");
    }
}

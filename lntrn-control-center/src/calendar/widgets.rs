//! What the calendar's popup is built of besides the tray's popup kit:
//! arrows drawn as chevrons, a stepper (`‹ value ›`, its arrows and the
//! wheel over it stepping), and pill buttons at the popup's big sizes.

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui, WidgetId};

use crate::tray::popup::kit;

/// A chevron's half height, and its stroke, logical.
const CHEVRON: f64 = 7.0;
const STROKE: f64 = 2.5;

/// A chevron centred on `c` pointing left (`dir` -1) or right (1).
pub fn chevron(ui: &mut Ui, c: Vec2, dir: f64, color: Color) {
    let s = ui.m.px(CHEVRON);
    let dx = s * 0.5 * dir;
    let pts = [Vec2::new(c.x - dx, c.y - s), Vec2::new(c.x + dx, c.y), Vec2::new(c.x - dx, c.y + s)];
    ui.draw.polyline(&pts, ui.m.px(STROKE), color, false);
}

/// A plus (or with `plus` false, a minus) centred on `c`.
pub fn plus(ui: &mut Ui, c: Vec2, plus: bool, color: Color) {
    let (s, w) = (ui.m.px(CHEVRON), ui.m.px(STROKE));
    ui.draw.line(Vec2::new(c.x - s, c.y), Vec2::new(c.x + s, c.y), w, color);
    if plus {
        ui.draw.line(Vec2::new(c.x, c.y - s), Vec2::new(c.x, c.y + s), w, color);
    }
}

/// A round face lit under the pointer, darker while pressed. Returns the
/// response.
fn face(ui: &mut Ui, id: WidgetId, rect: Rect) -> lntrn_ui::Response {
    let r = ui.interact(id, rect, Sense::CLICK);
    let lit = if r.held {
        0.16
    } else if r.hovered {
        0.10
    } else {
        0.0
    };
    if lit > 0.0 {
        ui.draw.rounded_rect(rect, rect.height().min(rect.width()) * 0.5, Color::WHITE.fade(lit));
    }
    r
}

/// An arrow button in `rect`: a chevron pointing `dir`. Returns whether it
/// was clicked.
pub fn arrow(ui: &mut Ui, id: WidgetId, rect: Rect, dir: f64) -> bool {
    let r = face(ui, id, rect);
    chevron(ui, rect.center(), dir, ui.theme.text);
    r.clicked
}

/// A plus or minus button in `rect`. Returns whether it was clicked.
pub fn plus_minus(ui: &mut Ui, id: WidgetId, rect: Rect, up: bool) -> bool {
    let r = face(ui, id, rect);
    plus(ui, rect.center(), up, ui.theme.text);
    r.clicked
}

/// The wheel over `rect` in whole notches, a part of one kept in `acc`
/// for the next turn: down (toward you) is on, positive.
pub fn notches(ui: &mut Ui, rect: Rect, acc: &mut f64) -> i32 {
    let dy = ui.state.wheel.y;
    if dy == 0.0 || !rect.contains(ui.state.pointer) {
        return 0;
    }
    ui.state.wheel = Vec2::ZERO;
    let notch = ui.m.widget_h.max(1.0);
    *acc += dy;
    let n = (*acc / notch).trunc();
    *acc -= n * notch;
    -(n as i32)
}

/// The well a stepper's value sits in.
pub fn well(ui: &mut Ui, rect: Rect) {
    ui.draw.rounded_rect(rect, ui.m.px(12.0), look::well().fade(0.55));
}

/// A stepper across `rect`: `‹` at its left end, `›` at its right, `text`
/// in a well between. Returns the steps taken: an arrow's click, or the
/// wheel over it (down is on).
pub fn stepper(ui: &mut Ui, id: WidgetId, rect: Rect, text: &str, acc: &mut f64) -> i32 {
    let side = rect.height();
    let left = Rect::from_min_size(rect.min, Vec2::splat(side));
    let right = Rect::from_min_size(Vec2::new(rect.max.x - side, rect.min.y), Vec2::splat(side));
    let mid = Rect::new(Vec2::new(left.max.x + ui.m.px(4.0), rect.min.y), Vec2::new(right.min.x - ui.m.px(4.0), rect.max.y));
    well(ui, mid);
    let shown = kit::fit(ui, text, &kit::name_style(ui), mid.width() - ui.m.px(16.0));
    ui.text_centered(&shown, &kit::name_style(ui), mid, ui.theme.text);
    let mut steps = notches(ui, rect, acc);
    if arrow(ui, id.with("less"), left, -1.0) {
        steps -= 1;
    }
    if arrow(ui, id.with("more"), right, 1.0) {
        steps += 1;
    }
    steps
}

/// How wide a [`pill`] with `label` is, pixels.
pub fn pill_w(ui: &mut Ui, label: &str) -> f64 {
    ui.measure(label, &kit::name_style(ui)) + ui.m.px(40.0)
}

/// A button `label`led across `rect` in `ink` on `fill` (the plate's lift
/// when `None`), or dim and deaf to clicks when `ink` is `None`. Returns
/// whether it was clicked.
pub fn pill(ui: &mut Ui, id: WidgetId, rect: Rect, label: &str, ink: Option<Color>, fill: Option<Color>) -> bool {
    let r = ui.interact(id, rect, Sense::CLICK);
    let base = fill.unwrap_or(Color::WHITE.fade(0.07));
    let shade = match ink {
        None => Color::WHITE.fade(0.04),
        Some(_) if r.held => base.lerp(Color::WHITE, 0.18),
        Some(_) if r.hovered => base.lerp(Color::WHITE, 0.10),
        Some(_) => base,
    };
    ui.draw.rounded_rect(rect, rect.height() * 0.5, shade);
    let style: TextStyle = kit::name_style(ui);
    ui.text_centered(label, &style, rect, ink.unwrap_or(ui.theme.text_dim.fade(0.6)));
    r.clicked && ink.is_some()
}

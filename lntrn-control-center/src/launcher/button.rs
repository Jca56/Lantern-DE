//! The launcher's button at the plate's left end: nine big dots in the
//! clock's ink, on a face of its own that fills the bar's height, lights
//! under the pointer and stays lit while the menu is up.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Sense, Ui};

/// Between the button and the plate's edges, logical.
const MARGIN: f64 = 4.0;
/// The face's corners: the floating plate's 16 less the margin, so the
/// two curves nest.
const RADIUS: f64 = 12.0;
/// From one dot's centre to the next, and a dot's radius, as parts of the
/// button's side.
const STEP: f64 = 0.25;
const DOT: f64 = 0.09;

/// Draw the button at the left of `plate` (window pixels), on a plate of
/// colour `under`. Returns whether it was clicked, and the room it took.
pub fn button(ui: &mut Ui, plate: Rect, under: Color, open: bool) -> (bool, Rect) {
    let m = ui.m;
    let side = (plate.height() - m.px(MARGIN * 2.0)).max(m.px(24.0));
    let left = plate.min.x + m.px(MARGIN);
    let rect = Rect::new(Vec2::new(left, (plate.center().y - side * 0.5).round()), Vec2::new(left + side, (plate.center().y + side * 0.5).round()));
    let id = ui.id("launcher");
    let r = ui.interact(id, rect, Sense::CLICK);
    let (accent, ink) = (ui.theme.accent, ui.theme.text);
    // Faintly there at rest, so it reads as a button from across the desk.
    let face = if open {
        accent.fade(0.22)
    } else if r.held {
        under.lerp(Color::WHITE, 0.16)
    } else if r.hovered {
        under.lerp(Color::WHITE, 0.11)
    } else {
        under.lerp(Color::WHITE, 0.05)
    };
    ui.draw.rounded_rect(rect, m.px(RADIUS), face);
    // Three rows of three.
    let step = side * STEP;
    let dot = side * DOT;
    for row in -1..=1 {
        for col in -1..=1 {
            ui.draw.circle(rect.center() + Vec2::new(f64::from(col) * step, f64::from(row) * step), dot, ink);
        }
    }
    (r.clicked, rect)
}

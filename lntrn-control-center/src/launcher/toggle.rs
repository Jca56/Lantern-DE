//! Beside the search field: two buttons, the list and the grid, the one
//! showing lit. A click on the other switches the menu over at once; the
//! app keeps the choice in `lantern.toml` ([`super::keep`]), as System
//! Settings' View does.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Sense, Ui};

use super::Layout;

/// The two buttons in `rect`, side by side; the layout clicked, when it
/// isn't `now`.
pub fn draw(ui: &mut Ui, rect: Rect, now: Layout) -> Option<Layout> {
    let m = ui.m;
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    ui.draw.rounded_rect(rect, m.px(10.0), ink.fade(0.06));
    let w = rect.width() / 2.0;
    let mut picked = None;
    for (k, layout) in [Layout::List, Layout::Grid].into_iter().enumerate() {
        let button = Rect::from_min_size(Vec2::new(rect.min.x + k as f64 * w, rect.min.y), Vec2::new(w, rect.height()));
        let r = ui.interact(ui.id(layout.word()), button, Sense::CLICK);
        let lit = layout == now;
        let face = button.shrink(m.px(4.0));
        if lit {
            ui.draw.rounded_rect(face, m.px(8.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(face, m.px(8.0), Color::WHITE.fade(0.06));
        }
        glyph(ui, button, layout, if lit { accent } else { dim });
        if r.clicked && !lit {
            picked = Some(layout);
        }
    }
    picked
}

/// What each button shows: three rows of a dot and a line, or four tiles.
fn glyph(ui: &mut Ui, button: Rect, layout: Layout, color: Color) {
    let s = (button.width().min(button.height()) * 0.42).round();
    let b = Rect::from_center_size(button.center(), Vec2::splat(s));
    match layout {
        Layout::List => {
            let bar = (s / 5.0).round().max(2.0);
            for k in 0..3 {
                let y = (b.min.y + k as f64 * (s - bar) / 2.0).round();
                ui.draw.rounded_rect(Rect::from_min_size(Vec2::new(b.min.x, y), Vec2::splat(bar)), bar * 0.5, color);
                let x = b.min.x + bar * 1.8;
                ui.draw.rounded_rect(Rect::from_min_size(Vec2::new(x, y), Vec2::new(b.max.x - x, bar)), bar * 0.5, color);
            }
        }
        Layout::Grid => {
            let gap = (s * 0.16).round().max(2.0);
            let cell = (s - gap) / 2.0;
            for (col, row) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let at = Vec2::new(b.min.x + col * (cell + gap), b.min.y + row * (cell + gap));
                ui.draw.rounded_rect(Rect::from_min_size(at, Vec2::splat(cell)), cell * 0.25, color);
            }
        }
    }
}

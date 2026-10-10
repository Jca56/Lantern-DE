//! The ring in small: its buttons where the desktop puts them,
//! clockwise from the top, round the hub that opens the Command Center.
//! A click on a button opens it up for editing.

use std::f64::consts::{FRAC_PI_2, TAU};

use lntrn_app::lntrn_render::ImageHandle;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, Metrics, Sense, Ui};

use crate::look;

/// The desktop's ring (184, 60 and 44 there) at two thirds, in logical
/// pixels: how far out the buttons sit, and how big they and the hub are.
const ORBIT: f64 = 124.0;
const BUTTON_R: f64 = 40.0;
const HUB_R: f64 = 30.0;
/// The room around the ring inside its card.
const PAD: f64 = 30.0;

/// One button as the preview needs it.
pub struct Button<'a> {
    pub image: Option<ImageHandle>,
    /// Its first letter stands in while there is no picture.
    pub label: &'a str,
    /// Dim when the desktop would leave it out.
    pub live: bool,
}

pub fn height(m: Metrics) -> f64 {
    m.px((ORBIT + BUTTON_R + PAD) * 2.0)
}

/// Draw the ring in the middle of `rect`, `selected` lit. Returns the
/// button that was clicked.
pub fn draw(ui: &mut Ui, rect: Rect, buttons: &[Button], selected: Option<usize>) -> Option<usize> {
    let m = ui.m;
    let accent = ui.theme.accent;
    let c = rect.center().round();
    let (orbit, radius, hub) = (m.px(ORBIT), m.px(BUTTON_R), m.px(HUB_R));

    // The desktop under it, the track the buttons sit on, and the hub
    // with its four tiles.
    ui.draw.circle(c, orbit + radius + m.px(14.0), look::WELL);
    ui.draw.ring(c, orbit, m.px(2.5), accent.fade(0.45));
    ui.draw.circle(c, hub, look::BUTTON);
    ui.draw.ring(c, hub, m.px(2.0), accent.fade(0.85));
    let (tile, off) = (hub * 0.38, hub * 0.08);
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = Vec2::new(if x < 0.0 { c.x - off - tile } else { c.x + off }, if y < 0.0 { c.y - off - tile } else { c.y + off });
        ui.draw.rounded_rect(Rect::from_min_size(corner, Vec2::splat(tile)), tile * 0.28, accent);
    }

    let style = ui.text_style().bold();
    let mut clicked = None;
    ui.push_id("ring");
    for (i, button) in buttons.iter().enumerate() {
        let angle = -FRAC_PI_2 + i as f64 * TAU / buttons.len() as f64;
        let center = (c + Vec2::from_angle(angle) * orbit).round();
        let hit = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
        let id = ui.id("button").with_index(i);
        let mut r = ui.interact(id, hit, Sense::CLICK);
        ui.focusable(id, hit);
        ui.key_click(id, &mut r);
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        if r.clicked {
            clicked = Some(i);
        }
        let picked = selected == Some(i);
        let dim = if button.live { 1.0 } else { 0.35 };
        if picked {
            ui.draw.circle(center, radius + m.px(8.0), accent.fade(0.3));
        }
        ui.draw.circle(center, radius, if r.hovered { look::BUTTON.scale_rgb(1.45) } else { look::BUTTON });
        ui.draw.ring(center, radius, m.px(if picked { 3.5 } else { 2.0 }), if picked { accent } else { accent.fade(0.5) });
        match button.image {
            Some(image) => {
                let fit = radius * 1.3 / image.width.max(image.height).max(1) as f64;
                let size = Vec2::new(image.width as f64 * fit, image.height as f64 * fit);
                ui.draw.image(Rect::from_center_size(center, size).round(), image, 0.0, Color::WHITE.fade(dim));
            }
            None => {
                let initial: String = button.label.trim().chars().take(1).flat_map(char::to_uppercase).collect();
                ui.text_centered(&initial, &style, hit, look::TEXT_DIM.fade(dim));
            }
        }
        ui.focus_ring(id, hit);
    }
    ui.pop_id();
    clicked
}

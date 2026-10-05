//! Small things that say something at a glance: a badge beside a name,
//! and a banner across the page for what went wrong.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use crate::layout::small_style;
use crate::look;

/// How tall a badge is, in logical pixels.
pub const BADGE_H: f64 = 30.0;

/// How wide [`badge`] draws `text`.
pub fn badge_width(ui: &mut Ui, text: &str) -> f64 {
    let style = small_style(ui).bold();
    ui.measure(text, &style) + ui.m.px(18.0)
}

/// `text` on a pill tinted `color`, its left edge at `left` and its
/// middle on `mid_y`. Returns its width, for placing what follows.
pub fn badge(ui: &mut Ui, left: f64, mid_y: f64, text: &str, color: Color) -> f64 {
    let m = ui.m;
    let style = small_style(ui).bold();
    let w = ui.measure(text, &style) + m.px(18.0);
    let h = m.px(BADGE_H);
    let rect = Rect::from_min_size(Vec2::new(left.round(), (mid_y - h * 0.5).round()), Vec2::new(w, h));
    ui.draw.rounded_rect(rect, m.px(8.0), color.lerp(look::CARD, 0.78));
    ui.text_centered(text, &style, rect, color);
    w
}

/// A banner across the column saying `text`, tinted `color`, wrapped to
/// as many lines as it needs, with a cross at its right. `true` when the
/// cross was pressed: the banner is the caller's to stop showing.
pub fn banner(ui: &mut Ui, id: &str, text: &str, color: Color) -> bool {
    let m = ui.m;
    let style = small_style(ui);
    let pad = m.px(16.0);
    let cross = m.px(44.0);
    let text_w = (ui.avail_width() - pad * 2.0 - cross).max(1.0);
    let text_h = ui.text.measure_wrapped(text, &style, text_w as f32).height as f64;
    let rect = ui.alloc(Vec2::new(FILL, (text_h + pad * 2.0).max(cross + m.px(12.0))));
    let radius = m.px(12.0);
    ui.draw.rounded_rect(rect, radius, color.lerp(look::CARD, 0.82));
    ui.draw.stroke_rect(rect, m.px(1.0), radius, color.fade(0.6));
    ui.text_at(text, &style, Vec2::new(rect.min.x + pad, (rect.center().y - text_h * 0.5).round()), text_w, look::TEXT);

    let hit = Rect::from_center_size(Vec2::new(rect.max.x - pad - cross * 0.5 + m.px(6.0), rect.center().y), Vec2::splat(cross));
    let id = ui.id(id).with("dismiss");
    let mut r = ui.interact(id, hit, Sense::CLICK);
    ui.focusable(id, hit);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
        ui.draw.rounded_rect(hit, m.px(10.0), Color::WHITE.fade(0.1));
    }
    let (c, s) = (hit.center(), m.px(8.0));
    ui.draw.line(c - Vec2::splat(s), c + Vec2::splat(s), m.px(2.5), look::TEXT);
    ui.draw.line(c + Vec2::new(-s, s), c + Vec2::new(s, -s), m.px(2.5), look::TEXT);
    ui.focus_ring(id, hit);
    r.clicked
}

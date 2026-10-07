//! A sidebar's parts: the shade that sets it apart from the page, small
//! captions over its groups, and rows, the one showing on an accent pill.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, IconFn, Sense, Ui};

use crate::layout::small_style;
use crate::look;

/// How tall a row is, in logical pixels.
pub const ROW_H: f64 = 56.0;
/// Room between a row and the sidebar's right edge.
const EDGE: f64 = 12.0;

/// Darken the sidebar from the cursor down and rule its right edge. Call
/// it first, at the top of the sidebar's column. The shell insets a body
/// by its padding; the shade runs out to the window's edges.
pub fn shade(ui: &mut Ui) {
    let m = ui.m;
    let (top, w) = (ui.cursor(), ui.avail_width());
    let rect = Rect::new(Vec2::new(top.x - m.pad, top.y - m.pad), Vec2::new(top.x + w, top.y + ui.remaining_height() + m.pad));
    ui.draw.rect(rect, Color::BLACK.fade(0.25));
    ui.draw.vline(rect.max.x - m.px(1.0), rect.min.y, rect.max.y, m.px(1.0), look::LINE);
}

/// A small heading over a group of rows.
pub fn caption(ui: &mut Ui, s: &str) {
    let m = ui.m;
    let style = small_style(ui).bold();
    let r = ui.alloc(Vec2::new(FILL, m.px(44.0)));
    let text = Rect::new(Vec2::new(r.min.x + m.px(16.0), r.min.y + m.px(8.0)), r.max);
    ui.text_in_rect(&s.to_uppercase(), &style, text, look::TEXT_DIM);
}

/// A row: `label`, with `glyph` before it when there is one. The row
/// `showing` sits on an accent pill. `id` tells rows with the same label
/// apart. `true` when it was clicked.
pub fn row(ui: &mut Ui, id: &str, label: &str, glyph: Option<IconFn>, showing: bool) -> bool {
    row_value(ui, id, label, glyph, showing, "")
}

/// [`row`] with `value` small at its right end: what the page behind it
/// reads right now ("37%"). The label gives way to it.
pub fn row_value(ui: &mut Ui, id: &str, label: &str, glyph: Option<IconFn>, showing: bool, value: &str) -> bool {
    let m = ui.m;
    let full = ui.alloc(Vec2::new(FILL, m.px(ROW_H)));
    let rect = Rect::new(full.min, Vec2::new(full.max.x - m.px(EDGE), full.max.y));
    let id = ui.id(id);
    let mut r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let radius = m.px(12.0);
    if showing {
        ui.draw.rounded_rect(rect, radius, ui.theme.accent);
    } else if r.hovered || r.held {
        ui.draw.rounded_rect(rect, radius, Color::WHITE.fade(0.07));
    }
    let ink = if showing { ui.theme.accent_text } else { look::TEXT };
    let mut text_x = rect.min.x + m.px(18.0);
    if let Some(glyph) = glyph {
        let picture = Rect::from_center_size(Vec2::new(rect.min.x + m.px(34.0), rect.center().y), Vec2::splat(m.px(30.0)));
        glyph(ui.draw, picture, ink, m.px(2.5));
        text_x = rect.min.x + m.px(64.0);
    }
    let mut text_end = rect.max.x - m.px(10.0);
    if !value.is_empty() {
        let small = small_style(ui);
        let end = rect.max.x - m.px(14.0);
        text_end = end - ui.measure(value, &small) - m.px(8.0);
        let dim = if showing { ink.fade(0.75) } else { look::TEXT_DIM };
        ui.text_right(value, &small, Rect::new(Vec2::new(text_end, rect.min.y), Vec2::new(end, rect.max.y)), dim);
    }
    let style = ui.text_style();
    ui.text_in_rect(label, &style, Rect::new(Vec2::new(text_x, rect.min.y), Vec2::new(text_end.max(text_x), rect.max.y)), ink);
    ui.focus_ring(id, rect);
    r.clicked
}

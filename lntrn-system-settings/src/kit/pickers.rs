//! The two controls that open a popup: a dropdown and a colour chip. The
//! popups are Lantern UI's own (only it can draw on the layer above); the
//! parts that sit in the row are drawn here to match the rest of the kit.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, KeyStep, Sense, Ui, WidgetId};

use super::controls::BUTTON_H;
use crate::look;

pub const DROPDOWN_W: f64 = 360.0;
/// A colour's slot: its hex, then its chip.
pub const COLOR_W: f64 = 220.0;

/// A button across `slot` showing the picked one of `options`; pressed,
/// the rest drop down under it. Up and Down step through them unopened.
pub fn dropdown(ui: &mut Ui, id: WidgetId, slot: Rect, selected: &mut usize, options: &[&str]) -> bool {
    let m = ui.m;
    let h = m.px(BUTTON_H);
    let rect = Rect::from_min_size(Vec2::new(slot.min.x, (slot.center().y - h * 0.5).round()), Vec2::new(slot.width(), h));
    let mut r = ui.interact(id, rect, Sense::CLICK);
    let focused = ui.focusable(id, rect);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let mut changed = false;
    if focused && !options.is_empty() {
        let last = options.len() as i64 - 1;
        // Up is the row above: one less.
        let next = match ui.key_step(id) {
            KeyStep::By(by) => (*selected as i64 - by as i64).clamp(0, last) as usize,
            KeyStep::Min => 0,
            KeyStep::Max => options.len() - 1,
            KeyStep::None => *selected,
        };
        if next != *selected {
            *selected = next;
            changed = true;
        }
    }

    let open = *ui.state.open(id);
    let radius = m.px(12.0);
    let face = if r.hovered || open { look::BUTTON.scale_rgb(1.35) } else { look::BUTTON };
    ui.draw.rounded_rect(rect, radius, face);
    ui.draw.stroke_rect(rect, m.px(1.0), radius, if r.hovered || open { ui.theme.accent } else { look::TRACK });
    let style = ui.text_style();
    let chevron = m.px(7.0);
    let inner = Rect::new(Vec2::new(rect.min.x + m.px(18.0), rect.min.y), Vec2::new(rect.max.x - m.px(24.0) - chevron * 2.0, rect.max.y));
    ui.text_in_rect(options.get(*selected).copied().unwrap_or(""), &style, inner, look::TEXT);
    let c = Vec2::new(rect.max.x - m.px(18.0) - chevron, rect.center().y);
    let tips = [Vec2::new(c.x - chevron, c.y - chevron * 0.5), Vec2::new(c.x, c.y + chevron * 0.5), Vec2::new(c.x + chevron, c.y - chevron * 0.5)];
    ui.draw.polyline(&tips, m.px(2.5), look::TEXT_DIM, false);
    ui.focus_ring(id, rect);

    if r.clicked {
        *ui.state.open(id) = !open;
        ui.state.request_rebuild = true;
    }
    if *ui.state.open(id) {
        let res = ui.popup_list(id, rect, options, Some(*selected));
        if let Some(i) = res.picked {
            changed |= *selected != i;
            *selected = i;
        }
        if res.picked.is_some() || res.closed {
            *ui.state.open(id) = false;
        }
    }
    changed
}

/// A colour kept as a `#RRGGBB` string: its hex written out, then a
/// round chip at the right end of `slot` that opens the picker. An empty
/// or bad string shows as `fallback`.
pub fn color(ui: &mut Ui, label: &str, slot: Rect, hex: &mut String, fallback: Color) -> bool {
    let m = ui.m;
    let mut color = Color::parse_hex(hex).unwrap_or(fallback).with_alpha(1.0);
    // Lantern UI's picker lays its swatch (a square, a widget high) out
    // at the cursor, so the cursor goes where the chip is and comes back.
    let side = m.widget_h;
    let swatch = Rect::from_min_size(Vec2::new((slot.max.x - side).round(), (slot.center().y - side * 0.5).round()), Vec2::splat(side));
    let home = ui.cursor();
    ui.push_id(label);
    let id = ui.id("");
    ui.space(swatch.min.y - home.y);
    let mut changed = false;
    ui.indent(swatch.min.x - home.x, |ui| changed = ui.color_picker("", &mut color));
    ui.space(home.y - ui.cursor().y);
    ui.pop_id();
    if changed {
        *hex = color.with_alpha(1.0).to_hex_string();
    }

    // Its swatch is painted over with the chip. The picker, when open,
    // is on the layer above and stays.
    let open = *ui.state.open(id);
    let hot = open || (ui.state.pointer_in_window && swatch.contains(ui.state.pointer));
    let (c, r) = (swatch.center(), side * 0.5);
    ui.draw.rect(swatch.expand(m.px(1.0)), look::CARD);
    ui.draw.circle(c, r, if hot { look::TEXT } else { look::TRACK });
    ui.draw.circle(c, r - m.px(3.0), look::CARD);
    ui.draw.circle(c, r - m.px(6.0), color);
    ui.focus_ring(id, swatch);

    let mono = ui.mono_style();
    let text = Rect::new(slot.min, Vec2::new(swatch.min.x - m.px(15.0), slot.max.y));
    ui.text_right(&color.to_hex_string().to_uppercase(), &mono, text, look::TEXT_DIM);
    changed
}

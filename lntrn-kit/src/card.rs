//! Cards: a rounded panel holding a stack of rows. A row is a label (and
//! maybe a hint under it) on the left and one control on the right; when
//! the card is too narrow for both, the control goes under the text.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{FILL, TextResponse, Ui};

use crate::layout::{Keep, elide, small_style};
use crate::{chart, controls, look, pickers, probe};

/// The least a row is tall, and the room inside a card's left and right.
const ROW_H: f64 = 70.0;
pub const ROW_PAD: f64 = 25.0;
const CARD_RADIUS: f64 = 15.0;
/// The least width a hint is wrapped in beside a control; with less, the
/// row's text goes above the control instead.
const HINT_MIN_W: f64 = 260.0;
/// A text row's field.
const FIELD_W: f64 = 440.0;
/// A meter row's bar.
const METER_W: f64 = 240.0;

/// A card being filled. Rows go in through its methods, which know
/// whether a line belongs above them.
pub struct Card<'a, 'u> {
    pub ui: &'a mut Ui<'u>,
    rows: usize,
}

/// A rounded panel around the rows `f` declares. Its height is only known
/// once they are laid out, so the panel is drawn at the height it came to
/// last time, and a change asks for another pass before anything shows.
pub fn card(ui: &mut Ui, id: &str, f: impl FnOnce(&mut Card)) {
    ui.push_id(id);
    let m = ui.m;
    let slot = ui.id("card-height");
    let (top, w) = (ui.cursor(), ui.avail_width());
    let last = ui.state.floats(slot, [0.0; 4])[0];
    if last > 0.0 {
        let rect = Rect::from_min_size(top, Vec2::new(w, last));
        let r = m.px(CARD_RADIUS);
        ui.draw.rounded_rect(rect, r, look::CARD);
        ui.draw.stroke_rect(rect, m.px(1.0), r, look::LINE);
    }
    let mut card = Card { ui, rows: 0 };
    f(&mut card);
    let ui = card.ui;
    // Every row leaves a gap after it; the card ends at its last row.
    let h = (ui.cursor().y - m.gap - top.y).max(0.0);
    if (h - last).abs() > 0.5 {
        ui.state.floats(slot, [0.0; 4])[0] = h;
        ui.state.request_rebuild = true;
    }
    ui.space(m.px(10.0) - m.gap);
    ui.pop_id();
}

/// Where a row's parts go: the whole row, the control's slot at its
/// right, and the top-left and width of its text.
struct Laid {
    row: Rect,
    slot: Rect,
    text: Vec2,
    text_w: f64,
}

impl Card<'_, '_> {
    /// Reserve the next row, `h` tall, with a line above it unless it is
    /// the card's first.
    fn next(&mut self, h: f64) -> Rect {
        let m = self.ui.m;
        let rect = self.ui.alloc(Vec2::new(FILL, h));
        if self.rows > 0 {
            let pad = m.px(ROW_PAD);
            let y = (rect.min.y - (m.gap + m.px(1.0)) * 0.5).round();
            self.ui.draw.hline(rect.min.x + pad, rect.max.x - pad, y, m.px(1.0), look::LINE);
        }
        self.rows += 1;
        rect
    }

    /// Reserve a row for `label`, `hint` and a control `control_w` wide and
    /// `control_h` tall. The text sits beside the control when it fits
    /// there, and above it when the card is too narrow for both.
    fn lay(&mut self, label: &str, hint: &str, control_w: f64, control_h: f64) -> Laid {
        let m = self.ui.m;
        let pad = m.px(ROW_PAD);
        let style = self.ui.text_style();
        let small = small_style(self.ui);
        let label_h = style.line_height() as f64;
        let label_w = self.ui.measure(label, &style);
        let inner = (self.ui.avail_width() - pad * 2.0).max(1.0);
        let beside = inner - pad - control_w;
        // A hint needs room to wrap in, not just the label's width.
        let need = if hint.is_empty() { label_w } else { label_w.max(m.px(HINT_MIN_W)) };
        let stacked = beside < need;
        let text_w = if stacked { inner } else { beside };
        let hint_h = if hint.is_empty() { 0.0 } else { self.ui.text.measure_wrapped(hint, &small, text_w as f32).height as f64 };
        let text_h = label_h + hint_h;
        if label_w > text_w {
            probe::clipped(label);
        }
        if stacked {
            probe::stacked(label);
            let (edge, between) = (m.px(15.0), m.px(8.0));
            let control_h = control_h.max(m.px(56.0));
            let row = self.next(edge + text_h + between + control_h + edge);
            let top = row.min.y + edge + text_h + between;
            let slot = Rect::new(Vec2::new((row.max.x - pad - control_w).max(row.min.x + pad), top), Vec2::new(row.max.x - pad, top + control_h));
            return Laid { row, slot, text: Vec2::new(row.min.x + pad, row.min.y + edge), text_w };
        }
        let h = (text_h + m.px(30.0)).max(control_h + m.px(40.0)).max(m.px(ROW_H));
        let row = self.next(h);
        let top = (row.center().y - text_h * 0.5).round();
        let slot = Rect::new(Vec2::new(row.max.x - pad - control_w, row.min.y), Vec2::new(row.max.x - pad, row.max.y));
        Laid { row, slot, text: Vec2::new(row.min.x + pad, top), text_w }
    }

    /// Write a row's label and the hint under it.
    fn write(&mut self, at: &Laid, label: &str, hint: &str) {
        let ui = &mut *self.ui;
        let style = ui.text_style();
        let label_h = style.line_height() as f64;
        ui.text_in_rect(label, &style, Rect::from_min_size(at.text, Vec2::new(at.text_w, label_h)), look::TEXT);
        if !hint.is_empty() {
            let small = small_style(ui);
            ui.text_at(hint, &small, Vec2::new(at.text.x, at.text.y + label_h), at.text_w, look::TEXT_DIM);
        }
    }

    /// A row: `label` (with `hint` under it, when there is one) on the
    /// left, and a slot `control_w` wide on the right for `f` to draw its
    /// control in.
    pub fn row<R>(&mut self, label: &str, hint: &str, control_w: f64, f: impl FnOnce(&mut Ui, Rect) -> R) -> R {
        self.row_tall(label, hint, control_w, 0.0, f)
    }

    /// [`Self::row`] for a control `control_h` tall: bigger than a line.
    pub fn row_tall<R>(&mut self, label: &str, hint: &str, control_w: f64, control_h: f64, f: impl FnOnce(&mut Ui, Rect) -> R) -> R {
        let at = self.lay(label, hint, control_w, control_h);
        self.write(&at, label, hint);
        f(self.ui, at.slot)
    }

    /// A row with no label, `h` tall: all of it is `f`'s.
    pub fn block<R>(&mut self, h: f64, f: impl FnOnce(&mut Ui, Rect) -> R) -> R {
        let rect = self.next(h);
        f(self.ui, rect)
    }

    /// An on/off row. The whole row is the click target.
    pub fn switch(&mut self, label: &str, hint: &str, value: &mut bool) -> bool {
        let w = self.ui.m.px(controls::SWITCH_W);
        let at = self.lay(label, hint, w, 0.0);
        let id = self.ui.id(label);
        let changed = controls::switch(self.ui, id, at.row, at.slot, value);
        self.write(&at, label, hint);
        changed
    }

    /// A switch that stands for "this string is set": on seeds it with
    /// `seed`, off clears it.
    pub fn switch_string(&mut self, label: &str, hint: &str, value: &mut String, seed: &str) -> bool {
        let mut on = !value.is_empty();
        let changed = self.switch(label, hint, &mut on);
        if changed {
            *value = if on { seed.to_owned() } else { String::new() };
        }
        changed
    }

    /// A slider over `range` in `step`s, its value written by `show`.
    pub fn slider(&mut self, label: &str, hint: &str, value: &mut f64, range: (f64, f64), step: f64, show: impl Fn(f64) -> String) -> bool {
        let id = self.ui.id(label);
        let w = self.ui.m.px(controls::SLIDER_W);
        self.row(label, hint, w, |ui, slot| controls::slider(ui, id, slot, value, range, step, &show))
    }

    /// [`Self::slider`] over whole numbers.
    pub fn slider_int(&mut self, label: &str, hint: &str, value: &mut i64, range: (i64, i64), step: i64, show: impl Fn(f64) -> String) -> bool {
        let mut v = *value as f64;
        let changed = self.slider(label, hint, &mut v, (range.0 as f64, range.1 as f64), step as f64, show);
        if changed {
            *value = v.round() as i64;
        }
        changed
    }

    /// One of a few `options`, all showing.
    pub fn segmented(&mut self, label: &str, hint: &str, selected: &mut usize, options: &[&str]) -> bool {
        let id = self.ui.id(label);
        let w = controls::segmented_width(self.ui, options);
        self.row(label, hint, w, |ui, slot| controls::segmented(ui, id, slot, selected, options))
    }

    /// [`Self::segmented`] over `(stored word, shown label)` pairs. A
    /// stored word not in the list shows the first option.
    pub fn choice(&mut self, label: &str, hint: &str, value: &mut String, options: &[(&str, &str)]) -> bool {
        let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
        let mut index = options.iter().position(|o| o.0 == value.as_str()).unwrap_or(0);
        let changed = self.segmented(label, hint, &mut index, &labels);
        if changed {
            *value = options[index].0.to_owned();
        }
        changed
    }

    /// One of many `options`, in a list that drops down.
    pub fn dropdown(&mut self, label: &str, hint: &str, selected: &mut usize, options: &[&str]) -> bool {
        let id = self.ui.id(label);
        let w = self.ui.m.px(pickers::DROPDOWN_W);
        self.row(label, hint, w, |ui, slot| pickers::dropdown(ui, id, slot, selected, options))
    }

    /// A colour kept as a `#RRGGBB` string; an empty or bad one shows as
    /// `fallback`.
    pub fn color(&mut self, label: &str, hint: &str, hex: &mut String, fallback: Color) -> bool {
        let w = self.ui.m.px(pickers::COLOR_W);
        self.row(label, hint, w, |ui, slot| pickers::color(ui, label, slot, hex, fallback))
    }

    /// A row with a one-line text field, showing `placeholder` dimly while
    /// it is empty.
    pub fn text(&mut self, label: &str, hint: &str, value: &mut String, placeholder: &str) -> TextResponse {
        let id = self.ui.id(label);
        let w = self.ui.m.px(FIELD_W);
        self.row(label, hint, w, |ui, slot| {
            let h = ui.m.px(controls::BUTTON_H);
            let rect = Rect::from_min_size(Vec2::new(slot.min.x, (slot.center().y - h * 0.5).round()), Vec2::new(slot.width(), h));
            controls::text_field(ui, id, rect, value, placeholder)
        })
    }

    /// A row that only says something: `value` at its right, cut short
    /// with an ellipsis when the card is too narrow for all of it.
    pub fn value(&mut self, label: &str, hint: &str, value: &str) {
        let style = self.ui.text_style();
        let w = self.ui.measure(value, &style) + self.ui.m.px(2.0);
        self.row(label, hint, w, |ui, slot| {
            let shown = elide(ui, value, &style, slot.width(), Keep::Start);
            ui.text_right(&shown, &style, slot, look::TEXT_DIM);
        });
    }

    /// A row that says how full something is: `text` (the reading, as
    /// words), then a meter filled with `color` to `frac` (0 to 1). The
    /// meters of a card's rows line up, whatever their readings say.
    pub fn meter(&mut self, label: &str, hint: &str, frac: f64, text: &str, color: Color) {
        let m = self.ui.m;
        let id = self.ui.id(label);
        let style = self.ui.text_style();
        let (bar_w, between) = (m.px(METER_W), m.px(18.0));
        let w = bar_w + between + self.ui.measure(text, &style);
        self.row(label, hint, w, |ui, slot| {
            let (bar, words) = slot.take_right(bar_w.min(slot.width()));
            chart::meter(ui, id, bar, frac, color);
            ui.text_right(text, &style, Rect::new(words.min, Vec2::new(words.max.x - between, words.max.y)), look::TEXT_DIM);
        });
    }

    /// A row that does something: `text` on a button. `true` when pressed.
    pub fn button(&mut self, label: &str, hint: &str, text: &str) -> bool {
        let id = self.ui.id(label);
        let w = controls::button_width(self.ui, text);
        self.row(label, hint, w, |ui, slot| {
            let h = ui.m.px(controls::BUTTON_H);
            let rect = Rect::from_min_size(Vec2::new(slot.min.x, (slot.center().y - h * 0.5).round()), Vec2::new(slot.width(), h));
            controls::button(ui, id, rect, text)
        })
    }
}

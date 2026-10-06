//! The row of formatting controls over the page: the paragraph's style,
//! the font and its size, the four ways a run is marked, its colours,
//! the lists, the alignment, and a picture to put in. Each shows how the
//! selection stands and changes it when pressed. When the window is too
//! narrow for them all in a row, the groups that don't fit go under.

use lntrn_kit::{look, pickers};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{CursorIcon, FILL, IconFn, Sense, Ui, WidgetId};

use crate::doc::{Align, Font, List, Style};
use crate::editor::Editor;
use crate::editor_ops::Toggle;

/// A control's height, a square button's side, and the room between
/// groups, in logical pixels.
const H: f64 = 44.0;
const GAP: f64 = 4.0;
const GROUP_GAP: f64 = 10.0;
/// The style list's, the font list's and the size stepper's widths.
const WIDE: [f64; 3] = [140.0, 178.0, 132.0];
/// The sizes the size list offers.
pub const SIZES: [f32; 14] = [12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 32.0, 40.0, 48.0, 64.0, 80.0, 96.0];

/// Which colour a swatch menu is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swatch {
    Text,
    Highlight,
}

#[derive(Default)]
pub struct ToolOut {
    /// A colour button was pressed: which, and where its menu goes.
    pub swatches: Option<(Swatch, Vec2)>,
    /// A picture is asked for.
    pub picture: bool,
    /// Something was changed: the page should take the keyboard back.
    pub acted: bool,
}

/// A point at `(x, y)` of the rect's half-size from its centre.
fn at(rect: Rect, x: f64, y: f64) -> Vec2 {
    let s = rect.width().min(rect.height()) * 0.5;
    rect.center() + Vec2::new(x * s, y * s)
}

const BULLETS: IconFn = |d, r, c, w| {
    for y in [-0.6, 0.0, 0.6] {
        d.circle(at(r, -0.8, y), w * 0.9, c);
        d.line(at(r, -0.35, y), at(r, 0.95, y), w, c);
    }
};
const NUMBERS: IconFn = |d, r, c, w| {
    for y in [-0.6, 0.0, 0.6] {
        d.line(at(r, -0.95, y - 0.16), at(r, -0.95, y + 0.16), w, c);
        d.line(at(r, -0.35, y), at(r, 0.95, y), w, c);
    }
};
const CHECKS: IconFn = |d, r, c, w| {
    d.stroke_rect(Rect::new(at(r, -0.95, -0.75), at(r, -0.25, -0.05)), w, 0.0, c);
    d.polyline(&[at(r, -0.95, 0.42), at(r, -0.7, 0.7), at(r, -0.2, 0.15)], w, c, false);
    d.line(at(r, 0.1, -0.4), at(r, 0.95, -0.4), w, c);
    d.line(at(r, 0.1, 0.45), at(r, 0.95, 0.45), w, c);
};
const PICTURE: IconFn = |d, r, c, w| {
    d.stroke_rect(Rect::new(at(r, -0.95, -0.75), at(r, 0.95, 0.75)), w, w, c);
    d.circle(at(r, -0.4, -0.25), w * 1.2, c);
    d.polyline(&[at(r, -0.7, 0.5), at(r, -0.1, -0.05), at(r, 0.25, 0.3), at(r, 0.5, 0.1), at(r, 0.75, 0.5)], w, c, false);
};

/// Lines as a paragraph aligned this way has them.
fn align_icon(align: Align) -> IconFn {
    match align {
        Align::Left => |d, r, c, w| [(-0.75, 0.9), (-0.25, 0.2), (0.25, 0.9), (0.75, 0.4)].into_iter().for_each(|(y, x1)| d.line(at(r, -0.9, y), at(r, x1, y), w, c)),
        Align::Center => |d, r, c, w| [(-0.75, 0.9), (-0.25, 0.5), (0.25, 0.9), (0.75, 0.6)].into_iter().for_each(|(y, x)| d.line(at(r, -x, y), at(r, x, y), w, c)),
        Align::Right => |d, r, c, w| [(-0.75, -0.9), (-0.25, -0.2), (0.25, -0.9), (0.75, -0.4)].into_iter().for_each(|(y, x0)| d.line(at(r, x0, y), at(r, 0.9, y), w, c)),
        // (No button of its own: it is drawn should it ever get one.)
        Align::Justify => |d, r, c, w| [-0.75, -0.25, 0.25, 0.75].into_iter().for_each(|y| d.line(at(r, -0.9, y), at(r, 0.9, y), w, c)),
    }
}

/// What a button shows.
enum Face<'a> {
    Icon(IconFn),
    /// A letter in a style, with a bar under it of a colour.
    Letter(&'a str, TextStyle, Option<Color>),
}

/// A square button, lit when what it stands for is on. `true` when
/// pressed.
fn button(ui: &mut Ui, id: WidgetId, rect: Rect, face: Face, on: bool, tip: &str) -> bool {
    let m = ui.m;
    let r = ui.interact(id, rect, Sense::CLICK);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let accent = ui.theme.accent;
    let radius = m.px(10.0);
    if on {
        ui.draw.rounded_rect(rect, radius, accent.fade(0.22));
        ui.draw.stroke_rect(rect, m.px(1.5), radius, accent);
    } else if r.hovered || r.held {
        ui.draw.rounded_rect(rect, radius, Color::WHITE.fade(if r.held { 0.12 } else { 0.07 }));
    }
    let ink = if on { accent.lerp(Color::WHITE, 0.25) } else { look::TEXT };
    match face {
        Face::Icon(icon) => icon(ui.draw, Rect::from_center_size(rect.center(), Vec2::splat(rect.height() * 0.42)), ink, m.px(2.2)),
        Face::Letter(letter, style, bar) => {
            let up = if bar.is_some() { m.px(3.0) } else { 0.0 };
            ui.text_centered(letter, &style, Rect::new(rect.min - Vec2::new(0.0, up), rect.max - Vec2::new(0.0, up)), ink);
            if let Some(color) = bar {
                let y = rect.max.y - m.px(9.0);
                ui.draw.rounded_rect(Rect::new(Vec2::new(rect.min.x + m.px(9.0), y), Vec2::new(rect.max.x - m.px(9.0), y + m.px(5.0))), m.px(2.0), color);
            }
        }
    }
    ui.tooltip(&r, tip);
    r.clicked
}

/// Draw the controls and do what is pressed. `fonts` are the families
/// there are; `body` is body text's size.
pub fn toolbar(ui: &mut Ui, ed: &mut Editor, fonts: &[String], body: f32) -> ToolOut {
    let m = ui.m;
    let (h, gap, between) = (m.px(H), m.px(GAP), m.px(GROUP_GAP));
    let run = ed.format_state();
    let para = ed.para_state();
    let mut out = ToolOut::default();
    let width = ui.avail_width();
    // The groups' widths; how many rows they take decides the height.
    let wide = WIDE.map(|w| m.px(w));
    let groups = [wide[0] + gap + wide[1] + gap + wide[2], h * 4.0 + gap * 3.0, h * 2.0 + gap, h * 3.0 + gap * 2.0, h * 3.0 + gap * 2.0, h];
    let mut places = [Vec2::ZERO; 6];
    let (mut x, mut row) = (0.0, 0.0);
    for (place, wide) in places.iter_mut().zip(groups) {
        if x > 0.0 && x + wide > width {
            (x, row) = (0.0, row + 1.0);
        }
        *place = Vec2::new(x, row * (h + gap));
        x += wide + between;
    }
    let area = ui.alloc(Vec2::new(FILL, (row + 1.0) * (h + gap) - gap));
    let id = ui.id("tools");
    let slot = |group: usize, x: f64, w: f64| Rect::from_min_size(area.min + places[group] + Vec2::new(x, 0.0), Vec2::new(w, h));
    let square = |group: usize, i: usize| slot(group, i as f64 * (h + gap), h);
    let style = ui.text_style();

    // ---- style, font, size ----
    let mut picked = Style::ALL.iter().position(|s| *s == para.style).unwrap_or(0);
    if pickers::dropdown(ui, id.with("style"), slot(0, 0.0, wide[0]), &mut picked, &Style::ALL.map(Style::label)) {
        ed.set_style(Style::ALL[picked]);
        out.acted = true;
    }
    let mut names: Vec<&str> = vec!["Default"];
    names.extend(fonts.iter().map(String::as_str));
    let mut picked = run.font.and_then(|f| names.iter().position(|n| *n == f.name())).unwrap_or(0);
    if pickers::dropdown(ui, id.with("font"), slot(0, wide[0] + gap, wide[1]), &mut picked, &names) {
        let font = (picked > 0).then(|| Font::named(names[picked]));
        ed.format(|a| a.font = font);
        out.acted = true;
    }
    let size = run.size.unwrap_or(body * para.style.scale());
    let sized = slot(0, wide[0] + wide[1] + gap * 2.0, wide[2]);
    let part = sized.width() / 3.0;
    let third = |i: usize| Rect::from_min_size(Vec2::new(sized.min.x + part * i as f64, sized.min.y), Vec2::new(part, h));
    ui.draw.rounded_rect(sized, m.px(12.0), look::BUTTON);
    ui.draw.stroke_rect(sized, m.px(1.0), m.px(12.0), look::TRACK);
    let mut new_size = None;
    for (i, (name, sign, by)) in [("smaller", "\u{2212}", -2.0), ("bigger", "+", 2.0)].into_iter().enumerate() {
        if button(ui, id.with(name), third(i * 2), Face::Letter(sign, style.clone(), None), false, if by < 0.0 { "Smaller text" } else { "Bigger text" }) {
            new_size = Some((size + by).clamp(6.0, 200.0));
        }
    }
    let sizes = id.with("size");
    let value = ui.interact(sizes, third(1), Sense::CLICK);
    if value.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    ui.text_centered(&format!("{}", size.round()), &style, third(1), if value.hovered { ui.theme.accent } else { look::TEXT });
    ui.tooltip(&value, "Text size");
    if value.clicked {
        *ui.state.open(sizes) = !*ui.state.open(sizes);
        ui.state.request_rebuild = true;
    }
    if *ui.state.open(sizes) {
        let labels = SIZES.map(|s| format!("{s}"));
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let res = ui.popup_list(sizes, sized, &refs, SIZES.iter().position(|s| *s == size.round()));
        new_size = res.picked.map(|i| SIZES[i]).or(new_size);
        if res.picked.is_some() || res.closed {
            *ui.state.open(sizes) = false;
        }
    }
    if let Some(size) = new_size {
        ed.format(|a| a.size = Some(size));
        out.acted = true;
    }

    // ---- bold, italic, underline, strike ----
    let big = TextStyle::new(style.size * 1.1);
    let marks = [(Toggle::Bold, "B", big.clone().bold(), "Bold  Ctrl+B"), (Toggle::Italic, "I", big.clone().italic(), "Italic  Ctrl+I"), (Toggle::Underline, "U", big.clone(), "Underline  Ctrl+U"), (Toggle::Strike, "S", big.clone(), "Strike through  Ctrl+Shift+X")];
    for (i, (which, letter, letter_style, tip)) in marks.into_iter().enumerate() {
        let rect = square(1, i);
        if button(ui, id.with(tip), rect, Face::Letter(letter, letter_style, None), which.get(&run), tip) {
            ed.toggle(which);
            out.acted = true;
        }
        // The U is underlined and the S struck, as their text would be.
        let (c, half, w) = (rect.center(), m.px(8.0), m.px(1.8));
        match which {
            Toggle::Underline => ui.draw.line(c + Vec2::new(-half, m.px(13.0)), c + Vec2::new(half, m.px(13.0)), w, look::TEXT),
            Toggle::Strike => ui.draw.line(c + Vec2::new(-half, m.px(1.0)), c + Vec2::new(half, m.px(1.0)), w, look::TEXT),
            _ => {}
        }
    }

    // ---- colours ----
    for (i, (which, letter, color, tip)) in [(Swatch::Text, "A", run.color.map_or(look::TEXT, Color::hex), "Text colour"), (Swatch::Highlight, "\u{25A7}", run.highlight.map_or(look::TRACK, Color::hex), "Highlight")].into_iter().enumerate() {
        let rect = square(2, i);
        if button(ui, id.with(tip), rect, Face::Letter(letter, big.clone().bold(), Some(color)), false, tip) {
            out.swatches = Some((which, Vec2::new(rect.min.x, rect.max.y + gap)));
        }
    }

    // ---- lists ----
    for (i, (kind, icon, tip)) in [(List::Bullet, BULLETS, "Bullets"), (List::Number, NUMBERS, "Numbered list"), (List::Check(false), CHECKS, "Checklist")].into_iter().enumerate() {
        if button(ui, id.with(tip), square(3, i), Face::Icon(icon), para.list.same_kind(kind), tip) {
            ed.set_list(kind);
            out.acted = true;
        }
    }

    // ---- alignment (justified is in the Format menu, and on Ctrl+J) ----
    for (i, (align, tip)) in [(Align::Left, "Align left  Ctrl+L"), (Align::Center, "Centre  Ctrl+E"), (Align::Right, "Align right  Ctrl+R")].into_iter().enumerate() {
        if button(ui, id.with(tip), square(4, i), Face::Icon(align_icon(align)), para.align == align, tip) {
            ed.set_align(align);
            out.acted = true;
        }
    }

    // ---- a picture ----
    out.picture = button(ui, id.with("picture"), square(5, 0), Face::Icon(PICTURE), false, "Insert a picture");
    ui.space(gap);
    out
}

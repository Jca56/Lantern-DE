//! Painting the picker: the shade over all that isn't chosen, the chosen
//! area's frame, handles and size, the hint, and the toolbar. Sizes here
//! are logical pixels, big on purpose.

use lntrn_kit::controls::{self, Kind, SWITCH_W};
use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

use super::area::Edge;
use crate::audio::Sources;

/// How far from a handle it can still be taken.
pub const REACH: f64 = 26.0;
const HANDLE: f64 = 20.0;
const LINE: f64 = 3.0;
const BAR_PAD: f64 = 14.0;
const BAR_GAP: f64 = 14.0;
/// From the toolbar to the monitor's bottom edge, and the least room
/// beside it.
const BAR_BELOW: f64 = 64.0;
const BAR_BESIDE: f64 = 48.0;
const BAR_RADIUS: f64 = 18.0;
const SHADE: f64 = 0.45;

pub const MODES: [&str; 2] = ["Area", "Window"];

/// A press on the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    WholeScreen,
    Record,
    Cancel,
}

/// Where the toolbar and its parts go on a monitor: centred near its
/// bottom edge, in one row, or two on a narrow monitor. Known before the
/// frame's input, so a press on it never starts an area.
pub struct Bar {
    pub panel: Rect,
    modes: Rect,
    whole: Rect,
    desktop: Rect,
    mic: Rect,
    record: Rect,
    cancel: Rect,
}

const DESKTOP: &str = "Desktop audio";
const MIC: &str = "Mic";

impl Bar {
    pub fn layout(ui: &mut Ui, surface: Rect) -> Bar {
        let m = ui.m;
        let (pad, gap, h) = (m.px(BAR_PAD), m.px(BAR_GAP), m.widget_h);
        let style = ui.text_style();
        let mut toggle = |label: &str| ui.measure(label, &style) + m.px(SWITCH_W) + m.px(44.0);
        let (desktop, mic) = (toggle(DESKTOP), toggle(MIC));
        let widths = [controls::segmented_width(ui, &MODES), controls::button_width(ui, "Whole Screen"), desktop, mic, controls::button_width(ui, "Record"), controls::button_width(ui, "Cancel")];
        let span = |row: &[f64]| row.iter().sum::<f64>() + gap * (row.len() as f64 - 1.0);
        // Picking on one row and the rest on another, when one won't fit.
        let room = surface.width() - m.px(BAR_BESIDE) * 2.0 - pad * 2.0;
        let rows: Vec<&[f64]> = if span(&widths) <= room { vec![&widths[..]] } else { vec![&widths[..2], &widths[2..]] };
        let inner = rows.iter().map(|r| span(r)).fold(0.0, f64::max);
        let size = Vec2::new(inner + pad * 2.0, h * rows.len() as f64 + gap * (rows.len() as f64 - 1.0) + pad * 2.0);
        let min = Vec2::new((surface.center().x - size.x * 0.5).round(), (surface.max.y - m.px(BAR_BELOW) - size.y).round());
        let panel = Rect::from_min_size(min, size);
        let mut rects = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let y = panel.min.y + pad + (h + gap) * i as f64;
            let mut x = (panel.center().x - span(row) * 0.5).round();
            for &w in row.iter() {
                rects.push(Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, h)));
                x += w + gap;
            }
        }
        Bar { panel, modes: rects[0], whole: rects[1], desktop: rects[2], mic: rects[3], record: rects[4], cancel: rects[5] }
    }
}

/// The toolbar: Area or Window, Whole Screen, the sound to record, Record
/// (once there's something to record), Cancel.
pub fn bar(ui: &mut Ui, bar: &Bar, mode: &mut usize, sound: &mut Sources, can_record: bool) -> Option<Press> {
    let m = ui.m;
    let radius = m.px(BAR_RADIUS);
    ui.draw.shadow(bar.panel.translate(Vec2::new(0.0, m.px(4.0))), radius, m.px(16.0), Color::BLACK.fade(0.5));
    ui.draw.rounded_rect(bar.panel, radius, look::card());
    ui.draw.stroke_rect(bar.panel, m.px(1.0), radius, look::line());
    let id = ui.id("modes");
    controls::segmented(ui, id, bar.modes, mode, &MODES);
    toggle(ui, bar.desktop, DESKTOP, &mut sound.desktop);
    toggle(ui, bar.mic, MIC, &mut sound.mic);
    let mut press = None;
    let buttons = [(bar.whole, "Whole Screen", Kind::Plain, true, Press::WholeScreen), (bar.record, "Record", Kind::Primary, can_record, Press::Record), (bar.cancel, "Cancel", Kind::Plain, true, Press::Cancel)];
    for (rect, text, kind, enabled, what) in buttons {
        let id = ui.id(text);
        if controls::button_of(ui, id, rect, text, kind, enabled) {
            press = Some(what);
        }
    }
    press
}

/// A switch with its label, in a well of its own; a click anywhere on it
/// flips it.
fn toggle(ui: &mut Ui, rect: Rect, label: &str, value: &mut bool) {
    let m = ui.m;
    ui.draw.rounded_rect(rect, m.px(12.0), look::well());
    ui.draw.stroke_rect(rect, m.px(1.0), m.px(12.0), look::line());
    let id = ui.id(label);
    let slot = Rect::new(rect.min, Vec2::new(rect.max.x - m.px(12.0), rect.max.y));
    controls::switch(ui, id, rect, slot, value);
    let style = ui.text_style();
    let text = Rect::new(Vec2::new(rect.min.x + m.px(16.0), rect.min.y), Vec2::new(slot.max.x - m.px(SWITCH_W), rect.max.y));
    ui.text_in_rect(label, &style, text, look::text());
}

/// Darken all of `surface` but `hole`.
pub fn shade(ui: &mut Ui, surface: Rect, hole: Option<Rect>) {
    let dim = Color::BLACK.fade(SHADE);
    let Some(h) = hole.map(|h| h.intersection(&surface)).filter(|h| !h.is_empty()) else {
        ui.draw.rect(surface, dim);
        return;
    };
    let around = [
        Rect::new(surface.min, Vec2::new(surface.max.x, h.min.y)),
        Rect::new(Vec2::new(surface.min.x, h.max.y), surface.max),
        Rect::new(Vec2::new(surface.min.x, h.min.y), Vec2::new(h.min.x, h.max.y)),
        Rect::new(Vec2::new(h.max.x, h.min.y), Vec2::new(surface.max.x, h.max.y)),
    ];
    for r in around.into_iter().filter(|r| r.width() > 0.0 && r.height() > 0.0) {
        ui.draw.rect(r, dim);
    }
}

/// The chosen area's frame, just outside it, and its eight handles when
/// it can be resized.
pub fn frame(ui: &mut Ui, r: Rect, handles: bool) {
    let m = ui.m;
    let accent = ui.theme.accent;
    let line = m.px(LINE);
    ui.draw.stroke_rect(r.expand(line), line, 0.0, accent);
    if !handles {
        return;
    }
    let side = m.px(HANDLE);
    for edge in Edge::ALL {
        let square = Rect::from_center_size(edge.handle(r), Vec2::new(side, side));
        ui.draw.rounded_rect(square, m.px(5.0), Color::WHITE);
        ui.draw.stroke_rect(square, m.px(2.0), m.px(5.0), accent);
    }
}

/// `text` in a pill over the top left of `r`, or just inside it when
/// there's no room above.
pub fn label(ui: &mut Ui, r: Rect, surface: Rect, text: &str) {
    let m = ui.m;
    let style = ui.text_style();
    let size = Vec2::new(ui.measure(text, &style) + m.px(28.0), f64::from(style.line_height()) + m.px(14.0));
    let gap = m.px(14.0);
    let y = if r.min.y - size.y - gap >= surface.min.y { r.min.y - size.y - gap } else { r.min.y + gap };
    let x = r.min.x.clamp(surface.min.x, (surface.max.x - size.x).max(surface.min.x));
    pill(ui, Rect::from_min_size(Vec2::new(x, y), size), text, look::text());
}

/// What to do next, in a pill over the toolbar.
pub fn hint(ui: &mut Ui, bar: Rect, text: &str) {
    let m = ui.m;
    let style = ui.text_style();
    let size = Vec2::new(ui.measure(text, &style) + m.px(40.0), f64::from(style.line_height()) + m.px(18.0));
    let at = Vec2::new((bar.center().x - size.x * 0.5).round(), bar.min.y - m.px(16.0) - size.y);
    pill(ui, Rect::from_min_size(at, size), text, look::text());
}

fn pill(ui: &mut Ui, r: Rect, text: &str, ink: Color) {
    let style = ui.text_style();
    ui.draw.rounded_rect(r, r.height() * 0.5, look::card().fade(0.94));
    ui.draw.stroke_rect(r, ui.m.px(1.0), r.height() * 0.5, look::line());
    ui.text_centered(text, &style, r, ink);
}

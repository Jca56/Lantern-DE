//! The monitors' popups, in the overlay above their faces, built of the
//! tray's popup kit: each a big graph of its load (the moment under the
//! pointer said in numbers), its readings large, the rest there is to
//! know, and a button into the System Monitor's page about it. A click
//! off the panel, or Escape, puts it away.

mod cpu;
mod gpu;
mod ram;

use lntrn_kit::chart::{self, Graph, Series};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_status::LoadState;
use lntrn_text::TextStyle;
use lntrn_ui::{Key, Sense, Ui};

use super::{Kind, Monitors, gib, percent, sold};
use crate::tray::popup::kit::{self, Column, LINE_H};
use crate::tray::popup::Frame;

/// The panel's width, logical.
const WIDTH: f64 = 460.0;
/// The graph's height, the big readings' row and their text, logical.
const GRAPH_H: f64 = 150.0;
const BIG_H: f64 = 58.0;
const BIG: f64 = 38.0;
/// Between the parts, logical.
const GAP: f64 = 10.0;
/// A reading with a meter under it, and the button at the bottom.
const METER_ROW_H: f64 = LINE_H + 22.0;
const BUTTON_H: f64 = 54.0;
/// The busiest programs' caption.
const CAPTION_H: f64 = 36.0;

/// What one frame of a popup did.
pub struct PopupFrame {
    /// Its panel; `None` when it is to go (clicked off, Escaped, or its
    /// monitor gone).
    pub panel: Option<Rect>,
    /// Its button was clicked: the System Monitor, on its page.
    pub sysmon: bool,
}

/// Draw `kind`'s popup.
pub fn draw(monitors: &Monitors, ui: &mut Ui, kind: Kind, frame: &Frame) -> PopupFrame {
    let gone = PopupFrame { panel: None, sysmon: false };
    if ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() {
        return gone;
    }
    let Some(s) = monitors.state() else { return gone };
    let slots = monitors.options().load().history;
    let mut sysmon = false;
    let panel = match kind {
        Kind::Cpu => Some(cpu::draw(ui, &s, slots, frame, &mut sysmon)),
        Kind::Gpu => s.gpu.as_ref().map(|g| gpu::draw(ui, &s, g, slots, frame, &mut sysmon)),
        Kind::Ram => Some(ram::draw(ui, &s, slots, frame, &mut sysmon)),
    };
    let Some(panel) = panel else { return gone };
    // Its own parts took their clicks; any other is off it.
    if ui.state.pressed && !panel.contains(ui.state.pointer) {
        return gone;
    }
    PopupFrame { panel: Some(panel), sysmon }
}

/// How tall the parts every popup has are, pixels: the title, the graph,
/// the big readings and the button, with the gaps between.
fn frame_h(ui: &Ui) -> f64 {
    ui.m.px(kit::HEAD_H + GRAPH_H + GAP + BIG_H + GAP * 2.0 + BUTTON_H)
}

/// The load graphed big across the column, in `kind`'s colour. Returns
/// how many looks back the moment under the pointer is.
fn graph(ui: &mut Ui, col: &mut Column, kind: Kind, values: &[f32], slots: usize) -> Option<usize> {
    let rect = col.take(ui.m.px(GRAPH_H));
    col.take(ui.m.px(GAP));
    let g = Graph { series: &[Series::area(values, kind.color())], max: 100.0, slots, top: "100%" };
    chart::graph(ui, ui.id("monitor-graph").with(kind.word()), rect, &g)
}

/// The big readings: the load (or, under the pointer, the load then and
/// how long ago) at the left, `right` at the right in its colour.
fn big(ui: &mut Ui, col: &mut Column, s: &LoadState, values: &[f32], hovered: Option<usize>, right: Option<(String, Color)>) {
    let m = ui.m;
    let row = col.take(m.px(BIG_H));
    let style = TextStyle::new(m.px(BIG) as f32).bold();
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let then = hovered.and_then(|back| values.len().checked_sub(back + 1).map(|i| (back, values[i])));
    let now = values.last().copied().unwrap_or(0.0);
    let text = percent(then.map_or(now, |(_, v)| v));
    let w = ui.measure(&text, &style);
    ui.text_in_rect(&text, &style, row, ink);
    if let Some((back, _)) = then {
        let ago = ago(back as f64 * s.options.interval);
        let rest = Rect::new(Vec2::new(row.min.x + w + m.px(14.0), row.min.y), row.max);
        ui.text_in_rect(&ago, &kit::name_style(ui), rest, dim);
    } else if let Some((text, color)) = right {
        ui.text_right(&text, &style, row, color);
    }
    col.take(m.px(GAP));
}

/// How long ago `secs` was, as said: `now`, `12 s ago`, `1 min 30 s ago`.
fn ago(secs: f64) -> String {
    let secs = secs.round() as u64;
    match (secs / 60, secs % 60) {
        (0, 0) => "now".to_owned(),
        (0, s) => format!("{s} s ago"),
        (m, 0) => format!("{m} min ago"),
        (m, s) => format!("{m} min {s} s ago"),
    }
}

/// A line: `what` at its left, `value` at its right.
fn pair(ui: &mut Ui, row: Rect, what: &str, value: &str) {
    let style = kit::name_style(ui);
    let value_w = ui.measure(value, &style) + ui.m.px(4.0);
    kit::line(ui, Rect::new(row.min, Vec2::new(row.max.x - value_w - ui.m.px(12.0), row.max.y)), what, &style, ui.theme.text_dim);
    ui.text_right(value, &style, row, ui.theme.text);
}

/// A reading with a meter under it: `what` and `value` on a line, the
/// meter `frac` full in `color`.
fn meter(ui: &mut Ui, col: &mut Column, id: &str, what: &str, value: &str, frac: f64, color: Color) {
    let m = ui.m;
    let row = col.take(m.px(METER_ROW_H));
    let (line, bar) = row.take_top(m.px(LINE_H));
    pair(ui, line, what, value);
    chart::meter(ui, ui.id("monitor-meter").with(id), bar, frac, color);
}

/// Used of a total, spaced for a popup: `2.8 / 32 GB`.
fn used_of(used: u64, total: u64) -> String {
    format!("{} / {} GB", gib(used), sold(total))
}

/// Bytes as a program's share is said: `1.2 GB`, `512 MB`.
fn bytes(b: u64) -> String {
    if b >= 1 << 30 { format!("{} GB", gib(b)) } else { format!("{} MB", (b + (1 << 19)) >> 20) }
}

/// How tall [`programs`] is for `rows` of them, pixels (a line saying
/// it is looking, until it has some).
fn programs_h(ui: &Ui, rows: usize) -> f64 {
    ui.m.px(CAPTION_H + LINE_H * rows.max(1) as f64)
}

/// The busiest programs under a caption, each its name and `value`.
fn programs(ui: &mut Ui, col: &mut Column, caption: &str, rows: &[(String, String)]) {
    let m = ui.m;
    let head = col.take(m.px(CAPTION_H));
    let bold = kit::name_style(ui).bold();
    ui.text_in_rect(caption, &bold, head, ui.theme.text);
    if rows.is_empty() {
        kit::line(ui, col.take(m.px(LINE_H)), "Looking…", &kit::dim_style(ui), ui.theme.text_dim);
    }
    for (name, value) in rows {
        let row = col.take(m.px(LINE_H));
        let style = kit::name_style(ui);
        let value_w = ui.measure(value, &style) + m.px(4.0);
        kit::line(ui, Rect::new(row.min, Vec2::new(row.max.x - value_w - m.px(12.0), row.max.y)), name, &style, ui.theme.text);
        ui.text_right(value, &style, row, ui.theme.text_dim);
    }
}

/// The button along the bottom: into the System Monitor. Returns whether
/// it was clicked.
fn sysmon_button(ui: &mut Ui, col: &mut Column, kind: Kind) -> bool {
    let m = ui.m;
    col.take(m.px(GAP));
    let rect = col.take(m.px(BUTTON_H));
    let r = ui.interact(ui.id("monitor-sysmon").with(kind.word()), rect, Sense::CLICK);
    let face = if r.held {
        0.16
    } else if r.hovered {
        0.12
    } else {
        0.07
    };
    ui.draw.rounded_rect(rect, m.px(12.0), Color::WHITE.fade(face));
    ui.text_centered("Open System Monitor", &kit::name_style(ui), rect, ui.theme.text);
    r.clicked
}

/// Open the frame's panel `h` pixels of parts tall, the title along its
/// top. Returns the column under the title, and the panel.
fn open(ui: &mut Ui, frame: &Frame, kind: Kind, h: f64) -> (Column, Rect) {
    let (mut col, panel) = frame.open(ui, WIDTH, h);
    kit::header(ui, col.take(ui.m.px(kit::HEAD_H)), kind.title(), None);
    (col, panel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_and_sizes_read_as_said() {
        assert_eq!(ago(0.2), "now");
        assert_eq!(ago(12.0), "12 s ago");
        assert_eq!(ago(120.0), "2 min ago");
        assert_eq!(ago(90.0), "1 min 30 s ago");
        assert_eq!(bytes(512 << 20), "512 MB");
        assert_eq!(bytes(3 << 29), "1.5 GB");
        assert_eq!(used_of(2 << 30, 12 << 30), "2.0 / 12 GB");
    }
}

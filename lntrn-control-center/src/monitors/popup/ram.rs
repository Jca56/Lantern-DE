//! The memory's popup: its load graphed and said big with how much is in
//! use, a bar of what programs hold, what the kernel keeps handy and
//! what is free, the swap, and the programs holding the most.

use lntrn_kit::{chart, look};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_status::LoadState;
use lntrn_ui::Ui;

use super::{Frame, GAP, METER_ROW_H, big, bytes, frame_h, graph, meter, open, programs, programs_h, sysmon_button, used_of};
use crate::monitors::Kind;
use crate::tray::popup::kit::LINE_H;

/// Between the legend's entries, logical.
const LEGEND_GAP: f64 = 22.0;

pub fn draw(ui: &mut Ui, s: &LoadState, slots: usize, frame: &Frame, sysmon: &mut bool) -> Rect {
    let m = ui.m;
    let mem = &s.mem;
    let blue = Kind::Ram.color();
    let parts = [("Used", mem.used(), blue), ("Cache", mem.cache(), blue.fade(0.45)), ("Free", mem.free, look::TRACK)];
    let legend: Vec<(String, Color)> = parts.iter().map(|(what, b, color)| (format!("{what} {}", bytes(*b)), *color)).collect();
    let legend_rows = rows(ui, &legend, frame_inner(ui));
    let rows: Vec<(String, String)> = s.top.as_ref().map(|t| t.memory.iter().map(|(name, b)| (name.to_string(), bytes(*b))).collect()).unwrap_or_default();
    let swap = mem.swap_total > 0;
    let h = frame_h(ui) + m.px(LINE_H) * (1 + legend_rows.len()) as f64 + if swap { m.px(METER_ROW_H) } else { 0.0 } + m.px(GAP) * 2.0 + programs_h(ui, rows.len());
    let (mut col, panel) = open(ui, frame, Kind::Ram, h);
    let hovered = graph(ui, &mut col, Kind::Ram, &mem.history, slots);
    big(ui, &mut col, s, &mem.history, hovered, Some((used_of(mem.used(), mem.total), ui.theme.text)));
    // Used, kept handy and free, end to end, and what each is under it.
    let whole = mem.total.max(1) as f64;
    let bar = col.take(m.px(LINE_H));
    chart::meter_parts(ui, bar, &[(mem.used() as f64 / whole, parts[0].2), (mem.cache() as f64 / whole, parts[1].2)]);
    for row in &legend_rows {
        let line = col.take(m.px(LINE_H));
        let mut x = line.min.x;
        for &i in row {
            let (label, color) = &legend[i];
            x += chart::legend(ui, Vec2::new(x, line.min.y), *color, label) + m.px(LEGEND_GAP);
        }
    }
    col.take(m.px(GAP));
    if swap {
        meter(ui, &mut col, "swap", "Swap", &used_of(mem.swap_used(), mem.swap_total), f64::from(mem.swap_percent()) / 100.0, blue);
    }
    col.take(m.px(GAP));
    programs(ui, &mut col, "Holding the most", &rows);
    *sysmon = sysmon_button(ui, &mut col, Kind::Ram);
    panel
}

/// The panel's inside width, pixels: what a legend row has.
fn frame_inner(ui: &Ui) -> f64 {
    ui.m.px(super::WIDTH - crate::tray::popup::kit::PAD * 2.0)
}

/// The legend's entries in rows that fit `w`, by index.
fn rows(ui: &mut Ui, legend: &[(String, Color)], w: f64) -> Vec<Vec<usize>> {
    let m = ui.m;
    let style = lntrn_kit::layout::small_style(ui);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0.0;
    for (i, (label, _)) in legend.iter().enumerate() {
        // The dot, its gap, the words.
        let entry = m.px(20.0) + ui.measure(label, &style);
        if x > 0.0 && x + entry > w {
            rows.push(Vec::new());
            x = 0.0;
        }
        if let Some(row) = rows.last_mut() {
            row.push(i);
        }
        x += entry + m.px(LEGEND_GAP);
    }
    rows
}

//! The processor: how busy it has been, how busy each of its threads is
//! now, and how it is clocked.

use lntrn_kit::chart::{self, Graph, Series};
use lntrn_kit::{caption, card, look, small_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::Ui;

use super::{at, graph, span};
use crate::format;
use crate::sample::Frame;

/// The least room a thread's meter gets in the grid, and how tall its
/// row is.
const CELL_W: f64 = 215.0;
const CELL_H: f64 = 40.0;

pub fn blurb(frame: &Frame) -> String {
    let f = &frame.facts;
    let name = if f.cpu.is_empty() { "The processor" } else { &f.cpu };
    match (f.cores, f.threads) {
        (0, t) => format!("{name}, {t} threads."),
        (c, t) if c == t => format!("{name}, {c} cores."),
        (c, t) => format!("{name}, {c} cores and {t} threads."),
    }
}

pub fn draw(frame: &Frame, ui: &mut Ui) {
    let cpu = &frame.cpu;
    let accent = ui.theme.accent;
    let percent = |v: f64| format::percent(v, false);
    let g = Graph { series: &[Series::area(&cpu.history, accent)], max: 100.0, slots: crate::sample::HISTORY, top: "100%" };
    graph(ui, "usage", &format!("In use, {}", span(frame)), &percent(f64::from(cpu.usage)), frame, &g, |back| percent(at(&cpu.history, back)));

    caption(ui, "Each thread");
    card(ui, "threads", |c| {
        let m = c.ui.m;
        let pad = m.px(lntrn_kit::card::ROW_PAD);
        let inner = (c.ui.avail_width() - pad * 2.0).max(1.0);
        let columns = ((inner / m.px(CELL_W)).floor() as usize).clamp(1, 4);
        let lines = cpu.threads.len().div_ceil(columns);
        let cell_h = m.px(CELL_H);
        let edge = m.px(14.0);
        c.block(lines as f64 * cell_h + edge * 2.0, |ui, rect| {
            let small = small_style(ui);
            let cell_w = inner / columns as f64;
            let (number_w, value_w, gap) = (ui.measure(&cpu.threads.last().map_or(0, |t| t.index).to_string(), &small) + m.px(12.0), ui.measure("100%", &small) + m.px(10.0), m.px(22.0));
            ui.push_id("thread");
            // Down each column in turn, so neighbours in number are
            // neighbours on the page.
            for (i, t) in cpu.threads.iter().enumerate() {
                let (col, line) = (i / lines, i % lines);
                let cell = Rect::from_min_size(Vec2::new(rect.min.x + pad + col as f64 * cell_w, rect.min.y + edge + line as f64 * cell_h), Vec2::new(cell_w - gap, cell_h));
                let (number, rest) = cell.take_left(number_w);
                let (value, bar) = rest.take_right(value_w);
                ui.text_in_rect(&t.index.to_string(), &small, number, look::TEXT_DIM);
                let frac = f64::from(t.usage) / 100.0;
                let id = ui.id("meter").with_index(t.index);
                chart::meter(ui, id, bar, frac, chart::heat(frac, accent));
                ui.text_right(&percent(f64::from(t.usage)), &small, value, look::TEXT);
            }
            ui.pop_id();
        });
    });

    caption(ui, "How it is running");
    card(ui, "running", |c| {
        if cpu.mhz > 0.0 {
            c.value("Clock", "The mean of every thread's, right now.", &format::clock(f64::from(cpu.mhz)));
        }
        if !cpu.governor.is_empty() {
            c.value("Governor", "What decides the clocks.", &cpu.governor);
        }
        let [one, five, fifteen] = frame.load;
        c.value("Load", "How many things wanted a processor at once, over one, five and fifteen minutes.", &format!("{one:.2}   {five:.2}   {fifteen:.2}"));
        c.value("Running", "", &format!("{} processes, {} threads", format::count(frame.programs() as u64), format::count(frame.threads())));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::system::Facts;

    #[test]
    fn the_blurb_names_the_processor_and_counts_what_it_can() {
        let with = |cpu: &str, cores, threads| blurb(&Frame { facts: Facts { cpu: cpu.to_owned(), cores, threads, ..Facts::default() }.into(), ..Frame::default() });
        assert_eq!(with("Intel Core i7-14700K", 20, 28), "Intel Core i7-14700K, 20 cores and 28 threads.");
        assert_eq!(with("Some ARM", 4, 4), "Some ARM, 4 cores.");
        assert_eq!(with("", 0, 2), "The processor, 2 threads.");
    }
}

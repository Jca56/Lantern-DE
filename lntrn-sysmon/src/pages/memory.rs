//! Memory: how much is in use, what the rest is doing, and swap where
//! there is any.

use lntrn_kit::chart::{self, Graph, Series};
use lntrn_kit::{caption, card, look};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

use super::{MEMORY, at, graph, span};
use crate::format;
use crate::sample::{Frame, HISTORY};

pub fn blurb(frame: &Frame) -> String {
    let mem = &frame.mem;
    if mem.swap_total > 0 { format!("{} of memory, and {} of swap behind it.", format::bytes(mem.total), format::bytes(mem.swap_total)) } else { format!("{} of memory, and no swap.", format::bytes(mem.total)) }
}

pub fn draw(frame: &Frame, ui: &mut Ui) {
    let mem = &frame.mem;
    let percent = |v: f64| format::percent(v, false);
    let g = Graph { series: &[Series::area(&mem.history, MEMORY)], max: 100.0, slots: HISTORY, top: "100%" };
    graph(ui, "memory", &format!("In use, {}", span(frame)), &format::bytes_of(mem.used(), mem.total), frame, &g, |back| percent(at(&mem.history, back)));

    caption(ui, "What holds it");
    card(ui, "parts", |c| {
        let m = c.ui.m;
        let total = mem.total.max(1) as f64;
        // The bar, and under it what its three parts are.
        c.block(m.px(92.0), |ui, rect| {
            let pad = m.px(lntrn_kit::card::ROW_PAD);
            let (top, under) = rect.take_top(m.px(56.0));
            let bar = Rect::new(Vec2::new(top.min.x + pad, top.min.y + m.px(8.0)), Vec2::new(top.max.x - pad, top.max.y));
            chart::meter_parts(ui, bar, &[(mem.used() as f64 / total, MEMORY), (mem.cache() as f64 / total, cache_ink())]);
            let mut x = rect.min.x + pad;
            for (color, label) in [(MEMORY, "In use"), (cache_ink(), "Kept handy"), (look::TRACK, "Free")] {
                x += chart::legend(ui, Vec2::new(x, under.min.y), color, label) + m.px(26.0);
            }
        });
        c.value("In use", "What programs hold and would not give back.", &format::bytes(mem.used()));
        c.value("Kept handy", "Files read lately. Given back the moment a program wants the room.", &format::bytes(mem.cache()));
        c.value("Free", "Holding nothing at all.", &format::bytes(mem.free));
        if mem.shared > 0 {
            c.value("Shared", "Held between programs, and in files that live in memory.", &format::bytes(mem.shared));
        }
    });

    if mem.swap_total > 0 {
        let g = Graph { series: &[Series::area(&mem.swap_history, look::WARN)], max: 100.0, slots: HISTORY, top: "100%" };
        graph(ui, "swap", &format!("Swap in use, {}", span(frame)), &format::bytes_of(mem.swap_used(), mem.swap_total), frame, &g, |back| percent(at(&mem.swap_history, back)));
        lntrn_kit::note(ui, "Swap is disk standing in for memory. A little in use is ordinary; a lot, while memory is full, is what makes a machine crawl.");
    }
}

/// The colour of what is only kept handy: memory's own, held back.
fn cache_ink() -> Color {
    MEMORY.lerp(look::CARD, 0.55)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    #[test]
    fn the_blurb_says_how_much_there_is_and_whether_there_is_swap() {
        assert_eq!(blurb(&fixture::frame()), "32.0 GiB of memory, and 8.0 GiB of swap behind it.");
        assert_eq!(blurb(&fixture::bare()), "4.0 GiB of memory, and no swap.");
    }
}

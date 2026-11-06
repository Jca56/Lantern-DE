//! The graphics card: its clock over the last two minutes and what holds
//! it back, then its clock offsets, the clock lock (the undervolt) and
//! its power limit.

use lntrn_kit::chart::{Graph, Series};
use lntrn_kit::{caption, card, look, note};
use lntrn_ui::Ui;

use super::overview::holding;
use super::{SLOTS, SPAN, at, graph, reading, shown_range, top, tuning};
use crate::app::App;
use crate::format;
use crate::sample::Frame;

/// The parts of the offsets' ranges worth a slider: the cards allow far
/// more than any of them can run.
const CORE_USEFUL: (i64, i64) = (-300, 500);
const MEMORY_USEFUL: (i64, i64) = (-1000, 2000);
/// NVIDIA's clocks move in bins of 15 MHz.
const BIN: i64 = 15;

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui) {
    if let Some(g) = &frame.gpu {
        let accent = ui.theme.accent;
        let ceiling = frame.limits().and_then(|l| l.gpu.as_ref()).and_then(|l| l.lock).map_or(2000.0, |r| f64::from(r.1));
        let shown = Graph { series: &[Series::area(&g.clock, accent)], max: top(&g.clock, ceiling), slots: SLOTS, top: "" };
        let now = g.now.core_mhz.map(|m| format::clock(m.into())).unwrap_or_default();
        graph(ui, "clock", &format!("Core clock, {SPAN}"), &now, &shown, |back| format::clock(at(&g.clock, back)));
        if let Some(bits) = g.now.reasons {
            let names: Vec<&str> = holding(bits).iter().map(|r| r.0).collect();
            reading(ui, "Held back by", &if names.is_empty() { "Nothing".to_owned() } else { names.join(", ") });
        }
    }
    let Some((limits, draft)) = tuning(&mut app.draft, frame, ui) else { return };
    let (Some(l), Some(g)) = (&limits.gpu, draft.gpu.as_mut()) else {
        note(ui, "There is no NVIDIA card to tune.");
        return;
    };
    let watts_now = frame.gpu.as_ref().and_then(|g| g.now.watts);

    caption(ui, "Clocks");
    card(ui, "clocks", |c| {
        let mut core = i64::from(g.core_offset);
        let range = shown_range((l.core_offset.0.into(), l.core_offset.1.into()), CORE_USEFUL, core);
        if c.slider_int("Core offset", "Added to every step of its clock curve. Go up 15 MHz at a time, testing as you go.", &mut core, range, BIN, |v| format::offset(v as i64)) {
            g.core_offset = core as i32;
        }
        let mut memory = i64::from(g.memory_offset);
        let range = shown_range((l.memory_offset.0.into(), l.memory_offset.1.into()), MEMORY_USEFUL, memory);
        if c.slider_int("Memory offset", "Added to its memory's clock. Too far shows as flickering specks, or slower frames.", &mut memory, range, 50, |v| format::offset(v as i64)) {
            g.memory_offset = memory as i32;
        }
        let Some((lo, hi)) = l.lock else { return };
        let mut held = g.lock.is_some();
        if c.switch("Hold the core clock", "With a positive offset: the same clock at less voltage. This is how Linux undervolts.", &mut held) {
            g.lock = held.then(|| hi.saturating_sub(13 * BIN as u32).max(lo));
        }
        if let Some(mhz) = g.lock.as_mut() {
            c.nested(|c| {
                let mut v = i64::from(*mhz);
                if c.slider_int("Hold it at", "The fastest it may run.", &mut v, (lo.into(), hi.into()), BIN, |v| format::clock(v)) {
                    *mhz = v as u32;
                }
            });
        }
    });

    caption(ui, "Power");
    card(ui, "power", |c| {
        let mut watts = i64::from(g.power_limit);
        let hint = format!("Stock is {}. More lets it boost further, and run hotter.", format::watts(l.power_default.into()));
        if c.slider_int("Power limit", &hint, &mut watts, (l.power.0.into(), l.power.1.into()), 5, |v| format::watts(v)) {
            g.power_limit = watts as u32;
        }
        if let Some(w) = watts_now {
            let limit = f64::from(g.power_limit.max(1));
            c.meter("Drawing now", "", f64::from(w) / limit, &format!("{} of {}", format::watts(w.into()), format::watts(limit)), look::info());
        }
    });
    note(ui, "Nothing changes until Apply. Whatever you apply goes back by itself after a few seconds unless you keep it, so a setting that freezes the screen undoes itself.");
}

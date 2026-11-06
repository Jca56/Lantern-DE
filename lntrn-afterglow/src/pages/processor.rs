//! The processor: how busy and hot it has been, then how it runs (the
//! governor, its energy preference, turbo, a ceiling on speed), its
//! package power limits, and its cores' clocks now.

use lntrn_afterglowd::tune::CpuLimits;
use lntrn_kit::chart::{Graph, Series};
use lntrn_kit::{caption, card, look, note};
use lntrn_ui::Ui;

use super::{HEAT, LOAD, SLOTS, SPAN, at, graph, legend, tuning};
use crate::app::App;
use crate::format;
use crate::sample::Frame;

/// What the menus call a governor or an energy preference.
fn named(word: &str) -> String {
    match word {
        "performance" => "Performance".to_owned(),
        "powersave" => "Powersave".to_owned(),
        "schedutil" => "Schedutil".to_owned(),
        "ondemand" => "On demand".to_owned(),
        "conservative" => "Conservative".to_owned(),
        "default" => "Default".to_owned(),
        "balance_performance" => "Balanced, quicker".to_owned(),
        "balance_power" => "Balanced, cooler".to_owned(),
        "power" => "Power saving".to_owned(),
        other => other.replace('_', " "),
    }
}

/// What the top-speed ceiling comes to, for each kind of core.
fn ceiling(l: &CpuLimits, percent: f64) -> String {
    let at = |mhz: u32| format::clock(f64::from(mhz) * percent / 100.0);
    match (l.pcore_mhz, l.ecore_mhz) {
        (0, 0) => format::percent(percent),
        (p, 0) => format!("{} · {}", format::percent(percent), at(p)),
        (p, e) => format!("{} · {} / {}", format::percent(percent), at(p), at(e)),
    }
}

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui) {
    let cpu = &frame.cpu;
    let shown = Graph { series: &[Series::area(&cpu.usage, LOAD), Series::line(&cpu.celsius, HEAT)], max: 100.0, slots: SLOTS, top: "100" };
    let said = format!("{}, {}", format::percent(cpu.now.usage.into()), cpu.now.celsius.map(|c| format::celsius(c.into())).unwrap_or_default());
    graph(ui, "processor", &format!("Load and heat, {SPAN}"), &said, &shown, |back| format!("{}, {}", format::percent(at(&cpu.usage, back)), format::celsius(at(&cpu.celsius, back))));
    legend(ui, &[(LOAD, "Load, %"), (HEAT, "Package heat, °C")]);

    let watts_now = frame.status().and_then(|s| s.package_watts);
    let (p_mhz, e_mhz) = frame.core_clocks();
    let counts = |pcore: bool| cpu.now.threads.iter().filter(|t| t.pcore == pcore).count();
    let Some((limits, draft)) = tuning(&mut app.draft, frame, ui) else { return };
    let (l, c) = (&limits.cpu, &mut draft.cpu);

    caption(ui, "How it runs");
    card(ui, "runs", |card| {
        if l.governors.len() > 1 {
            let labels: Vec<String> = l.governors.iter().map(|g| named(g)).collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut i = l.governors.iter().position(|g| *g == c.governor).unwrap_or(0);
            let hint = if c.governor == "performance" { "Every core kept at its top speed, all the time." } else { "Speed follows the load: it still boosts when there's work." };
            if card.segmented("Governor", hint, &mut i, &labels) {
                c.governor = l.governors[i].clone();
            }
        }
        if !l.epps.is_empty() {
            if c.governor == "performance" {
                card.value("Energy preference", "The performance governor always prefers performance.", "Performance");
            } else {
                let labels: Vec<String> = l.epps.iter().map(|e| named(e)).collect();
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                let mut i = l.epps.iter().position(|e| *e == c.epp).unwrap_or(0);
                if card.dropdown("Energy preference", "How eagerly it speeds up for work, against how much power it saves.", &mut i, &labels) {
                    c.epp = l.epps[i].clone();
                }
            }
        }
        if l.turbo {
            card.switch("Turbo", "Run past the base clock whenever there's room to.", &mut c.turbo);
        }
        let mut top = i64::from(c.max_percent);
        let shown = |v: f64| ceiling(l, v);
        if card.slider_int("Top speed", "A ceiling on every core's clock: cooler and quieter, a little slower.", &mut top, (10, 100), 5, shown) {
            c.max_percent = top as u32;
        }
    });

    if let Some((lo, hi)) = l.power {
        caption(ui, "Package power");
        card(ui, "power", |card| {
            let (mut pl1, mut pl2) = (i64::from(c.pl1), i64::from(c.pl2));
            if card.slider_int("Sustained (PL1)", "What it may draw for as long as it likes.", &mut pl1, (lo.into(), hi.into()), 5, |v| format::watts(v)) {
                c.pl1 = pl1 as u32;
                // Boost is never below sustained.
                c.pl2 = c.pl2.max(c.pl1);
            }
            if card.slider_int("Boost (PL2)", "What it may draw in bursts of a few seconds.", &mut pl2, (lo.into(), hi.into()), 5, |v| format::watts(v)) {
                c.pl2 = pl2 as u32;
                c.pl1 = c.pl1.min(c.pl2);
            }
            if let Some(w) = watts_now {
                let limit = f64::from(c.pl1.max(1));
                card.meter("Drawing now", "", f64::from(w) / limit, &format!("{} of {}", format::watts(w.into()), format::watts(limit)), look::info());
            }
        });
        note(ui, &format!("Afterglow won't go past the {hi} W the firmware set: on recent Intel chips, more than that is how they get hurt."));
    }

    caption(ui, "Cores now");
    card(ui, "cores", |card| {
        if p_mhz > 0.0 {
            let label = if e_mhz > 0.0 { "Performance cores" } else { "Cores" };
            card.value(label, &format!("{} threads", counts(true)), &format!("{} on average", format::clock(p_mhz)));
        }
        if e_mhz > 0.0 {
            card.value("Efficiency cores", &format!("{} threads", counts(false)), &format!("{} on average", format::clock(e_mhz)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smoke::fixture;

    #[test]
    fn a_ceiling_says_what_each_kind_of_core_comes_to() {
        let l = fixture::limits().cpu;
        assert_eq!(ceiling(&l, 80.0), "80% · 4.40 GHz / 3.44 GHz");
        assert_eq!(ceiling(&CpuLimits { ecore_mhz: 0, ..l.clone() }, 100.0), "100% · 5.50 GHz");
        assert_eq!((named("balance_power"), named("odd_one")), ("Balanced, cooler".to_owned(), "odd one".to_owned()));
    }
}

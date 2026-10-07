//! What the machine's sensors read: the battery when there is one, then
//! each chip's temperatures and fans.

use lntrn_kit::chart;
use lntrn_kit::{Card, caption, card, look, note};
use lntrn_ui::Ui;

use crate::format;
use crate::sample::Frame;
use crate::sample::sensors::{Battery, Chip, Temp};

/// Where a temperature's meter is full when the chip names no limit.
const HOT: f32 = 100.0;
/// A chip with more temperatures than this (one per core of a big
/// processor) has its cores summed up in one row.
const MANY: usize = 6;

/// A sensor's name as the chip gives it, said more plainly.
fn plain(label: &str) -> String {
    match label {
        l if l.starts_with("Package id") => "Whole package".to_owned(),
        "Tctl" | "Tdie" => "Processor".to_owned(),
        "Composite" => "Drive".to_owned(),
        l => l.to_owned(),
    }
}

/// Whether a temperature is one core's: a row among many like it.
fn is_core(t: &Temp) -> bool {
    t.label.strip_prefix("Core ").is_some_and(|n| n.parse::<u32>().is_ok())
}

/// The coolest and hottest of `temps`, and the lowest limit any names.
fn range(temps: &[&Temp]) -> Option<(f32, f32, f32)> {
    let hottest = temps.iter().map(|t| t.celsius).reduce(f32::max)?;
    let coolest = temps.iter().map(|t| t.celsius).reduce(f32::min)?;
    Some((coolest, hottest, temps.iter().filter_map(|t| t.limit).reduce(f32::min).unwrap_or(HOT)))
}

/// How long a battery has, as its row says it.
fn time_left(b: &Battery) -> Option<(&'static str, String)> {
    let hours = b.hours?;
    let label = if b.status == "Charging" { "Full in" } else { "Lasts" };
    Some((label, format::duration(f64::from(hours) * 3600.0)))
}

fn battery(ui: &mut Ui, b: &Battery) {
    card(ui, &b.name, |c| {
        let frac = f64::from(b.percent) / 100.0;
        // Running low is what to warn of here, not running full.
        let color = if b.percent < 15.0 {
            look::BAD
        } else if b.percent < 30.0 {
            look::WARN
        } else {
            look::GOOD
        };
        c.meter("Charge", &b.status, frac, &format::percent(f64::from(b.percent), false), color);
        if let Some((label, time)) = time_left(b) {
            c.value(label, "At the rate it is going now.", &time);
        }
        if let Some(watts) = b.watts {
            c.value("Power", "", &format!("{watts:.1} W"));
        }
        if let Some(health) = b.health {
            c.value("Health", "What it holds when full, of what it held new.", &format::percent(f64::from(health) * 100.0, false));
        }
        if let Some(cycles) = b.cycles {
            c.value("Charge cycles", "", &format::count(u64::from(cycles)));
        }
    });
}

/// A temperature's row: a meter of how near its limit it is, in the
/// accent until it gets close.
fn temp_row(c: &mut Card, label: &str, hint: &str, celsius: f32, limit: f32, said: &str) {
    let frac = f64::from(celsius / limit.max(1.0));
    let color = chart::heat(frac, c.ui.theme.accent);
    c.meter(label, hint, frac, said, color);
}

fn chip(ui: &mut Ui, chip: &Chip) {
    card(ui, "chip", |c| {
        let fold = chip.temps.len() > MANY;
        for t in chip.temps.iter().filter(|t| !(fold && is_core(t))) {
            temp_row(c, &plain(&t.label), "", t.celsius, t.limit.unwrap_or(HOT), &format::celsius(f64::from(t.celsius)));
        }
        let cores: Vec<&Temp> = chip.temps.iter().filter(|t| fold && is_core(t)).collect();
        if let Some((coolest, hottest, limit)) = range(&cores) {
            temp_row(c, "Cores", &format!("The coolest and the hottest of {}.", cores.len()), hottest, limit, &format!("{coolest:.0} to {hottest:.0} °C"));
        }
        for fan in &chip.fans {
            let said = if fan.rpm == 0 { "Stopped".to_owned() } else { format!("{} rpm", format::count(u64::from(fan.rpm))) };
            c.value(&fan.label, "", &said);
        }
    });
}

pub fn draw(frame: &Frame, ui: &mut Ui) {
    let s = &frame.sensors;
    if s.chips.is_empty() && s.batteries.is_empty() {
        note(ui, "Nothing here has a sensor the kernel can read.");
    }
    if !s.batteries.is_empty() {
        caption(ui, "Battery");
        for b in &s.batteries {
            battery(ui, b);
        }
    }
    for (i, c) in s.chips.iter().enumerate() {
        ui.push_index(i);
        // Two of a kind (a pair of memory modules) are told apart.
        let alike = s.chips.iter().filter(|o| o.name == c.name).count();
        let nth = s.chips[..i].iter().filter(|o| o.name == c.name).count() + 1;
        caption(ui, &if alike > 1 { format!("{} {nth}", c.name) } else { c.name.clone() });
        chip(ui, c);
        ui.pop_id();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str, celsius: f32, limit: Option<f32>) -> Temp {
        Temp { label: label.to_owned(), celsius, limit }
    }

    #[test]
    fn sensors_are_said_plainly_and_cores_are_summed_up() {
        assert_eq!((plain("Package id 0"), plain("Composite"), plain("Sensor 2")), ("Whole package".to_owned(), "Drive".to_owned(), "Sensor 2".to_owned()));
        assert!(is_core(&temp("Core 12", 0.0, None)));
        assert!(!is_core(&temp("Core", 0.0, None)) && !is_core(&temp("Core temp", 0.0, None)) && !is_core(&temp("Package id 0", 0.0, None)));
        let (a, b, c) = (temp("Core 0", 50.0, Some(100.0)), temp("Core 4", 58.0, Some(90.0)), temp("Core 8", 47.0, None));
        assert_eq!(range(&[&a, &b, &c]), Some((47.0, 58.0, 90.0)));
        assert_eq!(range(&[&c]), Some((47.0, 47.0, HOT)));
        assert_eq!(range(&[]), None);
    }

    #[test]
    fn a_battery_says_how_long_it_has_either_way() {
        let b = |status: &str, hours| Battery { status: status.to_owned(), hours, ..Battery::default() };
        assert_eq!(time_left(&b("Discharging", Some(3.4))), Some(("Lasts", "3 h 24 min".to_owned())));
        assert_eq!(time_left(&b("Charging", Some(0.5))), Some(("Full in", "30 min".to_owned())));
        assert_eq!(time_left(&b("Full", None)), None);
    }
}

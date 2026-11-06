//! The tune being edited, beside the one applied: what differs between
//! them, by name, and a short line saying what a tune does.
//!
//! The pages edit a draft of their own; it is the applied tune when the
//! helper first answers, and changes only when the user changes it (or
//! loads a profile, discards, resets). After a trial goes back the draft
//! still holds what was tried, so it can be nudged and tried again.

use lntrn_afterglowd::tune::{Fan, Tune};

use crate::format;

/// The knobs whose values differ between `a` and `b`, as the pages name
/// them.
pub fn differences(a: &Tune, b: &Tune) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut differ = |name: &'static str, differs: bool| {
        if differs {
            out.push(name);
        }
    };
    match (&a.gpu, &b.gpu) {
        (Some(x), Some(y)) => {
            differ("core offset", x.core_offset != y.core_offset);
            differ("memory offset", x.memory_offset != y.memory_offset);
            differ("power limit", x.power_limit != y.power_limit);
            differ("clock lock", x.lock != y.lock);
            differ("fans", x.fan != y.fan);
        }
        (x, y) => differ("graphics card", x.is_some() != y.is_some()),
    }
    let (x, y) = (&a.cpu, &b.cpu);
    differ("governor", x.governor != y.governor);
    // The performance governor forces its own preference: what the
    // other says only matters under another governor.
    differ("energy preference", x.epp != y.epp && !(x.governor == "performance" && y.governor == "performance"));
    differ("turbo", x.turbo != y.turbo);
    differ("top speed", x.max_percent != y.max_percent);
    differ("sustained power", x.pl1 != y.pl1);
    differ("boost power", x.pl2 != y.pl2);
    out
}

/// Whether `a` and `b` set the machine alike.
pub fn same(a: &Tune, b: &Tune) -> bool {
    differences(a, b).is_empty()
}

/// What the fans do, in a word or two.
pub fn fans(fan: &Fan) -> String {
    match fan {
        Fan::Auto => "Automatic".to_owned(),
        Fan::Fixed(p) => format!("Fixed at {}", format::percent(f64::from(*p))),
        Fan::Curve(points) => format!("Curve of {} points", points.len()),
    }
}

/// A short line saying what `tune` changes from `stock`: "+150 core,
/// +500 memory, 450 W, fan curve". "Stock" when nothing.
pub fn summary(tune: &Tune, stock: &Tune) -> String {
    let mut parts = Vec::new();
    if let (Some(g), Some(s)) = (&tune.gpu, &stock.gpu) {
        if g.core_offset != s.core_offset {
            parts.push(format!("{} core", format::offset(g.core_offset.into())));
        }
        if g.memory_offset != s.memory_offset {
            parts.push(format!("{} memory", format::offset(g.memory_offset.into())));
        }
        if g.power_limit != s.power_limit {
            parts.push(format::watts(f64::from(g.power_limit)));
        }
        if let Some(mhz) = g.lock {
            parts.push(format!("held at {}", format::clock(f64::from(mhz))));
        }
        match &g.fan {
            Fan::Auto => {}
            Fan::Fixed(p) => parts.push(format!("fans {}", format::percent(f64::from(*p)))),
            Fan::Curve(_) => parts.push("fan curve".to_owned()),
        }
    }
    let (c, s) = (&tune.cpu, &stock.cpu);
    if c.governor != s.governor {
        parts.push(c.governor.clone());
    }
    if c.epp != s.epp && c.governor != "performance" {
        parts.push(c.epp.replace('_', " "));
    }
    if c.turbo != s.turbo {
        parts.push(if c.turbo { "turbo on" } else { "turbo off" }.to_owned());
    }
    if c.max_percent != s.max_percent {
        parts.push(format!("top speed {}", format::percent(f64::from(c.max_percent))));
    }
    if c.pl1 != s.pl1 || c.pl2 != s.pl2 {
        parts.push(format!("PL {}/{} W", c.pl1, c.pl2));
    }
    if parts.is_empty() { "Stock".to_owned() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smoke::fixture;

    #[test]
    fn differences_are_named_and_a_forced_preference_is_no_difference() {
        let stock = fixture::limits().stock;
        assert!(same(&stock, &stock));
        let mut t = stock.clone();
        t.gpu.as_mut().unwrap().core_offset = 150;
        t.cpu.turbo = false;
        assert_eq!(differences(&stock, &t), ["core offset", "turbo"]);
        let (mut a, mut b) = (stock.clone(), stock.clone());
        a.cpu.governor = "performance".into();
        b.cpu.governor = "performance".into();
        b.cpu.epp = "power".into();
        assert!(same(&a, &b), "the governor forces the preference");
    }

    #[test]
    fn a_summary_says_only_what_changed() {
        let stock = fixture::limits().stock;
        assert_eq!(summary(&stock, &stock), "Stock");
        let mut t = stock.clone();
        let g = t.gpu.as_mut().unwrap();
        g.core_offset = 150;
        g.memory_offset = 500;
        g.power_limit = 450;
        g.fan = Fan::Curve(Vec::new());
        t.cpu.pl1 = 200;
        assert_eq!(summary(&t, &stock), "+150 MHz core, +500 MHz memory, 450 W, fan curve, PL 200/253 W");
    }
}

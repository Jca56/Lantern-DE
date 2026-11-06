//! The first page: a tile for each reading worth a glance (each a way in
//! to the page that tunes it), what is holding the graphics card back,
//! and what is applied right now.

use lntrn_afterglowd::hw::nvml::reason;
use lntrn_afterglowd::tune::Tune;
use lntrn_kit::chart::{self, Tile};
use lntrn_kit::{caption, card, note};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{FILL, Ui};

use super::{FANS, HEAT, LOAD, POWER, SLOTS, top};
use crate::app::App;
use crate::edit;
use crate::format;
use crate::nav::Page;
use crate::sample::Frame;

const TILE_H: f64 = 190.0;
/// The least width a tile is given before a row holds fewer of them.
const TILE_MIN_W: f64 = 260.0;

/// Why the card is running slower than it could: each reason the driver
/// gives, as a name and a line saying what it means.
const REASONS: [(u64, &str, &str); 9] = [
    (reason::POWER_CAP, "Power limit", "It is drawing all the power it is allowed. A higher power limit lets it boost further."),
    (reason::SW_THERMAL, "Too hot", "The driver is slowing it to bring its heat down."),
    (reason::HW_THERMAL, "Too hot (board)", "The board itself is slowing it down to cool it."),
    (reason::HW_SLOWDOWN, "Board slowdown", "The board slowed it down: too hot, or drawing too much."),
    (reason::POWER_BRAKE, "Power brake", "The power supply asked it to slow down."),
    (reason::SYNC_BOOST, "Sync boost", "It is held in step with another card."),
    (reason::APP_CLOCKS, "Clock setting", "An application clock setting holds it."),
    (reason::DISPLAY_CLOCK, "Display clock", "The display's clock setting holds it."),
    (reason::IDLE, "Resting", "There isn't much to do, so it idles."),
];

/// The reasons `bits` gives, most telling first.
pub fn holding(bits: u64) -> Vec<(&'static str, &'static str)> {
    REASONS.iter().filter(|(bit, ..)| bits & bit != 0).map(|(_, name, why)| (*name, *why)).collect()
}

/// What one tile shows, and the page it opens.
struct Glance {
    page: Page,
    title: &'static str,
    value: String,
    note: String,
    values: Vec<f32>,
    max: f64,
    color: Color,
}

fn glances(frame: &Frame, accent: Color) -> Vec<Glance> {
    let mut out = Vec::new();
    let status = frame.status();
    let limits = frame.limits();
    let dash = || "—".to_owned();
    if let Some(g) = &frame.gpu {
        let n = &g.now;
        let lock_top = limits.and_then(|l| l.gpu.as_ref()).and_then(|l| l.lock).map_or(2000.0, |r| f64::from(r.1));
        out.push(Glance { page: Page::Graphics, title: "Core clock", value: n.core_mhz.map_or_else(dash, |m| format::clock(m.into())), note: n.usage.map(|u| format!("{u}% busy")).unwrap_or_default(), values: g.clock.clone(), max: top(&g.clock, lock_top), color: accent });
        let slowdown = limits.and_then(|l| l.gpu.as_ref()).map(|l| format!("slows itself at {}", format::celsius(l.slowdown.into()))).unwrap_or_default();
        out.push(Glance { page: Page::Fans, title: "Card heat", value: n.celsius.map_or_else(dash, |c| format::celsius(c.into())), note: slowdown, values: g.celsius.clone(), max: 100.0, color: HEAT });
        let limit = n.watts_limit.map(f64::from);
        out.push(Glance { page: Page::Graphics, title: "Card power", value: n.watts.map_or_else(dash, |w| format::watts(w.into())), note: limit.map(|l| format!("of {}", format::watts(l))).unwrap_or_default(), max: top(&g.watts, limit.unwrap_or(100.0)), values: g.watts.clone(), color: POWER });
        let mode = status.and_then(|s| s.applied.gpu.as_ref()).map(|a| edit::fans(&a.fan)).unwrap_or_default();
        out.push(Glance { page: Page::Fans, title: "Fans", value: n.fans.first().map_or_else(dash, |p| format::percent(f64::from(*p))), note: mode, values: g.fan.clone(), max: 100.0, color: FANS });
    }
    let c = &frame.cpu;
    let (p, e) = frame.core_clocks();
    let clocks = match (p > 0.0, e > 0.0) {
        (true, true) => format!("P {} · E {}", format::clock(p), format::clock(e)),
        (true, false) => format::clock(p),
        _ => String::new(),
    };
    out.push(Glance { page: Page::Processor, title: "Processor", value: format::percent(c.now.usage.into()), note: clocks, values: c.usage.clone(), max: 100.0, color: LOAD });
    out.push(Glance { page: Page::Processor, title: "Processor heat", value: c.now.celsius.map_or_else(dash, |t| format::celsius(t.into())), note: "the package".to_owned(), values: c.celsius.clone(), max: 100.0, color: HEAT });
    let watts = status.and_then(|s| s.package_watts);
    let pl1 = status.map(|s| s.applied.cpu.pl1).filter(|w| *w > 0);
    out.push(Glance { page: Page::Processor, title: "Package power", value: watts.map_or_else(dash, |w| format::watts(w.into())), note: pl1.map(|w| format!("of {w} W sustained")).unwrap_or_else(|| if status.is_none() { "needs the helper".to_owned() } else { String::new() }), max: top(&c.watts, f64::from(pl1.unwrap_or(100))), values: c.watts.clone(), color: POWER });
    out
}

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui) {
    let m = ui.m;
    let glances = glances(frame, ui.theme.accent);
    let gap = m.px(14.0);
    let columns = (((ui.avail_width() + gap) / (m.px(TILE_MIN_W) + gap)).floor() as usize).clamp(1, 4);
    let lines = glances.len().div_ceil(columns);
    let tile_h = m.px(TILE_H);
    ui.space(m.px(6.0));
    let area = ui.alloc(Vec2::new(FILL, lines as f64 * (tile_h + gap) - gap));
    let tile_w = (area.width() - gap * (columns as f64 - 1.0)) / columns as f64;
    ui.push_id("tiles");
    for (i, g) in glances.iter().enumerate() {
        let (col, line) = ((i % columns) as f64, (i / columns) as f64);
        let rect = Rect::from_min_size(Vec2::new(area.min.x + col * (tile_w + gap), area.min.y + line * (tile_h + gap)), Vec2::new(tile_w, tile_h));
        let id = ui.id(g.title);
        if chart::tile(ui, id, rect.round(), &Tile { title: g.title, value: &g.value, note: &g.note, values: &g.values, max: g.max, slots: SLOTS, color: g.color }) {
            app.page = g.page;
        }
    }
    ui.pop_id();

    if let Some(bits) = frame.gpu.as_ref().and_then(|g| g.now.reasons) {
        caption(ui, "What holds the card back");
        let reasons = holding(bits);
        card(ui, "holding", |c| {
            if reasons.is_empty() {
                c.value("Nothing", "It is free to boost as far as its clock curve goes.", "");
            }
            for (name, why) in &reasons {
                c.value(name, why, "");
            }
        });
    }

    caption(ui, "Applied now");
    match frame.status() {
        Some(s) => applied(ui, &s.applied, s.fan_percent),
        None => note(ui, "The helper isn't answering, so what is applied can't be told."),
    }
}

/// Every knob of `t`, as a card of readings.
fn applied(ui: &mut Ui, t: &Tune, fan_now: Option<u32>) {
    card(ui, "applied", |c| {
        if let Some(g) = &t.gpu {
            c.value("Core offset", "", &format::offset(g.core_offset.into()));
            c.value("Memory offset", "", &format::offset(g.memory_offset.into()));
            c.value("Power limit", "", &format::watts(g.power_limit.into()));
            c.value("Core clock", "", &g.lock.map_or_else(|| "Free to boost".to_owned(), |mhz| format!("Held at {}", format::clock(mhz.into()))));
            let fans = match fan_now {
                Some(p) => format!("{}, now {}", edit::fans(&g.fan), format::percent(p.into())),
                None => edit::fans(&g.fan),
            };
            c.value("Fans", "", &fans);
        }
        let p = &t.cpu;
        c.value("Governor", "", &p.governor);
        if !p.epp.is_empty() {
            let epp = if p.governor == "performance" { "performance (the governor's)".to_owned() } else { p.epp.replace('_', " ") };
            c.value("Energy preference", "", &epp);
        }
        c.value("Turbo", "", if p.turbo { "On" } else { "Off" });
        c.value("Top speed", "", &format::percent(p.max_percent.into()));
        if p.pl1 > 0 {
            c.value("Package power", "", &format!("{} W sustained, {} W boost", p.pl1, p.pl2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_come_most_telling_first() {
        assert!(holding(0).is_empty());
        let names: Vec<&str> = holding(reason::IDLE | reason::POWER_CAP | reason::SW_THERMAL).iter().map(|r| r.0).collect();
        assert_eq!(names, ["Power limit", "Too hot", "Resting"]);
    }
}

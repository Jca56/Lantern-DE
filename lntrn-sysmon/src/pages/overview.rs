//! The first page: a tile for each thing worth a glance, each a way in
//! to the page about it, and the programs working hardest right now.

use lntrn_kit::chart::{self, Tile};
use lntrn_kit::{caption_action, card, note};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{FILL, Ui};

use super::{GRAPHICS, MEMORY, NETWORK, STORAGE, peak};
use crate::app::App;
use crate::format;
use crate::nav::Page;
use crate::rows::{self, Row, View};
use crate::sample::{Frame, HISTORY};

const TILE_H: f64 = 200.0;
/// The least width a tile is given before a row holds fewer of them.
const TILE_MIN_W: f64 = 280.0;
/// How many programs the page lists.
const BUSIEST: usize = 5;

pub fn blurb(frame: &Frame) -> String {
    let name = if frame.facts.hostname.is_empty() { "This machine" } else { &frame.facts.hostname };
    format!("{name} has been up {}.", format::duration(frame.uptime))
}

/// Several histories added up, sample by sample from the newest back:
/// every drive's traffic as one line.
fn summed(histories: &[&[f32]]) -> Vec<f32> {
    let longest = histories.iter().map(|h| h.len()).max().unwrap_or(0);
    (0..longest).map(|i| histories.iter().filter_map(|h| h.len().checked_sub(longest - i).map(|at| h[at])).sum()).collect()
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
    let percent = |v: f32| format::percent(f64::from(v), false);
    let (cpu, mem) = (&frame.cpu, &frame.mem);
    let mut out = vec![
        Glance { page: Page::Cpu, title: "CPU", value: percent(cpu.usage), note: if cpu.mhz > 0.0 { format::clock(f64::from(cpu.mhz)) } else { String::new() }, values: cpu.history.clone(), max: 100.0, color: accent },
        Glance { page: Page::Memory, title: "Memory", value: percent(mem.used_percent()), note: format::bytes_of(mem.used(), mem.total), values: mem.history.clone(), max: 100.0, color: MEMORY },
    ];
    // The first card that says how busy it is.
    if let Some((gpu, usage)) = frame.gpus.iter().find_map(|g| g.usage.map(|u| (g, u))) {
        let note = match (gpu.memory, gpu.celsius) {
            (Some((used, total)), Some(c)) => format!("{}, {}", format::bytes_of(used, total), format::celsius(f64::from(c))),
            (Some((used, total)), None) => format::bytes_of(used, total),
            (None, Some(c)) => format::celsius(f64::from(c)),
            (None, None) => String::new(),
        };
        out.push(Glance { page: Page::Gpu, title: "GPU", value: percent(usage), note, values: gpu.usage_history.clone(), max: 100.0, color: GRAPHICS });
    }
    let drives = &frame.disks.drives;
    if !drives.is_empty() {
        let traffic = summed(&drives.iter().flat_map(|d| [d.read_history.as_slice(), d.write_history.as_slice()]).collect::<Vec<_>>());
        let now: f64 = drives.iter().map(|d| f64::from(d.read + d.written)).sum();
        // The fullest filesystem is the one worth a word.
        let fullest = frame.disks.volumes.iter().max_by(|a, b| crate::sample::mem::share(a.used(), a.total).total_cmp(&crate::sample::mem::share(b.used(), b.total)));
        let note = fullest.map(|v| format!("{} is {} full", v.mount, percent(crate::sample::mem::share(v.used(), v.total)))).unwrap_or_default();
        out.push(Glance { page: Page::Disks, title: "Disks", value: format::rate(now), note, max: format::rate_ceiling(peak(&[&traffic])), values: traffic, color: STORAGE });
    }
    if !frame.net.is_empty() {
        let down = summed(&frame.net.iter().map(|i| i.down_history.as_slice()).collect::<Vec<_>>());
        let (now, up): (f64, f64) = frame.net.iter().fold((0.0, 0.0), |(d, u), i| (d + f64::from(i.down), u + f64::from(i.up_rate)));
        out.push(Glance { page: Page::Network, title: "Network", value: format::rate(now), note: format!("{} up", format::rate(up)), max: format::rate_ceiling(peak(&[&down])), values: down, color: NETWORK });
    }
    out
}

/// The programs working hardest, as `(name, what it is using)`.
fn busiest(frame: &Frame) -> Vec<(String, String)> {
    let view = View { grouped: true, ..View::default() };
    rows::rows(&frame.procs, &view)
        .into_iter()
        .take(BUSIEST)
        .filter_map(|row| match row {
            Row::Group { app, count, cpu, memory, .. } => Some((format!("{app} ({count})"), cpu, memory)),
            Row::Proc { index, .. } => frame.procs.get(index).map(|p| (p.name.to_string(), p.cpu, p.memory)),
        })
        .map(|(name, cpu, memory)| (name, format!("{}   {}", format::percent(f64::from(cpu), true), format::bytes(memory))))
        .collect()
}

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui) {
    let m = ui.m;
    let glances = glances(frame, ui.theme.accent);
    let gap = m.px(14.0);
    let columns = (((ui.avail_width() + gap) / (m.px(TILE_MIN_W) + gap)).floor() as usize).clamp(1, 3);
    let lines = glances.len().div_ceil(columns);
    let tile_h = m.px(TILE_H);
    ui.space(m.px(6.0));
    let area = ui.alloc(Vec2::new(FILL, lines as f64 * (tile_h + gap) - gap));
    let tile_w = (area.width() - gap * (columns as f64 - 1.0)) / columns as f64;
    ui.push_id("tiles");
    for (i, g) in glances.iter().enumerate() {
        let (col, line) = ((i % columns) as f64, (i / columns) as f64);
        let rect = Rect::from_min_size(Vec2::new(area.min.x + col * (tile_w + gap), area.min.y + line * (tile_h + gap)), Vec2::new(tile_w, tile_h));
        let id = ui.id(g.page.id());
        if chart::tile(ui, id, rect.round(), &Tile { title: g.title, value: &g.value, note: &g.note, values: &g.values, max: g.max, slots: HISTORY, color: g.color }) {
            app.page = g.page;
        }
    }
    ui.pop_id();

    if caption_action(ui, "Working hardest", "All processes") {
        app.page = Page::Processes;
    }
    let busiest = busiest(frame);
    if busiest.is_empty() {
        note(ui, "Nothing is running that can be read.");
        return;
    }
    card(ui, "busiest", |c| {
        for (name, using) in &busiest {
            c.value(name, "", using);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    #[test]
    fn histories_are_added_from_their_newest_back() {
        assert_eq!(summed(&[&[1.0, 2.0, 3.0], &[10.0, 20.0]]), [1.0, 12.0, 23.0]);
        assert!(summed(&[]).is_empty());
        assert_eq!(summed(&[&[], &[4.0]]), [4.0]);
    }

    #[test]
    fn there_is_a_tile_for_what_the_machine_has_and_none_for_what_it_hasnt() {
        let said = |frame: &Frame| glances(frame, Color::WHITE).iter().map(|c| format!("{}: {} / {}", c.title, c.value, c.note)).collect::<Vec<_>>();
        assert_eq!(said(&fixture::frame()), ["CPU: 37% / 3.95 GHz", "Memory: 66% / 21.0 of 32.0 GiB", "GPU: 12% / 3.0 of 12.0 GiB, 48 °C", "Disks: 1.4 MiB/s / /mnt/storage is 95% full", "Network: 2.1 MiB/s / 78 KiB/s up"]);
        // A card that won't say how busy it is has no tile; nor do
        // drives and a network that aren't there.
        assert_eq!(said(&fixture::bare()), ["CPU: 3% / ", "Memory: 25% / 1.0 of 4.0 GiB"]);
    }

    #[test]
    fn the_hardest_working_programs_are_listed_folded() {
        assert_eq!(busiest(&fixture::frame()), [("rustc (2)".to_owned(), "24%   1.3 GiB".to_owned()), ("firefox (4)".to_owned(), "12%   2.1 GiB".to_owned()), ("lntrn-compositor".to_owned(), "3.1%   310 MiB".to_owned()), ("steam".to_owned(), "1.0%   890 MiB".to_owned()), ("init".to_owned(), "0.0%   2.0 MiB".to_owned())]);
        assert_eq!(blurb(&fixture::frame()), "testbench has been up 3 d 4 h.");
    }
}

//! The processor's popup: its name, its load graphed and said big with
//! its temperature, its clock and what sets it, the load averages and how
//! long the machine has been up, and the busiest programs.

use lntrn_math::Rect;
use lntrn_status::LoadState;
use lntrn_ui::Ui;

use super::{Frame, GAP, big, frame_h, graph, open, programs, programs_h, sysmon_button};
use crate::monitors::{Kind, heat};
use crate::tray::popup::kit::{self, LINE_H};

pub fn draw(ui: &mut Ui, s: &LoadState, slots: usize, frame: &Frame, sysmon: &mut bool) -> Rect {
    let m = ui.m;
    let mut lines = Vec::new();
    let clock = (s.cpu.mhz > 0.0).then(|| format!("{:.2} GHz average", s.cpu.mhz / 1000.0));
    let parts: Vec<String> = clock.into_iter().chain((!s.cpu.governor.is_empty()).then(|| s.cpu.governor.clone())).collect();
    if !parts.is_empty() {
        lines.push(parts.join(" · "));
    }
    let [one, five, fifteen] = s.load;
    lines.push(format!("Load {one:.2} · {five:.2} · {fifteen:.2}"));
    lines.push(format!("Up {}", up(s.uptime)));
    let rows: Vec<(String, String)> = s.top.as_ref().map(|t| t.cpu.iter().map(|(name, share)| (name.to_string(), share_of(*share))).collect()).unwrap_or_default();
    let named = !s.cpu_name.is_empty();
    let h = frame_h(ui) + m.px(LINE_H) * (lines.len() + usize::from(named)) as f64 + m.px(GAP) + programs_h(ui, rows.len());
    let (mut col, panel) = open(ui, frame, Kind::Cpu, h);
    let (dim, text_dim) = (kit::dim_style(ui), ui.theme.text_dim);
    if named {
        kit::line(ui, col.take(m.px(LINE_H)), &s.cpu_name, &dim, text_dim);
    }
    let hovered = graph(ui, &mut col, Kind::Cpu, &s.cpu.history, slots);
    big(ui, &mut col, s, &s.cpu.history, hovered, s.cpu_celsius.map(|c| (format!("{} °C", c.round() as i32), heat(Kind::Cpu, c))));
    let name = kit::name_style(ui);
    for line in &lines {
        kit::line(ui, col.take(m.px(LINE_H)), line, &name, ui.theme.text);
    }
    col.take(m.px(GAP));
    programs(ui, &mut col, "Busiest programs", &rows);
    *sysmon = sysmon_button(ui, &mut col, Kind::Cpu);
    panel
}

/// How long the machine has been up, as said: `3 d 4 h`, `4 h 12 min`.
fn up(secs: f64) -> String {
    let mins = (secs / 60.0) as u64;
    match (mins / 1440, mins / 60 % 24, mins % 60) {
        (0, 0, m) => format!("{m} min"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

/// A program's share of the whole machine: `3.2%` under ten, `12%` over.
fn share_of(share: f32) -> String {
    if share < 10.0 { format!("{share:.1}%") } else { format!("{}%", share.round() as u32) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_and_shares_read_as_said() {
        assert_eq!(up(12.0 * 60.0 + 30.0), "12 min");
        assert_eq!(up(4.0 * 3600.0 + 12.0 * 60.0), "4 h 12 min");
        assert_eq!(up(3.0 * 86400.0 + 4.0 * 3600.0 + 59.0 * 60.0), "3 d 4 h");
        assert_eq!((share_of(3.24), share_of(12.4)), ("3.2%".to_owned(), "12%".to_owned()));
    }
}

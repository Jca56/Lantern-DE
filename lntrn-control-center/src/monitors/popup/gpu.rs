//! The graphics card's popup: its name and driver, its load graphed and
//! said big with its temperature, its video memory and power draw on
//! meters, its fans and clocks. Asleep, it says so instead: nothing is
//! asked of a sleeping card.

use lntrn_math::Rect;
use lntrn_status::LoadState;
use lntrn_status::load::gpu::Gpu;
use lntrn_ui::Ui;

use super::{Frame, GAP, METER_ROW_H, big, frame_h, graph, meter, open, sysmon_button, used_of};
use crate::monitors::{Kind, heat};
use crate::tray::popup::kit::{self, LINE_H};

pub fn draw(ui: &mut Ui, s: &LoadState, g: &Gpu, slots: usize, frame: &Frame, sysmon: &mut bool) -> Rect {
    let m = ui.m;
    let mut lines = Vec::new();
    if g.asleep {
        lines.push("Asleep, saving power".to_owned());
    }
    if !g.fans.is_empty() {
        let each: Vec<String> = g.fans.iter().map(|f| format!("{f}%")).collect();
        lines.push(format!("{} {}", if each.len() == 1 { "Fan" } else { "Fans" }, each.join(" · ")));
    }
    let clocks: Vec<String> = g.core_mhz.map(|c| format!("{c} MHz core")).into_iter().chain(g.memory_mhz.map(|c| format!("{c} MHz memory"))).collect();
    if !clocks.is_empty() {
        lines.push(clocks.join(" · "));
    }
    let meters = usize::from(g.memory.is_some()) + usize::from(g.watts.is_some());
    let heads = 1 + usize::from(!g.driver.is_empty());
    let h = frame_h(ui) + m.px(LINE_H) * (heads + lines.len()) as f64 + m.px(METER_ROW_H) * meters as f64 + m.px(GAP);
    let (mut col, panel) = open(ui, frame, Kind::Gpu, h);
    let (name, dim) = (kit::name_style(ui), kit::dim_style(ui));
    kit::line(ui, col.take(m.px(LINE_H)), &g.name, &name, ui.theme.text);
    if !g.driver.is_empty() {
        kit::line(ui, col.take(m.px(LINE_H)), &g.driver, &dim, ui.theme.text_dim);
    }
    let hovered = graph(ui, &mut col, Kind::Gpu, &g.usage_history, slots);
    big(ui, &mut col, s, &g.usage_history, hovered, g.celsius.map(|c| (format!("{} °C", c.round() as i32), heat(Kind::Gpu, c))));
    if let Some((used, total)) = g.memory {
        meter(ui, &mut col, "vram", "Video memory", &used_of(used, total), used as f64 / total.max(1) as f64, Kind::Gpu.color());
    }
    if let Some(watts) = g.watts {
        let (value, frac) = match g.watts_limit {
            Some(limit) => (format!("{} W of {} W", watts.round() as u32, limit.round() as u32), f64::from(watts / limit)),
            None => (format!("{} W", watts.round() as u32), 0.0),
        };
        meter(ui, &mut col, "power", "Power", &value, frac, Kind::Gpu.color());
    }
    for line in &lines {
        let color = if g.asleep { ui.theme.text_dim } else { ui.theme.text };
        kit::line(ui, col.take(m.px(LINE_H)), line, &name, color);
    }
    col.take(m.px(GAP));
    *sysmon = sysmon_button(ui, &mut col, Kind::Gpu);
    panel
}

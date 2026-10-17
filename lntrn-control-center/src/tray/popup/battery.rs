//! The battery's popup: its percent, big, over a bar of its charge; what
//! it is doing, how long it has left and what it draws; and its charge
//! limit, where the firmware has one this user may set.

use lntrn_math::{Rect, Vec2};
use lntrn_status::battery::Charge;
use lntrn_text::TextStyle;
use lntrn_ui::Ui;

use super::kit::{self, HEAD_H, LINE_H, ROW_H};
use super::{Frame, duration};
use crate::tray::Tray;
use crate::tray::bar::charge_color;

const WIDTH: f64 = 380.0;
/// The big percent's row, and its text, logical.
const BIG_H: f64 = 64.0;
const BIG: f64 = 44.0;
/// Where a set limit stops the charge.
const LIMIT: u32 = 80;

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let battery = tray.battery.as_ref()?;
    let s = battery.state();
    let m = ui.m;
    let mut lines = vec![s.charge.label().to_owned()];
    lines.extend(s.seconds_left.map(|secs| duration(secs, s.charge)));
    lines.extend(s.watts.map(|w| format!("{} {w:.1} W", if s.charge == Charge::Charging { "Taking" } else { "Drawing" })));
    let limit = s.limit.filter(|l| l.writable || l.percent < 100);
    let h = m.px(HEAD_H + BIG_H) + m.px(LINE_H) * lines.len() as f64 + limit.map_or(0.0, |_| m.px(ROW_H + 8.0));
    let (mut col, panel) = frame.open(ui, WIDTH, h);
    kit::header(ui, col.take(m.px(HEAD_H)), "Battery", None);

    // The percent, and a bar of the charge beside it.
    let big = col.take(m.px(BIG_H));
    let style = TextStyle::new(m.px(BIG) as f32).bold();
    let text = format!("{}%", s.percent);
    let text_w = ui.measure(&text, &style);
    ui.text_in_rect(&text, &style, big, ui.theme.text);
    let bar = Rect::new(Vec2::new(big.min.x + text_w + m.px(18.0), (big.center().y - m.px(7.0)).round()), Vec2::new(big.max.x, (big.center().y + m.px(7.0)).round()));
    ui.draw.rounded_rect(bar, bar.height() * 0.5, lntrn_kit::look::TRACK);
    let fill = (bar.width() * f64::from(s.percent) / 100.0).max(bar.height());
    ui.draw.rounded_rect(Rect::new(bar.min, Vec2::new(bar.min.x + fill, bar.max.y)), bar.height() * 0.5, charge_color(&s));

    let (dim, ink) = (kit::dim_style(ui), ui.theme.text);
    for (i, line) in lines.iter().enumerate() {
        let color = if i == 0 { ink } else { ui.theme.text_dim };
        kit::line(ui, col.take(m.px(LINE_H)), line, &dim, color);
    }
    if let Some(limit) = limit {
        col.take(m.px(8.0));
        let row = col.take(m.px(ROW_H));
        let on = limit.percent < 100;
        let words = if on { format!("Stops charging at {}%", limit.percent) } else { format!("Stop charging at {LIMIT}%") };
        let name = kit::name_style(ui);
        if limit.writable {
            let size = Vec2::new(m.px(56.0), m.px(32.0));
            let switch = Rect::from_min_size(Vec2::new(row.max.x - size.x, (row.center().y - size.y * 0.5).round()), size);
            kit::line(ui, Rect::new(row.min, Vec2::new(switch.min.x - m.px(12.0), row.max.y)), &words, &name, ink);
            if kit::switch(ui, ui.id("charge-limit"), switch, on)
                && let Err(e) = battery.set_limit(if on { 100 } else { LIMIT })
            {
                lntrn_core::log_warn!("battery: can't set the charge limit: {e}");
            }
        } else {
            kit::line(ui, row, &words, &name, ui.theme.text_dim);
        }
    }
    Some(panel)
}

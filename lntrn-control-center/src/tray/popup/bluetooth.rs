//! The Bluetooth popup: the switch, and the paired devices, connected
//! first, each with its kind's picture, its battery where it says, and
//! what it is doing; a click connects one, or disconnects it. Pairing new
//! ones is the Command Center's for now (it holds the desktop's pairing
//! agent).

use lntrn_kit::look;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::Ui;

use super::Frame;
use super::kit::{self, HEAD_H, LINE_H, ROW_H};
use crate::tray::Tray;
use crate::tray::glyphs::Glyph;

const WIDTH: f64 = 440.0;
const PICTURE: f64 = 28.0;

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let s = tray.bluetooth.as_ref()?.state();
    let m = ui.m;
    let devices = if s.powered { s.devices.as_slice() } else { &[] };
    let mut lines: Vec<(String, bool)> = Vec::new();
    if !s.powered {
        lines.push(("Bluetooth is off".to_owned(), false));
    } else if devices.is_empty() {
        lines.push(("No paired devices yet".to_owned(), false));
    }
    if let Some(error) = &s.error {
        lines.push((format!("Couldn't: {error}"), true));
    }
    let h = m.px(HEAD_H) + m.px(LINE_H) * (lines.len() + 1) as f64 + m.px(8.0) + kit::list_h(ui, devices.len());
    let (mut col, panel) = frame.open(ui, WIDTH, h);
    let flip = kit::header(ui, col.take(m.px(HEAD_H)), "Bluetooth", Some(s.powered));
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    let dim_style = kit::dim_style(ui);
    for (line, bad) in &lines {
        kit::line(ui, col.take(m.px(LINE_H)), line, &dim_style, if *bad { look::BAD } else { ink });
    }
    col.take(m.px(8.0));

    let list = col.take(kit::list_h(ui, devices.len()));
    let name = kit::name_style(ui);
    let mut toggle = None;
    let mut scroll = tray.scroll;
    let glyphs = &mut tray.glyphs;
    kit::list(ui, list, devices.len(), m.px(ROW_H), &mut scroll, |ui, i, row| {
        let d = &devices[i];
        let busy = s.busy.contains(&d.path);
        let r = kit::row(ui, ui.id("device").with_index(i), row, d.connected);
        if r.clicked && !busy {
            toggle = Some((d.path.clone(), !d.connected));
        }
        let pic = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
        glyphs.draw(ui, Glyph::for_device(&d.icon), pic, ink);
        // What it is doing, then its battery, from the right.
        let mut right = row.max.x - m.px(12.0);
        let doing = match (busy, d.connected) {
            (true, true) => Some(("Disconnecting…", dim)),
            (true, false) => Some(("Connecting…", dim)),
            (false, true) => Some(("Connected", accent)),
            (false, false) => None,
        };
        let battery = d.battery.map(|p| format!("{p}%"));
        for (text, color) in doing.map(|(t, c)| (t.to_owned(), c)).into_iter().chain(battery.map(|b| (b, dim))) {
            let w = ui.measure(&text, &dim_style);
            ui.text_right(&text, &dim_style, Rect::new(Vec2::new(right - w, row.min.y), Vec2::new(right, row.max.y)), color);
            right -= w + m.px(12.0);
        }
        kit::line(ui, Rect::new(Vec2::new(pic.max.x + m.px(14.0), row.min.y), Vec2::new(right, row.max.y)), &d.name, &name, ink);
    });
    tray.scroll = scroll;
    kit::line(ui, col.take(m.px(LINE_H)), "Pair new devices from the Command Center", &dim_style, dim);

    if let Some(bt) = &tray.bluetooth {
        if flip {
            bt.set_powered(!s.powered);
        }
        if let Some((path, on)) = toggle {
            bt.connect(&path, on);
        }
    }
    Some(panel)
}

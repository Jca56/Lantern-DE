//! The WiFi popup: the switch, what it is connected to (and a cable, when
//! one is plugged in), and the networks in range, the one in use first
//! with a button to leave it. A click on a network iwd knows, or an open
//! one, joins it; one that needs a password iwd doesn't have yet is
//! joined from the Command Center for now (asking for it takes an agent,
//! the next step here).

use lntrn_kit::look;
use lntrn_math::{Rect, Vec2};
use lntrn_status::wifi::{Link, Network, Security, WifiState};
use lntrn_ui::Ui;

use super::Frame;
use super::kit::{self, HEAD_H, LINE_H, ROW_H};
use crate::tray::Tray;
use crate::tray::glyphs::Glyph;

const WIDTH: f64 = 460.0;
const PICTURE: f64 = 28.0;
const LOCK: f64 = 18.0;

/// What a frame's clicks ask of iwd.
enum Ask {
    Power(bool),
    Join(String),
    Leave,
}

/// The lines under the title: what WiFi is doing, the cable, a failure.
fn status(s: &WifiState) -> Vec<(String, bool)> {
    let mut lines = vec![(
        match (s.present, s.powered, s.link, s.ssid.as_deref()) {
            (false, _, _, _) => "No WiFi here (is iwd running?)".to_owned(),
            (_, false, _, _) => "WiFi is off".to_owned(),
            (_, _, Link::Connected, Some(ssid)) => format!("Connected to {ssid}"),
            (_, _, Link::Connecting, Some(ssid)) => format!("Joining {ssid}…"),
            _ => "Not connected".to_owned(),
        },
        false,
    )];
    if s.wired {
        lines.push(("A cable is plugged in".to_owned(), false));
    }
    if let Some(error) = &s.error {
        lines.push((format!("Couldn't: {error}"), true));
    }
    lines
}

/// The networks shown: all in range while it is on.
fn shown(s: &WifiState) -> &[Network] {
    if s.present && s.powered { &s.networks } else { &[] }
}

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let s = tray.wifi.as_ref()?.state();
    let m = ui.m;
    let lines = status(&s);
    let networks = shown(&s);
    let locked = networks.iter().any(|n| !n.joinable());
    let empty = s.present && s.powered && networks.is_empty();
    let h = m.px(HEAD_H) + m.px(LINE_H) * lines.len() as f64 + m.px(8.0) + kit::list_h(ui, networks.len()) + if empty || locked { m.px(LINE_H) } else { 0.0 };
    let (mut col, panel) = frame.open(ui, WIDTH, h);
    let mut ask = None;

    let head = col.take(m.px(HEAD_H));
    if kit::header(ui, head, "WiFi", s.present.then_some(s.powered)) {
        ask = Some(Ask::Power(!s.powered));
    }
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let dim_style = kit::dim_style(ui);
    if s.scanning && s.powered {
        let room = Rect::new(Vec2::new(head.min.x, head.min.y), Vec2::new(head.max.x - m.px(68.0), head.max.y));
        ui.text_right("Scanning…", &dim_style, room, dim);
    }
    for (line, bad) in &lines {
        kit::line(ui, col.take(m.px(LINE_H)), line, &dim_style, if *bad { look::BAD } else { ink });
    }
    col.take(m.px(8.0));

    let list = col.take(kit::list_h(ui, networks.len()));
    let name = kit::name_style(ui);
    let mut scroll = tray.scroll;
    let glyphs = &mut tray.glyphs;
    kit::list(ui, list, networks.len(), m.px(ROW_H), &mut scroll, |ui, i, row| {
        let net = &networks[i];
        let joining = s.joining.as_deref() == Some(net.path.as_str());
        let mut right = row.max.x - m.px(10.0);
        if net.connected {
            ui.draw.rounded_rect(row, m.px(10.0), ui.theme.accent.fade(0.20));
            let (leave, button) = kit::button(ui, ui.id("leave"), right, row.center().y, "Disconnect");
            if leave {
                ask = Some(Ask::Leave);
            }
            right = button.min.x - m.px(10.0);
        } else {
            let r = kit::row(ui, ui.id("network").with_index(i), row, false);
            if r.clicked && net.joinable() && !joining {
                ask = Some(Ask::Join(net.path.clone()));
            }
            let note = if joining { Some("Joining…") } else if !net.joinable() { Some("Needs a password") } else { None };
            if let Some(note) = note {
                let w = ui.measure(note, &dim_style);
                ui.text_right(note, &dim_style, Rect::new(Vec2::new(right - w, row.min.y), Vec2::new(right, row.max.y)), dim);
                right -= w + m.px(10.0);
            }
        }
        let color = if net.joinable() || net.connected { ink } else { dim };
        let pic = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
        glyphs.draw(ui, Glyph::Wifi(bars(net.strength)), pic, color);
        let mut left = pic.max.x + m.px(14.0);
        if net.security != Security::Open {
            let lock = Rect::from_min_size(Vec2::new(left, (row.center().y - m.px(LOCK) * 0.5).round()), Vec2::splat(m.px(LOCK).round()));
            glyphs.draw(ui, Glyph::Lock, lock, dim);
            left = lock.max.x + m.px(8.0);
        }
        kit::line(ui, Rect::new(Vec2::new(left, row.min.y), Vec2::new(right, row.max.y)), &net.name, &name, color);
    });
    tray.scroll = scroll;
    if empty {
        let words = if s.scanning { "Looking for networks…" } else { "No networks in range" };
        kit::line(ui, col.take(m.px(LINE_H)), words, &dim_style, dim);
    } else if locked {
        kit::line(ui, col.take(m.px(LINE_H)), "New networks with a password: join from the Command Center", &dim_style, dim);
    }

    if let (Some(ask), Some(wifi)) = (ask, &tray.wifi) {
        match ask {
            Ask::Power(on) => wifi.set_powered(on),
            Ask::Join(path) => wifi.join(&path),
            Ask::Leave => wifi.leave(),
        }
    }
    Some(panel)
}

/// A network's signal as the fan's lit parts, 1 to 4.
fn bars(strength: u32) -> u8 {
    match strength {
        0..=19 => 1,
        20..=44 => 2,
        45..=69 => 3,
        _ => 4,
    }
}

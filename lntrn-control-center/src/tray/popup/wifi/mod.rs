//! The WiFi popup: the switch, what it is connected to (and a cable, when
//! one is plugged in), and the networks in range, the one in use first
//! with a button to leave it. A click on a network iwd knows, or an open
//! one, joins it; on one that needs a password, a box opens under it to
//! type it in (`entry`). A saved network can be forgotten (a second click
//! on Forget makes sure), and a hidden one joined by its name from the
//! row under the list; under that, the saved networks out of range open
//! in a list of their own, to forget. A company login (802.1X) isn't
//! joined from here.

mod entry;
#[cfg(test)]
mod tests;

pub use entry::{Entry, Open};

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_status::wifi::{Link, Network, Saved, Security, Target, WifiState};
use lntrn_ui::Ui;

use super::Frame;
use super::kit::{self, HEAD_H, LINE_H, MAX_ROWS, ROW_H};
use crate::tray::Tray;
use crate::tray::glyphs::{Glyph, Glyphs};
use entry::{BOX_H, HIDDEN_H};

const WIDTH: f64 = 480.0;
const PICTURE: f64 = 28.0;
const LOCK: f64 = 18.0;
/// Where a saved network's row starts its lock and name, past where the
/// rows above have their pictures, logical.
const INDENT_SAVED: f64 = 52.0;
/// How long a first click on Forget waits for the second, seconds.
const FORGET_WAIT: f64 = 4.0;
/// Saved networks out of range shown before their list scrolls.
const SAVED_ROWS: usize = 4;

/// What a frame's clicks and keys ask of iwd.
#[cfg_attr(test, derive(Debug, PartialEq))]
enum Ask {
    Power(bool),
    Join(String),
    /// A network in range, by path, and the password typed for it.
    JoinWith(String, String),
    /// A hidden network's name, and its password (empty for none).
    JoinHidden(String, String),
    Leave,
    Forget(String),
}

/// The lines under the title: what WiFi is doing, the cable, a failure
/// (but one the open box says itself).
fn status(s: &WifiState, entry: &Entry) -> Vec<(String, bool)> {
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
    if let Some(error) = s.error.as_deref().filter(|_| !entry.claims(s)) {
        let line = match s.failed {
            Some(_) => format!("Couldn't join: {}", lowered(error)),
            None => format!("Couldn't: {error}"),
        };
        lines.push((line, true));
    }
    lines
}

/// `s` going on after a colon: its first letter small.
fn lowered(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect())
}

/// The networks shown: all in range while it is on.
fn shown(s: &WifiState) -> &[Network] {
    if s.present && s.powered { &s.networks } else { &[] }
}

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let s = tray.wifi.as_ref()?.state();
    let (panel, ask) = panel(ui, &s, &mut tray.entry, &mut tray.glyphs, &mut tray.scroll, frame);
    if let (Some(ask), Some(wifi)) = (ask, &tray.wifi) {
        match ask {
            Ask::Power(on) => wifi.set_powered(on),
            Ask::Join(path) => wifi.join(&path),
            Ask::JoinWith(path, password) => wifi.join_with_password(&path, password),
            Ask::JoinHidden(name, password) => wifi.join_hidden(&name, password),
            Ask::Leave => wifi.leave(),
            Ask::Forget(path) => wifi.forget(&path),
        }
    }
    Some(panel)
}

/// The popup for `s`, typed into `entry`, its list scrolled by `scroll`.
/// Returns the panel, and what was asked.
fn panel(ui: &mut Ui, s: &WifiState, entry: &mut Entry, glyphs: &mut Glyphs, scroll: &mut f64, frame: &Frame) -> (Rect, Option<Ask>) {
    entry.follow(s);
    let m = ui.m;
    let lines = status(s, entry);
    let networks = shown(s);
    let on = s.present && s.powered;
    // Whose box is open as the frame starts: a click that opens or closes
    // one shows from the next.
    let open_path = match &entry.open {
        Some(Open::Network(path)) => Some(path.clone()),
        _ => None,
    };
    let boxed = |n: &Network| open_path.as_deref() == Some(n.path.as_str());
    let heights: Vec<f64> = networks.iter().map(|n| m.px(ROW_H) + if boxed(n) { m.px(BOX_H) } else { 0.0 }).collect();
    let open_at = networks.iter().position(boxed);
    // As many rows as ever, and the box besides.
    let most = m.px(ROW_H) * MAX_ROWS as f64 + if open_at.is_some() { m.px(BOX_H) } else { 0.0 };
    let list_h = heights.iter().sum::<f64>().min(most);
    let empty = on && networks.is_empty();
    let hidden_h = match (on, entry.open == Some(Open::Hidden)) {
        (false, _) => 0.0,
        (true, open) => m.px(4.0) + m.px(if open { HIDDEN_H } else { ROW_H }),
    };
    let saved = if on { s.saved.as_slice() } else { &[] };
    let saved_h = match (saved.is_empty(), entry.saved_open) {
        (true, _) => 0.0,
        (false, false) => m.px(ROW_H),
        (false, true) => m.px(ROW_H) * (1 + saved.len().min(SAVED_ROWS)) as f64,
    };
    let h = m.px(HEAD_H) + m.px(LINE_H) * lines.len() as f64 + m.px(8.0) + list_h + if empty { m.px(LINE_H) } else { 0.0 } + hidden_h + saved_h;
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

    let list = col.take(list_h);
    if let Some(at) = open_at.filter(|_| std::mem::take(&mut entry.reveal)) {
        *scroll = kit::scrolled_to(&heights, at, list_h, *scroll);
    }
    let name = kit::name_style(ui);
    let now = ui.now();
    kit::list_rows(ui, list, &heights, scroll, |ui, i, rect| {
        let net = &networks[i];
        let row = Rect::from_min_size(rect.min, Vec2::new(rect.width(), m.px(ROW_H)));
        let joining = s.joining.as_ref().is_some_and(|t| *t == Target::Network(net.path.clone()));
        let open = boxed(net);
        let mut right = row.max.x - m.px(10.0);
        if net.connected {
            ui.draw.rounded_rect(row, m.px(10.0), ui.theme.accent.fade(0.20));
            let (leave, button) = kit::button(ui, ui.id("leave"), right, row.center().y, "Disconnect");
            if leave {
                ask = Some(Ask::Leave);
            }
            right = button.min.x - m.px(10.0);
        }
        // Forget goes down before the row, so the row doesn't take its click.
        let over = ui.state.pointer_in_window && list.contains(ui.state.pointer) && row.contains(ui.state.pointer);
        if net.known && (net.connected || over || armed(entry, &net.path, now)) {
            let (forget, left) = forget_button(ui, entry, &net.path, right, row.center().y, now);
            if forget.is_some() {
                ask = forget;
            }
            right = left;
        }
        if !net.connected {
            if open {
                ui.draw.rounded_rect(rect, m.px(12.0), Color::WHITE.fade(0.05));
            }
            let r = kit::row(ui, ui.id("network").with(net.path.as_str()), row, false);
            if r.clicked && !joining {
                if net.joinable() {
                    ask = Some(Ask::Join(net.path.clone()));
                } else if net.asks_password() {
                    entry.toggle(Open::Network(net.path.clone()));
                    ui.state.request_rebuild = true;
                }
            }
            let note = match () {
                _ if joining => Some("Joining…"),
                _ if open || net.joinable() => None,
                _ if net.asks_password() => Some("Needs a password"),
                _ if net.security == Security::Enterprise => Some("Company login"),
                _ => Some("Can't join from here"),
            };
            if let Some(note) = note {
                let w = ui.measure(note, &dim_style);
                ui.text_right(note, &dim_style, Rect::new(Vec2::new(right - w, row.min.y), Vec2::new(right, row.max.y)), dim);
                right -= w + m.px(10.0);
            }
        }
        let color = if net.joinable() || net.connected || net.asks_password() { ink } else { dim };
        let pic = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
        glyphs.draw(ui, Glyph::Wifi(bars(net.strength)), pic, color);
        let mut left = pic.max.x + m.px(14.0);
        if net.security != Security::Open {
            let lock = Rect::from_min_size(Vec2::new(left, (row.center().y - m.px(LOCK) * 0.5).round()), Vec2::splat(m.px(LOCK).round()));
            glyphs.draw(ui, Glyph::Lock, lock, dim);
            left = lock.max.x + m.px(8.0);
        }
        kit::line(ui, Rect::new(Vec2::new(left, row.min.y), Vec2::new(right, row.max.y)), &net.name, &name, color);
        if open {
            let under = Rect::new(Vec2::new(rect.min.x, row.max.y), rect.max);
            if let Some(typed) = entry.password_box(ui, glyphs, under, &net.path, s) {
                ask = Some(typed);
            }
        }
    });
    if empty {
        let words = if s.scanning { "Looking for networks…" } else { "No networks in range" };
        kit::line(ui, col.take(m.px(LINE_H)), words, &dim_style, dim);
    }
    if on {
        col.take(m.px(4.0));
        if let Some(typed) = entry.hidden(ui, glyphs, col.take(hidden_h - m.px(4.0)), s) {
            ask = Some(typed);
        }
    }
    if !saved.is_empty() {
        let away = saved_list(ui, entry, glyphs, &mut col, saved, now);
        if away.is_some() {
            ask = away;
        }
    }
    (panel, ask)
}

/// The row that opens the list of `saved` networks out of range, and the
/// list while it's open, each with Forget. Returns what was asked.
fn saved_list(ui: &mut Ui, entry: &mut Entry, glyphs: &mut Glyphs, col: &mut kit::Column, saved: &[Saved], now: f64) -> Option<Ask> {
    let m = ui.m;
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let (name, dim_style) = (kit::name_style(ui), kit::dim_style(ui));
    let head = col.take(m.px(ROW_H));
    if kit::row(ui, ui.id("saved"), head, entry.saved_open).clicked {
        entry.saved_open = !entry.saved_open;
        ui.state.request_rebuild = true;
    }
    let pic = Rect::from_min_size(Vec2::new(head.min.x + m.px(10.0), (head.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
    glyphs.draw(ui, Glyph::Wifi(0), pic, ink);
    let count = if saved.len() == 1 { "1 not in range".to_owned() } else { format!("{} not in range", saved.len()) };
    let w = ui.measure(&count, &dim_style);
    let right = head.max.x - m.px(10.0);
    ui.text_right(&count, &dim_style, Rect::new(Vec2::new(right - w, head.min.y), Vec2::new(right, head.max.y)), dim);
    kit::line(ui, Rect::new(Vec2::new(pic.max.x + m.px(14.0), head.min.y), Vec2::new(right - w - m.px(10.0), head.max.y)), "Saved networks…", &name, ink);
    if !entry.saved_open {
        return None;
    }
    let heights = vec![m.px(ROW_H); saved.len()];
    let list = col.take(m.px(ROW_H) * saved.len().min(SAVED_ROWS) as f64);
    let mut scroll = entry.saved_scroll;
    let mut ask = None;
    kit::list_rows(ui, list, &heights, &mut scroll, |ui, i, row| {
        let n = &saved[i];
        let (forget, left) = forget_button(ui, entry, &n.path, row.max.x - m.px(10.0), row.center().y, now);
        if forget.is_some() {
            ask = forget;
        }
        let lock = Rect::from_min_size(Vec2::new(row.min.x + m.px(INDENT_SAVED), (row.center().y - m.px(LOCK) * 0.5).round()), Vec2::splat(m.px(LOCK).round()));
        let mut right = left;
        if n.hidden {
            let w = ui.measure("Hidden", &dim_style);
            ui.text_right("Hidden", &dim_style, Rect::new(Vec2::new(right - w, row.min.y), Vec2::new(right, row.max.y)), dim);
            right -= w + m.px(10.0);
        }
        let mut text_x = row.min.x + m.px(INDENT_SAVED);
        if n.security != Security::Open {
            glyphs.draw(ui, Glyph::Lock, lock, dim);
            text_x = lock.max.x + m.px(8.0);
        }
        kit::line(ui, Rect::new(Vec2::new(text_x, row.min.y), Vec2::new(right, row.max.y)), &n.name, &name, dim);
    });
    entry.saved_scroll = scroll;
    ask
}

/// Whether Forget was pressed once on the network at `path`, a moment
/// ago.
fn armed(entry: &Entry, path: &str, now: f64) -> bool {
    entry.forgetting.as_ref().is_some_and(|(p, at)| p == path && now - at < FORGET_WAIT)
}

/// Forget, its right end at `right` on a row centred on `cy`: once to
/// arm it, again to forget the network at `path`. Returns what it asks,
/// and where the room left of it ends.
fn forget_button(ui: &mut Ui, entry: &mut Entry, path: &str, right: f64, cy: f64, now: f64) -> (Option<Ask>, f64) {
    let ready = armed(entry, path, now);
    let (label, ink) = if ready { ("Sure? Forget", look::BAD) } else { ("Forget", ui.theme.text) };
    let (clicked, button) = kit::button_with(ui, ui.id("forget").with(path), right, cy, label, Some(ink));
    if let Some((_, at)) = entry.forgetting.as_ref().filter(|_| ready) {
        ui.state.request_redraw_after(FORGET_WAIT - (now - at));
    }
    let ask = match clicked {
        true if ready => {
            entry.forgetting = None;
            Some(Ask::Forget(path.to_owned()))
        }
        true => {
            entry.forgetting = Some((path.to_owned(), now));
            ui.state.request_rebuild = true;
            None
        }
        false => None,
    };
    (ask, button.min.x - ui.m.px(10.0))
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

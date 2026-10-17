//! The Bluetooth popup: the switch, and the paired devices, connected
//! first, each with its kind's picture, its battery where it says, and
//! what it is doing; a click connects one, or disconnects it. Under the
//! pointer a device offers Send a file (when it takes them) and Forget (a
//! second click makes sure). "Add a device…" under them looks for new
//! ones, and makes this computer seen by them meanwhile; a click on one
//! pairs it. What pairing asks, a file a device wants to send, and files
//! on their way show under the device's row (`strips`), or in a card up
//! top for a device in neither list (a phone pairing from its end).

mod strips;
#[cfg(test)]
mod tests;

pub use strips::Entry;

use lntrn_kit::look;
use lntrn_math::{Rect, Vec2};
use lntrn_status::bluetooth::{BluetoothState, Device, Reply};
use lntrn_ui::Ui;

use super::Frame;
use super::kit::{self, HEAD_H, LINE_H, MAX_ROWS, ROW_H};
use crate::tray::Tray;
use crate::tray::glyphs::{Glyph, Glyphs};
use strips::{STRIP_H, TRANSFER_H};

const WIDTH: f64 = 500.0;
const PICTURE: f64 = 28.0;
/// Rows of devices nearby shown before the list scrolls.
const NEARBY_ROWS: usize = 5;
/// How long a first click on Forget waits for the second, seconds.
const FORGET_WAIT: f64 = 4.0;
/// Transfers shown under one device, at most.
const TRANSFERS: usize = 2;

/// What a frame's clicks and keys ask of BlueZ and obexd.
#[cfg_attr(test, derive(Debug, PartialEq))]
enum Ask {
    Power(bool),
    Connect(String, bool),
    Scan(bool),
    Pair(String),
    Answer(u64, Reply),
    Forget(String),
    /// Pick a file to send to the device at this address, called this.
    SendFile(String, String),
    TakeFile(u64, bool),
}

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let s = tray.bluetooth.as_ref()?.state();
    let (panel, ask) = panel(ui, &s, &mut tray.bt, &mut tray.glyphs, &mut tray.scroll, frame);
    let ask = match ask {
        // The picker is a window: the popup goes, so it can be seen.
        Some(Ask::SendFile(address, name)) => {
            tray.pick_file(&address, &name);
            return None;
        }
        other => other,
    };
    if let (Some(ask), Some(bt)) = (ask, &tray.bluetooth) {
        match ask {
            Ask::Power(on) => bt.set_powered(on),
            Ask::Connect(path, on) => bt.connect(&path, on),
            Ask::Scan(on) => bt.scan(on),
            Ask::Pair(path) => bt.pair(&path),
            Ask::Answer(id, reply) => bt.answer(id, reply),
            Ask::Forget(path) => bt.forget(&path),
            Ask::TakeFile(id, yes) => bt.take_file(id, yes),
            Ask::SendFile(..) => {}
        }
    }
    Some(panel)
}

/// How tall `d`'s row is, with what shows under it.
fn row_h(ui: &Ui, s: &BluetoothState, d: &Device) -> f64 {
    let m = ui.m;
    let asked = s.question.as_ref().is_some_and(|q| q.device == d.path);
    let offered = s.offer.as_ref().is_some_and(|o| o.device.eq_ignore_ascii_case(&d.address));
    let transfers = s.transfers.iter().filter(|t| t.device.eq_ignore_ascii_case(&d.address) && strips::shows(t)).count().min(TRANSFERS);
    m.px(ROW_H) + if asked { m.px(STRIP_H) } else { 0.0 } + if offered { m.px(STRIP_H) } else { 0.0 } + m.px(TRANSFER_H) * transfers as f64
}

/// How much of a list of rows `heights` tall shows: `rows` rows of a
/// row's height, and the one tallest row's extra besides.
fn shown_h(ui: &Ui, heights: &[f64], rows: usize) -> f64 {
    let extra = heights.iter().map(|h| h - ui.m.px(ROW_H)).fold(0.0, f64::max);
    heights.iter().sum::<f64>().min(ui.m.px(ROW_H) * rows as f64 + extra)
}

/// The popup for `s`, typed into `entry`, the paired list scrolled by
/// `scroll`. Returns the panel, and what was asked.
fn panel(ui: &mut Ui, s: &BluetoothState, entry: &mut Entry, glyphs: &mut Glyphs, scroll: &mut f64, frame: &Frame) -> (Rect, Option<Ask>) {
    let m = ui.m;
    let now = ui.now();
    let on = s.present && s.powered;
    let devices = if on { s.devices.as_slice() } else { &[] };
    let nearby = if on && entry.adding { s.nearby.as_slice() } else { &[] };
    let mut lines: Vec<(String, bool)> = Vec::new();
    match (s.present, s.powered) {
        (false, _) => lines.push(("No Bluetooth here".to_owned(), false)),
        (_, false) => lines.push(("Bluetooth is off".to_owned(), false)),
        _ if devices.is_empty() => lines.push(("No paired devices yet".to_owned(), false)),
        _ => {}
    }
    if let Some(error) = &s.error {
        lines.push((format!("Couldn't: {error}"), true));
    }
    // A question or an offer about a device in neither list shows up top.
    let listed = |path: &str| devices.iter().chain(nearby).any(|d| d.path == path);
    let lone_question = s.question.as_ref().filter(|q| !listed(&q.device));
    let lone_offer = s.offer.as_ref().filter(|o| !devices.iter().chain(nearby).any(|d| d.address.eq_ignore_ascii_case(&o.device)));
    let lone_h = |there: bool| if there { m.px(LINE_H) + m.px(STRIP_H) } else { 0.0 };
    let heights: Vec<f64> = devices.iter().map(|d| row_h(ui, s, d)).collect();
    let near_heights: Vec<f64> = nearby.iter().map(|d| row_h(ui, s, d)).collect();
    let list_h = shown_h(ui, &heights, MAX_ROWS);
    let near_h = shown_h(ui, &near_heights, NEARBY_ROWS);
    let adding_h = match (on, entry.adding) {
        (false, _) => 0.0,
        (true, false) => m.px(4.0 + ROW_H),
        (true, true) => m.px(4.0 + ROW_H) + near_h + m.px(LINE_H) * if nearby.is_empty() { 2.0 } else { 1.0 },
    };
    let h = m.px(HEAD_H) + m.px(LINE_H) * lines.len() as f64 + lone_h(lone_question.is_some()) + lone_h(lone_offer.is_some()) + m.px(8.0) + list_h + adding_h;
    let (mut col, panel) = frame.open(ui, WIDTH, h);
    let mut ask = None;

    if kit::header(ui, col.take(m.px(HEAD_H)), "Bluetooth", s.present.then_some(s.powered)) {
        ask = Some(Ask::Power(!s.powered));
    }
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    let dim_style = kit::dim_style(ui);
    let name = kit::name_style(ui);
    for (line, bad) in &lines {
        kit::line(ui, col.take(m.px(LINE_H)), line, &dim_style, if *bad { look::BAD } else { ink });
    }
    if let Some(q) = lone_question {
        kit::line(ui, col.take(m.px(LINE_H)), &q.name, &name.clone().bold(), ink);
        ask = strips::question(ui, entry, col.take(m.px(STRIP_H)), q).or(ask);
    }
    if let Some(o) = lone_offer {
        kit::line(ui, col.take(m.px(LINE_H)), &o.from, &name.clone().bold(), ink);
        ask = strips::offer(ui, col.take(m.px(STRIP_H)), o).or(ask);
    }
    col.take(m.px(8.0));

    let list = col.take(list_h);
    kit::list_rows(ui, list, &heights, scroll, |ui, i, rect| {
        let d = &devices[i];
        let row = Rect::from_min_size(rect.min, Vec2::new(rect.width(), m.px(ROW_H)));
        let busy = s.busy.contains(&d.path);
        let over = ui.state.pointer_in_window && list.contains(ui.state.pointer) && row.contains(ui.state.pointer);
        let mut right = row.max.x - m.px(12.0);
        // Its buttons go down before the row, so the row doesn't take
        // their clicks.
        let handy = over || armed(entry, &d.path, now);
        if handy {
            let (forget, left) = forget_button(ui, entry, &d.path, right, row.center().y, now);
            right = left;
            if forget.is_some() {
                ask = forget;
            }
            if d.takes_files {
                let (send, button) = kit::button(ui, ui.id("send").with(d.path.as_str()), right, row.center().y, "Send a file");
                right = button.min.x - m.px(10.0);
                if send {
                    ask = Some(Ask::SendFile(d.address.clone(), d.name.clone()));
                }
            }
        }
        let r = kit::row(ui, ui.id("device").with(d.path.as_str()), row, d.connected);
        if r.clicked && !busy {
            ask = Some(Ask::Connect(d.path.clone(), !d.connected));
        }
        let pic = picture(ui, glyphs, row, d);
        if !handy {
            let doing = match (busy, d.connected) {
                (true, true) => Some(("Disconnecting…", dim)),
                (true, false) => Some(("Connecting…", dim)),
                (false, true) => Some(("Connected", accent)),
                (false, false) => None,
            };
            let battery = d.battery.map(|p| (format!("{p}%"), dim));
            for (text, color) in doing.map(|(t, c)| (t.to_owned(), c)).into_iter().chain(battery) {
                let w = ui.measure(&text, &dim_style);
                ui.text_right(&text, &dim_style, Rect::new(Vec2::new(right - w, row.min.y), Vec2::new(right, row.max.y)), color);
                right -= w + m.px(12.0);
            }
        }
        kit::line(ui, Rect::new(Vec2::new(pic.max.x + m.px(14.0), row.min.y), Vec2::new(right, row.max.y)), &d.name, &name, ink);
        if let Some(typed) = under(ui, entry, s, d, Rect::new(Vec2::new(rect.min.x, row.max.y), rect.max)) {
            ask = Some(typed);
        }
    });

    if on {
        col.take(m.px(4.0));
        let add = col.take(m.px(ROW_H));
        if kit::row(ui, ui.id("add"), add, entry.adding).clicked {
            entry.adding = !entry.adding;
            ask = Some(Ask::Scan(entry.adding));
            ui.state.request_rebuild = true;
        }
        let pic = Rect::from_min_size(Vec2::new(add.min.x + m.px(10.0), (add.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
        glyphs.draw(ui, Glyph::Plus, pic, ink);
        let mut right = add.max.x - m.px(12.0);
        if entry.adding && s.scanning {
            let w = ui.measure("Looking…", &dim_style);
            ui.text_right("Looking…", &dim_style, Rect::new(Vec2::new(right - w, add.min.y), Vec2::new(right, add.max.y)), dim);
            right -= w + m.px(12.0);
        }
        kit::line(ui, Rect::new(Vec2::new(pic.max.x + m.px(14.0), add.min.y), Vec2::new(right, add.max.y)), "Add a device…", &name, ink);
    }
    if on && entry.adding {
        let near = col.take(near_h);
        let mut near_scroll = entry.nearby_scroll;
        kit::list_rows(ui, near, &near_heights, &mut near_scroll, |ui, i, rect| {
            let d = &nearby[i];
            let row = Rect::from_min_size(rect.min, Vec2::new(rect.width(), m.px(ROW_H)));
            let pairing = s.pairing.as_deref() == Some(d.path.as_str());
            let r = kit::row(ui, ui.id("nearby").with(d.path.as_str()), row, pairing);
            if r.clicked && s.pairing.is_none() {
                ask = Some(Ask::Pair(d.path.clone()));
            }
            let pic = picture(ui, glyphs, row, d);
            let mut right = row.max.x - m.px(12.0);
            if pairing {
                let w = ui.measure("Pairing…", &dim_style);
                ui.text_right("Pairing…", &dim_style, Rect::new(Vec2::new(right - w, row.min.y), Vec2::new(right, row.max.y)), dim);
                right -= w + m.px(12.0);
            }
            kit::line(ui, Rect::new(Vec2::new(pic.max.x + m.px(14.0), row.min.y), Vec2::new(right, row.max.y)), &d.name, &name, ink);
            if let Some(typed) = under(ui, entry, s, d, Rect::new(Vec2::new(rect.min.x, row.max.y), rect.max)) {
                ask = Some(typed);
            }
        });
        entry.nearby_scroll = near_scroll;
        if nearby.is_empty() {
            kit::line(ui, col.take(m.px(LINE_H)), "Looking for devices nearby…", &dim_style, dim);
        }
        let seen = if s.name.is_empty() { "This computer can be seen while this is open".to_owned() } else { format!("Seen as “{}” while this is open", s.name) };
        kit::line(ui, col.take(m.px(LINE_H)), &seen, &dim_style, dim);
    }
    // An ended transfer stops showing in a while.
    if let Some(left) = s.transfers.iter().filter_map(|t| t.end.as_ref()).map(|(_, at)| strips::ENDED_SHOWS.saturating_sub(at.elapsed())).filter(|d| !d.is_zero()).min() {
        ui.state.request_redraw_after(left.as_secs_f64());
    }
    (panel, ask)
}

/// `d`'s picture at the left of `row`. Returns where it went.
fn picture(ui: &mut Ui, glyphs: &mut Glyphs, row: Rect, d: &Device) -> Rect {
    let m = ui.m;
    let pic = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
    glyphs.draw(ui, Glyph::for_device(&d.icon), pic, ui.theme.text);
    pic
}

/// What shows under `d`'s row, down from the top of `rect`: what pairing
/// it asks, a file it offers, files on their way. Returns what was asked.
fn under(ui: &mut Ui, entry: &mut Entry, s: &BluetoothState, d: &Device, rect: Rect) -> Option<Ask> {
    let m = ui.m;
    let mut y = rect.min.y;
    let mut take = |h: f64| {
        let r = Rect::new(Vec2::new(rect.min.x, y), Vec2::new(rect.max.x, y + m.px(h)));
        y += m.px(h);
        r
    };
    let mut ask = None;
    if let Some(q) = s.question.as_ref().filter(|q| q.device == d.path) {
        ask = strips::question(ui, entry, take(STRIP_H), q);
    }
    if let Some(o) = s.offer.as_ref().filter(|o| o.device.eq_ignore_ascii_case(&d.address)) {
        ask = strips::offer(ui, take(STRIP_H), o).or(ask);
    }
    for t in s.transfers.iter().filter(|t| t.device.eq_ignore_ascii_case(&d.address) && strips::shows(t)).take(TRANSFERS) {
        strips::transfer(ui, take(TRANSFER_H), t);
    }
    ask
}

/// Whether Forget was pressed once on the device at `path`, a moment ago.
fn armed(entry: &Entry, path: &str, now: f64) -> bool {
    entry.forgetting.as_ref().is_some_and(|(p, at)| p == path && now - at < FORGET_WAIT)
}

/// Forget, its right end at `right` on a row centred on `cy`: once to
/// arm it, again to unpair the device at `path`. Returns what it asks,
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

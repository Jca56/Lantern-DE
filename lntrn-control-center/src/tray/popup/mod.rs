//! The tray's popups, in the overlay above the bar: a panel over its
//! item's face, as wide as it needs and as tall as its rows, kept on the
//! screen. The battery's details and charge limit; WiFi's switch and
//! networks; Bluetooth's switch and devices; the volume and the outputs;
//! the brightness. A click off the panel, or Escape, puts it away (the
//! WiFi box being typed in first, while there is one).

mod audio;
mod battery;
mod bluetooth;
mod brightness;
pub mod kit;
mod wifi;

pub use bluetooth::Entry as BtEntry;
pub use wifi::Entry;

use lntrn_math::{Rect, Vec2};
use lntrn_status::battery::Charge;
use lntrn_ui::{Key, Ui};

use super::{Item, Tray};
use crate::bar::Look;
use kit::Column;

/// Where a popup goes: centred over `x` (its item's face), its bottom at
/// `bottom` (window pixels).
pub struct Frame {
    pub x: f64,
    pub bottom: f64,
    pub look: Look,
}

impl Frame {
    /// Draw a panel `w` logical wide around `h` pixels of rows. Returns
    /// the column inside it, and the panel.
    pub fn open(&self, ui: &mut Ui, w: f64, h: f64) -> (Column, Rect) {
        let m = ui.m;
        let surface = ui.clip();
        let (edge, pad) = (m.px(8.0), m.px(kit::PAD));
        let w = m.px(w).min(surface.width() - edge * 2.0);
        let h = (h + pad * 2.0).min(self.bottom - surface.min.y - edge);
        let left = (self.x - w * 0.5).clamp(surface.min.x + edge, (surface.max.x - edge - w).max(surface.min.x + edge)).round();
        let panel = Rect::new(Vec2::new(left, (self.bottom - h).round()), Vec2::new(left + w, self.bottom.round()));
        kit::panel(ui, panel, self.look);
        (Column::new(panel.shrink(pad)), panel)
    }
}

/// Draw `item`'s popup. Returns its panel, or `None` when it is to go
/// (clicked off, Escaped, or its item gone).
pub fn draw(tray: &mut Tray, ui: &mut Ui, item: Item, frame: &Frame) -> Option<Rect> {
    if ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() && !escape(tray, item) {
        return None;
    }
    let panel = match item {
        Item::Battery => battery::draw(tray, ui, frame),
        Item::Wifi => wifi::draw(tray, ui, frame),
        Item::Bluetooth => bluetooth::draw(tray, ui, frame),
        Item::Audio => audio::draw(tray, ui, frame),
        Item::Brightness => brightness::draw(tray, ui, frame),
        Item::Apps | Item::Workspace | Item::GameMode => None,
    }?;
    // Its own rows took their clicks; any other is off it.
    if ui.state.pressed && !panel.contains(ui.state.pointer) {
        return None;
    }
    Some(panel)
}

/// Escape on `item`'s popup: closes the WiFi box being typed in, while
/// there is one. Returns whether the popup stays.
fn escape(tray: &mut Tray, item: Item) -> bool {
    let typing = item == Item::Wifi && tray.entry.open.is_some();
    if typing {
        tray.entry.close();
    }
    typing
}

/// How long a battery has left, as said: `3 h 20 min left`, `Full in
/// 45 min`.
pub fn duration(secs: u64, charge: Charge) -> String {
    let mins = (secs + 30) / 60;
    let span = match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    };
    match charge {
        Charge::Charging => format!("Full in {span}"),
        _ => format!("{span} left"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_closes_the_wifi_box_before_the_popup() {
        let mut tray = Tray::default();
        tray.entry.toggle(wifi::Open::Hidden);
        assert!(escape(&mut tray, Item::Wifi), "the box goes, the popup stays");
        assert!(tray.entry.open.is_none());
        assert!(!escape(&mut tray, Item::Wifi) && !escape(&mut tray, Item::Audio));
    }

    #[test]
    fn time_left_reads_as_said() {
        assert_eq!(duration(3 * 3600 + 20 * 60, Charge::Discharging), "3 h 20 min left");
        assert_eq!(duration(45 * 60 + 10, Charge::Charging), "Full in 45 min");
        assert_eq!(duration(2 * 3600, Charge::Discharging), "2 h left");
    }
}

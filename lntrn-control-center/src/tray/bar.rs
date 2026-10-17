//! The tray's faces on the bar, in their order up to the clock: a
//! picture each on a face as tall as the dock's (lit under the pointer,
//! in the accent while its popup is up), the battery wider for its
//! percent, the workspace's number in a box of its own. Hold Alt over one
//! for what it says in words.

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_status::audio::AudioState;
use lntrn_status::battery::{BatteryState, Charge};
use lntrn_status::bluetooth::BluetoothState;
use lntrn_status::wifi::{Link, WifiState};
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::glyphs::Glyph;
use super::{Item, Tray, TrayOptions};
use crate::compositor::{self, Workspaces};

/// Between a face and the plate's top and bottom, logical: the dock's.
const MARGIN: f64 = 4.0;
/// A face's width, and its picture's side, logical.
const CELL: f64 = 50.0;
const GLYPH: f64 = 30.0;
const RADIUS: f64 = 12.0;
/// Between faces, logical.
const GAP: f64 = 2.0;
/// Inside the battery's face, either side; between its parts.
const PAD: f64 = 10.0;
const SPACING: f64 = 7.0;
/// The battery's percent and the workspace's number, logical.
const PERCENT: f64 = 20.0;
const DIGIT: f64 = 22.0;
/// The workspace's box, logical.
const BOX: f64 = 36.0;
/// Below this the battery shows red, and below `LOW` amber (the Command
/// Center's zones).
const CRITICAL: u32 = 25;
const LOW: u32 = 50;

/// What a frame's clicks on the tray ask of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayPress {
    /// Open the item's popup, or close it when it is up.
    Toggle(Item),
}

/// What one frame of the tray did.
pub struct TrayFrame {
    /// The room it took on the plate (none: a zero-width rect at the
    /// right edge it was given).
    pub rect: Rect,
    pub press: Option<TrayPress>,
    /// The pointer went down on one of its faces.
    pub pressed: bool,
}

/// This frame's state of every item that shows.
struct Seen {
    battery: Option<BatteryState>,
    wifi: Option<WifiState>,
    bluetooth: Option<BluetoothState>,
    audio: Option<AudioState>,
    brightness: bool,
    workspaces: Option<Workspaces>,
    game: Option<bool>,
}

impl Seen {
    fn read(tray: &Tray, options: TrayOptions, link: &compositor::Link) -> Seen {
        let on = |item| options.shows(item);
        Seen {
            battery: tray.battery.as_ref().filter(|_| on(Item::Battery)).map(|b| b.state()).filter(|s| s.present),
            wifi: tray.wifi.as_ref().filter(|_| on(Item::Wifi)).map(|w| w.state()).filter(|s| s.present || s.wired),
            bluetooth: tray.bluetooth.as_ref().filter(|_| on(Item::Bluetooth)).map(|b| b.state()).filter(|s| s.present),
            audio: tray.audio.as_ref().filter(|_| on(Item::Audio)).map(|a| a.state()).filter(|s| s.present),
            brightness: on(Item::Brightness) && tray.backlight.as_ref().is_some_and(|b| b.state().present),
            workspaces: link.workspaces().filter(|_| on(Item::Workspace)),
            game: on(Item::GameMode).then(|| tray.game.on()).flatten(),
        }
    }

    fn shows(&self, item: Item, options: TrayOptions) -> bool {
        match item {
            Item::Battery => self.battery.is_some(),
            Item::Wifi => self.wifi.is_some(),
            Item::Bluetooth => self.bluetooth.is_some(),
            Item::Audio => self.audio.is_some(),
            Item::Brightness => self.brightness,
            Item::Workspace => self.workspaces.is_some(),
            // Dim while the compositor hasn't said, but there.
            Item::GameMode => options.game_mode,
        }
    }
}

/// Draw the tray on `plate` (window pixels), its right end at `right`,
/// on a plate of colour `under`.
pub fn draw(tray: &mut Tray, ui: &mut Ui, plate: Rect, right: f64, under: Color, options: TrayOptions, link: &compositor::Link) -> TrayFrame {
    let m = ui.m;
    let seen = Seen::read(tray, options, link);
    let items: Vec<Item> = options.order.into_iter().filter(|&i| seen.shows(i, options)).collect();
    tray.faces.clear();
    let mut frame = TrayFrame { rect: Rect::new(Vec2::new(right, plate.min.y), Vec2::new(right, plate.max.y)), press: None, pressed: false };
    if items.is_empty() {
        return frame;
    }
    let h = (plate.height() - m.px(MARGIN * 2.0)).max(m.px(24.0)).round();
    let g = m.px(GLYPH).min(h * 0.62).round();
    let percent = TextStyle::new(m.px(PERCENT) as f32).bold();
    let widths: Vec<f64> = items.iter().map(|&i| width(ui, i, &seen, options, g, &percent)).collect();
    let total = widths.iter().sum::<f64>() + m.px(GAP) * (items.len() - 1) as f64;
    let top = (plate.center().y - h * 0.5).round();
    let mut x = (right - total).round();
    frame.rect = Rect::new(Vec2::new(x, plate.min.y), Vec2::new(right, plate.max.y));

    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let mut hovered = None;
    for (&item, &w) in items.iter().zip(&widths) {
        let face = Rect::from_min_size(Vec2::new(x, top), Vec2::new(w, h));
        x += w + m.px(GAP);
        let r = ui.interact(ui.id("tray").with_index(item as usize), face, Sense::CLICK);
        frame.pressed |= r.pressed;
        let game_on = item == Item::GameMode && seen.game == Some(true);
        let fill = if tray.popup == Some(item) {
            Some(accent.fade(0.22))
        } else if r.held {
            Some(under.lerp(Color::WHITE, 0.16))
        } else if r.hovered {
            Some(under.lerp(Color::WHITE, 0.11))
        } else if game_on {
            Some(accent.fade(0.16))
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.draw.rounded_rect(face, m.px(RADIUS).min(h * 0.3), fill);
        }
        let pic = Rect::from_center_size(face.center(), Vec2::splat(g)).round();
        let tip = match item {
            Item::Battery => seen.battery.as_ref().map(|s| battery(tray, ui, face, g, s, options.battery_percent, &percent)),
            Item::Wifi => seen.wifi.as_ref().map(|s| {
                tray.glyphs.draw(ui, wifi_glyph(s), pic, ink);
                wifi_tip(s)
            }),
            Item::Bluetooth => seen.bluetooth.as_ref().map(|s| {
                tray.glyphs.draw(ui, Glyph::Bluetooth, pic, if s.powered { ink } else { ink.fade(0.4) });
                if s.powered && s.connected() > 0 {
                    ui.draw.circle(Vec2::new(pic.max.x - g * 0.1, pic.max.y - g * 0.1), g * 0.13, accent);
                }
                bluetooth_tip(s)
            }),
            Item::Audio => seen.audio.as_ref().map(|s| {
                tray.glyphs.draw(ui, speaker_glyph(s), pic, ink);
                if s.muted { "Sound muted".to_owned() } else { format!("Volume {}%", pct(s.volume)) }
            }),
            Item::Brightness => {
                tray.glyphs.draw(ui, Glyph::Sun, pic, ink);
                tray.backlight.as_ref().map(|b| format!("Brightness {}%", pct(b.state().level)))
            }
            Item::Workspace => seen.workspaces.as_ref().map(|ws| {
                workspace(ui, face, ws.active, r.hovered, accent);
                format!("Workspace {}", ws.active)
            }),
            Item::GameMode => {
                let color = match seen.game {
                    Some(true) => accent,
                    Some(false) => ink,
                    None => ink.fade(0.35),
                };
                tray.glyphs.draw(ui, Glyph::Gamepad, pic, color);
                Some(match seen.game {
                    Some(true) => "Game Mode is on".to_owned(),
                    Some(false) => "Game Mode is off".to_owned(),
                    None => "Game Mode (the compositor isn't answering)".to_owned(),
                })
            }
        };
        if let Some(tip) = tip {
            ui.tooltip(&r, &tip);
        }
        if r.clicked {
            match item {
                Item::Workspace => cycle(link, seen.workspaces.as_ref(), 1),
                Item::GameMode => tray.game.toggle(),
                _ => frame.press = Some(TrayPress::Toggle(item)),
            }
        }
        if r.hovered {
            hovered = Some(item);
            if item == Item::Audio
                && ui.state.middle_pressed
                && let Some(audio) = &tray.audio
            {
                audio.toggle_mute(true);
            }
            let notches = wheel(tray, ui, item);
            if notches != 0 {
                match (item, &tray.audio, &tray.backlight) {
                    (Item::Audio, Some(audio), _) => audio.step(notches, true),
                    (Item::Brightness, _, Some(backlight)) => backlight.step(notches, true),
                    // Wheel down (towards you) is the next one.
                    (Item::Workspace, _, _) => cycle(link, seen.workspaces.as_ref(), -notches),
                    _ => {}
                }
            }
        }
        tray.faces.push((item, face));
    }
    if tray.wheel.is_some_and(|(item, _)| hovered != Some(item)) {
        tray.wheel = None;
    }
    frame
}

/// Whole notches the wheel turned over `item` this frame, up positive;
/// a smooth wheel's pixels add up until they make one.
fn wheel(tray: &mut Tray, ui: &mut Ui, item: Item) -> i32 {
    let dy = ui.state.wheel.y;
    if dy == 0.0 || !matches!(item, Item::Audio | Item::Brightness | Item::Workspace) {
        return 0;
    }
    ui.state.wheel = Vec2::ZERO;
    let notch = ui.m.widget_h.max(1.0);
    let sum = tray.wheel.filter(|(i, _)| *i == item).map_or(0.0, |(_, s)| s) + dy;
    let notches = (sum / notch).trunc();
    tray.wheel = Some((item, sum - notches * notch));
    notches as i32
}

/// Ask the compositor for the workspace `steps` on from ours.
fn cycle(link: &compositor::Link, ws: Option<&Workspaces>, steps: i32) {
    let Some(ws) = ws else { return };
    for _ in 0..steps.unsigned_abs() {
        link.send(&format!("cycle:{}:{}", ws.output, steps.signum()));
    }
}

fn pct(level: f64) -> u32 {
    (level * 100.0).round() as u32
}

/// How wide `item`'s face is.
fn width(ui: &mut Ui, item: Item, seen: &Seen, options: TrayOptions, g: f64, percent: &TextStyle) -> f64 {
    let m = ui.m;
    let Some(s) = seen.battery.as_ref().filter(|_| item == Item::Battery) else { return m.px(CELL).max(g + m.px(16.0)).round() };
    let mut w = m.px(PAD) * 2.0 + body_size(g).x;
    if s.on_power() {
        w += g * 0.7 + m.px(2.0);
    }
    if options.battery_percent {
        w += m.px(SPACING) + ui.measure(&format!("{}%", s.percent), percent);
    }
    w.ceil()
}

/// The battery's body, nub included.
fn body_size(g: f64) -> Vec2 {
    Vec2::new((g * 1.08).round(), (g * 0.56).round())
}

/// The battery's colour at `percent`: red, amber, then green; green
/// while it charges.
pub fn charge_color(s: &BatteryState) -> Color {
    match s.percent {
        _ if s.charge == Charge::Charging => look::GOOD,
        p if p < CRITICAL => look::BAD,
        p if p < LOW => look::WARN,
        _ => look::GOOD,
    }
}

/// The battery's face: a bolt while plugged in, the body filled to its
/// charge, and the percent. Returns its words.
fn battery(tray: &mut Tray, ui: &mut Ui, face: Rect, g: f64, s: &BatteryState, show_percent: bool, style: &TextStyle) -> String {
    let m = ui.m;
    let ink = ui.theme.text;
    let cy = face.center().y;
    let mut x = face.min.x + m.px(PAD);
    if s.on_power() {
        let bolt = Rect::from_min_size(Vec2::new(x, (cy - g * 0.35).round()), Vec2::splat((g * 0.7).round()));
        tray.glyphs.draw(ui, Glyph::Bolt, bolt, ui.theme.accent);
        x = bolt.max.x + m.px(2.0);
    }
    let size = body_size(g);
    let nub_w = (g * 0.1).round().max(2.0);
    let body = Rect::from_min_size(Vec2::new(x.round(), (cy - size.y * 0.5).round()), Vec2::new(size.x - nub_w, size.y));
    let line = m.px(2.0).round().max(1.0);
    ui.draw.stroke_rect(body, line, m.px(4.0), ink);
    let nub = Rect::new(Vec2::new(body.max.x, (cy - size.y * 0.22).round()), Vec2::new(body.max.x + nub_w, (cy + size.y * 0.22).round()));
    ui.draw.rounded_rect(nub, m.px(1.5), ink);
    let inner = body.shrink(line + m.px(1.5));
    let fill = (inner.width() * f64::from(s.percent) / 100.0).max(if s.percent > 0 { m.px(2.0) } else { 0.0 });
    if fill > 0.0 {
        ui.draw.rounded_rect(Rect::new(inner.min, Vec2::new(inner.min.x + fill, inner.max.y)), m.px(2.0), charge_color(s));
    }
    if show_percent {
        let text = Rect::new(Vec2::new(nub.max.x + m.px(SPACING), face.min.y), Vec2::new(face.max.x, face.max.y));
        let low = s.percent < CRITICAL && !s.on_power();
        ui.text_in_rect(&format!("{}%", s.percent), style, text, if low { look::BAD } else { ink });
    }
    let left = s.seconds_left.map(|secs| format!(", {}", super::popup::duration(secs, s.charge))).unwrap_or_default();
    format!("Battery {}%: {}{left}", s.percent, s.charge.label().to_lowercase())
}

fn wifi_glyph(s: &WifiState) -> Glyph {
    match (s.powered && s.present, s.link, s.wired) {
        (_, _, true) if s.link != Link::Connected => Glyph::Wired,
        (true, Link::Connected, _) => Glyph::Wifi(s.bars() as u8),
        (true, _, _) => Glyph::Wifi(0),
        (false, _, _) => Glyph::WifiOff,
    }
}

fn wifi_tip(s: &WifiState) -> String {
    match (&s.ssid, s.link) {
        (Some(ssid), Link::Connected) => format!("WiFi: {ssid}, {}% signal", s.strength),
        (Some(ssid), Link::Connecting) => format!("WiFi: joining {ssid}…"),
        _ if s.wired => "Wired network".to_owned(),
        _ if !s.present || !s.powered => "WiFi is off".to_owned(),
        _ => "WiFi: not connected".to_owned(),
    }
}

fn bluetooth_tip(s: &BluetoothState) -> String {
    match (s.powered, s.connected()) {
        (false, _) => "Bluetooth is off".to_owned(),
        (true, 0) => "Bluetooth: nothing connected".to_owned(),
        (true, 1) => format!("Bluetooth: {}", s.devices.iter().find(|d| d.connected).map_or("", |d| d.name.as_str())),
        (true, n) => format!("Bluetooth: {n} devices connected"),
    }
}

/// The speaker, with as many waves as the volume is loud.
pub fn speaker_glyph(s: &AudioState) -> Glyph {
    match s.volume {
        _ if s.muted => Glyph::SpeakerMuted,
        v if v <= 0.005 => Glyph::Speaker(0),
        v if v < 0.34 => Glyph::Speaker(1),
        v if v < 0.67 => Glyph::Speaker(2),
        _ => Glyph::Speaker(3),
    }
}

/// The workspace's number in its box, its edge in the accent under the
/// pointer.
fn workspace(ui: &mut Ui, face: Rect, n: u32, hovered: bool, accent: Color) {
    let m = ui.m;
    let side = m.px(BOX).min(face.height() - m.px(8.0)).round();
    let rect = Rect::from_center_size(face.center(), Vec2::splat(side)).round();
    let ink = ui.theme.text;
    ui.draw.rounded_rect(rect, m.px(9.0), Color::WHITE.fade(0.06));
    ui.draw.stroke_rect(rect, m.px(1.5), m.px(9.0), if hovered { accent } else { ink.fade(0.3) });
    ui.text_centered(&n.to_string(), &TextStyle::new(m.px(DIGIT) as f32).bold(), rect, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wifi_picture_says_what_the_link_is() {
        let on = WifiState { present: true, powered: true, link: Link::Connected, strength: 80, ..WifiState::default() };
        assert_eq!(wifi_glyph(&on), Glyph::Wifi(4));
        assert_eq!(wifi_glyph(&WifiState { link: Link::Disconnected, ..on.clone() }), Glyph::Wifi(0));
        assert_eq!(wifi_glyph(&WifiState { powered: false, ..on.clone() }), Glyph::WifiOff);
        // A cable and no WiFi: the cable shows; WiFi in use wins over it.
        assert_eq!(wifi_glyph(&WifiState { link: Link::Disconnected, wired: true, ..on.clone() }), Glyph::Wired);
        assert_eq!(wifi_glyph(&WifiState { wired: true, ..on }), Glyph::Wifi(4));
    }

    #[test]
    fn the_speaker_has_as_many_waves_as_its_volume() {
        let at = |volume, muted| speaker_glyph(&AudioState { present: true, volume, muted, sinks: Vec::new() });
        assert_eq!((at(0.0, false), at(0.2, false), at(0.5, false), at(1.2, false)), (Glyph::Speaker(0), Glyph::Speaker(1), Glyph::Speaker(2), Glyph::Speaker(3)));
        assert_eq!(at(0.8, true), Glyph::SpeakerMuted);
    }

    #[test]
    fn the_battery_reddens_as_it_runs_down_but_not_while_charging() {
        let at = |percent, charge| charge_color(&BatteryState { present: true, percent, charge, ..BatteryState::default() });
        assert_eq!((at(10, Charge::Discharging), at(40, Charge::Discharging), at(80, Charge::Discharging)), (look::BAD, look::WARN, look::GOOD));
        assert_eq!(at(10, Charge::Charging), look::GOOD);
    }
}

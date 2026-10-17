//! The Bluetooth popup on the test harness, over a made-up BlueZ: codes
//! checked and typed, a device found and paired, one forgotten, files
//! offered and sent.

use lntrn_kit::desktop::Desktop;
use lntrn_status::bluetooth::{Asked, BluetoothState, Device, Ended, Offer, Question, Reply, Transfer};
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, Ui, WidgetId};

use super::super::Frame;
use super::{Ask, Entry, panel};
use crate::bar::{self, Look};
use crate::tray::glyphs::Glyphs;

const BUDS: &str = "/org/bluez/hci0/dev_AA_AA_AA_AA_AA_AA";
const PHONE: &str = "/org/bluez/hci0/dev_BB_BB_BB_BB_BB_BB";
const MOUSE: &str = "/org/bluez/hci0/dev_CC_CC_CC_CC_CC_CC";

fn device(path: &str, name: &str, icon: &str, connected: bool, takes_files: bool) -> Device {
    let address = path.rsplit_once("dev_").unwrap().1.replace('_', ":");
    Device { path: path.into(), address, name: name.into(), icon: icon.into(), connected, battery: None, takes_files, rssi: Some(-50) }
}

/// The buds paired and connected, the phone paired; the mouse nearby.
fn bluez() -> BluetoothState {
    BluetoothState {
        present: true,
        powered: true,
        name: "lantern".into(),
        devices: vec![device(BUDS, "Buds", "audio-headset", true, false), device(PHONE, "Pixel 9", "phone", false, true)],
        nearby: vec![device(MOUSE, "MX Master", "input-mouse", false, false)],
        scanning: true,
        ..BluetoothState::default()
    }
}

#[derive(Default)]
struct Popup {
    entry: Entry,
    glyphs: Glyphs,
    scroll: f64,
    asked: Vec<Ask>,
}

impl Popup {
    fn frame(&mut self, ui: &mut Ui, s: &BluetoothState) {
        let frame = Frame { x: 450.0, bottom: 1580.0, look: Look::of(&Desktop::default()) };
        if let (_, Some(ask)) = panel(ui, s, &mut self.entry, &mut self.glyphs, &mut self.scroll, &frame) {
            self.asked.push(ask);
        }
    }
}

fn harness() -> Harness {
    let mut h = Harness::new(900.0, 1600.0);
    h.theme = bar::theme(&Desktop::default());
    h
}

fn id(path: &[&str]) -> WidgetId {
    path.iter().fold(WidgetId::ROOT, |id, part| id.with(part))
}

fn asking(device: &str, kind: Asked) -> Question {
    Question { id: 7, device: device.into(), name: "MX Master".into(), kind }
}

#[test]
fn adding_looks_for_devices_and_a_click_pairs_one() {
    let (mut h, mut p) = (harness(), Popup::default());
    let s = bluez();
    h.settle(3, |ui| p.frame(ui, &s));
    assert!(h.rect_of(id(&["nearby", MOUSE])).is_none(), "not looking yet");
    h.click_on(id(&["add"]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::Scan(true)));
    h.click_on(id(&["nearby", MOUSE]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::Pair(MOUSE.into())));
    h.click_on(id(&["add"]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::Scan(false)));
}

#[test]
fn a_code_both_show_is_checked_with_a_click() {
    let (mut h, mut p) = (harness(), Popup::default());
    let mut s = bluez();
    p.entry.adding = true;
    s.pairing = Some(MOUSE.into());
    s.question = Some(asking(MOUSE, Asked::Confirm(482_913)));
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["question", "yes"]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked, [Ask::Answer(7, Reply::Yes)]);
}

#[test]
fn a_pin_is_typed_under_the_device_and_a_passkey_takes_six_digits() {
    let (mut h, mut p) = (harness(), Popup::default());
    let mut s = bluez();
    p.entry.adding = true;
    s.question = Some(asking(MOUSE, Asked::Pin));
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(h.state.focus, Some(id(&["pin"]).with_u64(7)), "ready to type in");
    h.type_text("0000");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::Answer(7, Reply::Pin("0000".into()))));

    s.question = Some(Question { id: 8, ..asking(MOUSE, Asked::Passkey) });
    h.settle(3, |ui| p.frame(ui, &s));
    h.type_text("12a345");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert!(p.asked.is_empty(), "five digits: a letter isn't one");
    h.type_text("6");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::Answer(8, Reply::Passkey(123_456))));
}

#[test]
fn a_phone_pairing_from_its_end_asks_up_top() {
    let (mut h, mut p) = (harness(), Popup::default());
    let mut s = bluez();
    s.question = Some(Question { id: 9, device: "/org/bluez/hci0/dev_DD_DD_DD_DD_DD_DD".into(), name: "Galaxy".into(), kind: Asked::Confirm(1) });
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["question", "no"]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked, [Ask::Answer(9, Reply::No)]);
}

#[test]
fn a_paired_device_offers_its_buttons_under_the_pointer() {
    let (mut h, mut p) = (harness(), Popup::default());
    let s = bluez();
    h.settle(3, |ui| p.frame(ui, &s));
    assert!(h.rect_of(id(&["send", PHONE])).is_none(), "only under the pointer");
    let row = h.rect_of(id(&["device", PHONE])).unwrap();
    h.move_to(row.center());
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["send", PHONE]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::SendFile("BB:BB:BB:BB:BB:BB".into(), "Pixel 9".into())));
    // Forget, made sure of with a second click.
    h.move_to(row.center());
    h.click_on(id(&["forget", PHONE]), |ui| p.frame(ui, &s));
    assert!(p.asked.is_empty());
    h.click_on(id(&["forget", PHONE]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::Forget(PHONE.into())));
    // The buds take no files.
    h.move_to(h.rect_of(id(&["device", BUDS])).unwrap().center());
    h.settle(3, |ui| p.frame(ui, &s));
    assert!(h.rect_of(id(&["send", BUDS])).is_none());
}

#[test]
fn a_file_on_its_way_stops_with_a_click_and_one_ended_has_no_stop() {
    let (mut h, mut p) = (harness(), Popup::default());
    let mut s = bluez();
    let sending = Transfer { path: "/org/bluez/obex/client/session1/transfer1".into(), device: "BB:BB:BB:BB:BB:BB".into(), name: "song.flac".into(), size: 1000, done: 400, incoming: false, end: None };
    s.transfers = vec![sending.clone(), Transfer { path: "/t/old".into(), end: Some((Ended::Sent, std::time::Instant::now())), ..sending.clone() }];
    h.settle(3, |ui| p.frame(ui, &s));
    assert!(h.rect_of(id(&["stop", "/t/old"])).is_none(), "it has ended");
    h.click_on(id(&["stop", &sending.path]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked, [Ask::Stop(sending.path)]);
}

#[test]
fn a_file_offered_is_taken_with_a_click() {
    let (mut h, mut p) = (harness(), Popup::default());
    let mut s = bluez();
    s.offer = Some(Offer { id: 3, device: "BB:BB:BB:BB:BB:BB".into(), from: "Pixel 9".into(), name: "photo.jpg".into(), size: 2_340_000 });
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["offer", "yes"]), |ui| p.frame(ui, &s));
    assert_eq!(p.asked, [Ask::TakeFile(3, true)]);
}

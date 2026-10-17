//! The WiFi popup on the test harness, over a made-up iwd: a password
//! typed under its network, a hidden network joined by name, a saved one
//! forgotten.

use lntrn_kit::desktop::Desktop;
use lntrn_math::Vec2;
use lntrn_status::wifi::{Link, Network, Security, Target, WifiState};
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, WidgetId};

use super::super::Frame;
use super::{Ask, Entry, Open, panel};
use crate::bar::{self, Look};
use crate::tray::glyphs::Glyphs;

const HOME: &str = "/net/connman/iwd/0/4/486f6d65_psk";
const CAFE: &str = "/net/connman/iwd/0/4/43616665_psk";
const OFFICE: &str = "/net/connman/iwd/0/4/4f6666696365_8021x";

/// Home in use and saved, the cafe's asking a password, the office's a
/// company login.
fn iwd() -> WifiState {
    let net = |path: &str, name: &str, security, known, connected| Network { path: path.into(), name: name.into(), security, known, connected, strength: 80 };
    WifiState {
        present: true,
        powered: true,
        link: Link::Connected,
        ssid: Some("Home".into()),
        strength: 80,
        networks: vec![net(HOME, "Home", Security::Psk, true, true), net(CAFE, "Cafe", Security::Psk, false, false), net(OFFICE, "Office", Security::Enterprise, false, false)],
        ..WifiState::default()
    }
}

/// The popup as the overlay holds it: what is typed, the pictures, the
/// list's scroll, and everything asked.
#[derive(Default)]
struct Popup {
    entry: Entry,
    glyphs: Glyphs,
    scroll: f64,
    asked: Vec<Ask>,
}

impl Popup {
    fn frame(&mut self, ui: &mut lntrn_ui::Ui, s: &WifiState) {
        let frame = Frame { x: 450.0, bottom: 1380.0, look: Look::of(&Desktop::default()) };
        if let (_, Some(ask)) = panel(ui, s, &mut self.entry, &mut self.glyphs, &mut self.scroll, &frame) {
            self.asked.push(ask);
        }
    }
}

fn harness() -> Harness {
    let mut h = Harness::new(900.0, 1400.0);
    h.theme = bar::theme(&Desktop::default());
    h
}

fn id(path: &[&str]) -> WidgetId {
    path.iter().fold(WidgetId::ROOT, |id, part| id.with(part))
}

#[test]
fn a_password_is_typed_under_its_network_and_tried_again_when_it_fails() {
    let (mut h, mut p) = (harness(), Popup::default());
    let mut s = iwd();
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["network", CAFE]), |ui| p.frame(ui, &s));
    assert_eq!(p.entry.open, Some(Open::Network(CAFE.into())));
    let field = id(&["password", CAFE]);
    assert_eq!(h.state.focus, Some(field), "the box opens ready to type in");

    // Too short to be one: Enter does nothing.
    h.type_text("hunter2");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert!(p.asked.is_empty());
    h.type_text("2");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::JoinWith(CAFE.into(), "hunter22".into())));

    // iwd turns it down: the box stays, the field focused again.
    s.joining = Some(Target::Network(CAFE.into()));
    h.settle(3, |ui| p.frame(ui, &s));
    (s.joining, s.failed, s.error) = (None, Some(Target::Network(CAFE.into())), Some("That password didn't work".into()));
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!((p.entry.open.clone(), h.state.focus), (Some(Open::Network(CAFE.into())), Some(field)));
    // The eye takes the click, and gives the typing its focus back.
    h.click_on(field.with("eye"), |ui| p.frame(ui, &s));
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(h.state.focus, Some(field));

    // Another try that works: the box goes.
    h.type_text("!");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(p.asked.pop(), Some(Ask::JoinWith(CAFE.into(), "hunter22!".into())));
    (s.joining, s.failed, s.error) = (Some(Target::Network(CAFE.into())), None, None);
    h.settle(3, |ui| p.frame(ui, &s));
    s.joining = None;
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(p.entry.open, None);
}

#[test]
fn a_hidden_network_is_joined_by_its_name() {
    let (mut h, mut p) = (harness(), Popup::default());
    let s = iwd();
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["hidden"]), |ui| p.frame(ui, &s));
    assert_eq!(h.state.focus, Some(id(&["hidden", "name"])));
    // Enter on the name goes on to the password; none is an open network.
    h.type_text(" Attic ");
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(h.state.focus, Some(id(&["hidden", "password"])));
    h.key(Key::Enter);
    h.settle(3, |ui| p.frame(ui, &s));
    assert_eq!(p.asked, [Ask::JoinHidden("Attic".into(), String::new())]);
}

#[test]
fn a_saved_network_is_forgotten_on_a_second_click() {
    let (mut h, mut p) = (harness(), Popup::default());
    let s = iwd();
    h.settle(3, |ui| p.frame(ui, &s));
    let forget = id(&["forget", HOME]);
    h.click_on(forget, |ui| p.frame(ui, &s));
    assert!(p.asked.is_empty(), "the first only asks if you're sure");
    h.click_on(forget, |ui| p.frame(ui, &s));
    assert_eq!(p.asked, [Ask::Forget(HOME.into())]);
}

#[test]
fn a_company_login_opens_nothing() {
    let (mut h, mut p) = (harness(), Popup::default());
    let s = iwd();
    h.settle(3, |ui| p.frame(ui, &s));
    h.click_on(id(&["network", OFFICE]), |ui| p.frame(ui, &s));
    assert_eq!((p.entry.open.clone(), p.asked.len()), (None, 0));
    let _ = Vec2::ZERO;
}

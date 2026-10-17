//! The monitors on Lantern UI's test harness, no window and no GPU: their
//! faces on a bar and each popup over them, as a hand would use them,
//! showing a machine fixed for the test.

use std::sync::Arc;

use lntrn_kit::desktop::Desktop;
use lntrn_math::{Rect, Vec2};
use lntrn_status::load::gpu::Gpu;
use lntrn_status::{LoadState, load::Top};
use lntrn_ui::testing::Harness;

use super::{Kind, Monitors, popup};
use crate::bar::Look;
use crate::tray::popup::Frame;

const GIB: u64 = 1 << 30;

/// A busy evening: a game running, a browser open.
fn machine() -> LoadState {
    let wave = |lo: f32, hi: f32| (0..60).map(|i| lo + (hi - lo) * ((i as f32 * 0.4).sin() * 0.5 + 0.5)).collect::<Vec<f32>>();
    let mut s = LoadState { seq: 60, cpu_name: "Intel Core i7-14700K".into(), cpu_celsius: Some(71.0), load: [2.1, 1.7, 1.2], uptime: 93_000.0, ..LoadState::default() };
    (s.cpu.usage, s.cpu.mhz, s.cpu.governor, s.cpu.history) = (34.0, 4210.0, "performance".into(), wave(10.0, 60.0));
    (s.mem.total, s.mem.available, s.mem.free, s.mem.swap_total, s.mem.swap_free, s.mem.history) = (32_547_948 * 1024, 20 * GIB, 9 * GIB, 8 * GIB, 7 * GIB, wave(35.0, 40.0));
    s.gpu = Some(Gpu { name: "NVIDIA GeForce RTX 3080 Ti".into(), driver: "nvidia 615.71.09".into(), usage: Some(88.0), memory: Some((7 * GIB, 12 * GIB)), celsius: Some(84.0), watts: Some(310.0), watts_limit: Some(350.0), fans: vec![62, 64], core_mhz: Some(1905), memory_mhz: Some(9501), usage_history: wave(40.0, 95.0), ..Gpu::default() });
    s.top = Some(Top { cpu: vec![(Arc::from("Borderlands4.exe"), 21.5), (Arc::from("firefox"), 4.2)], memory: vec![(Arc::from("Borderlands4.exe"), 9 * GIB), (Arc::from("firefox"), 3 * GIB / 2)] });
    s
}

/// One frame of `kind`'s popup: its panel, if it shows.
fn once(h: &mut Harness, mon: &Monitors, kind: Kind, frame: &Frame) -> Option<Rect> {
    let mut panel = None;
    h.frame(|ui| panel = popup::draw(mon, ui, kind, frame).panel);
    panel
}

fn monitors() -> Monitors {
    Monitors { fixed: Some(machine()), ..Monitors::default() }
}

/// A 1.3x bar 1600 logical wide, its plate the default 64 tall.
fn harness() -> Harness {
    let scale = 1.3;
    let mut h = Harness::new(1600.0 * scale, 900.0 * scale);
    h.scale = scale;
    h.theme = crate::bar::theme(&Desktop::default());
    h
}

fn plate(h: &Harness) -> Rect {
    let w = h.window();
    Rect::new(Vec2::new(w.min.x, w.max.y - 64.0 * h.scale), w.max)
}

#[test]
fn the_faces_stand_side_by_side_and_a_click_opens_one() {
    let mut h = harness();
    let mut mon = monitors();
    let (plate, under) = (plate(&h), Look::of(&Desktop::default()).plate);
    let mut room = Rect::default();
    h.settle(3, |ui| room = super::draw(&mut mon, ui, plate, plate.min.x + 80.0, under).rect);
    let faces: Vec<Rect> = Kind::ALL.iter().map(|&k| mon.face(k).expect("every monitor shows")).collect();
    for pair in faces.windows(2) {
        assert!(pair[0].max.x < pair[1].min.x, "{pair:?} overlap");
    }
    assert!(faces.iter().all(|f| f.width() >= 132.0 * 1.3 - 1.0 && plate.contains_rect(f)));
    assert_eq!((room.min.x, room.max.x), (plate.min.x + 80.0, faces[2].max.x));
    // Widths are the widest numbers', so a busier machine moves nothing.
    let mut busier = machine();
    busier.cpu.usage = 100.0;
    busier.cpu_celsius = Some(100.0);
    mon.fixed = Some(busier);
    h.settle(3, |ui| _ = super::draw(&mut mon, ui, plate, plate.min.x + 80.0, under));
    assert_eq!(mon.face(Kind::Cpu), Some(faces[0]));
    let mut press = None;
    h.click_at(faces[1].center(), |ui| press = press.or(super::draw(&mut mon, ui, plate, plate.min.x + 80.0, under).press));
    assert_eq!(press, Some(Kind::Gpu));
}

#[test]
fn a_short_bar_still_fits_its_faces() {
    let mut h = harness();
    let mut mon = monitors();
    let w = h.window();
    let short = Rect::new(Vec2::new(w.min.x, w.max.y - 40.0 * h.scale), w.max);
    h.settle(3, |ui| _ = super::draw(&mut mon, ui, short, short.min.x, Look::of(&Desktop::default()).plate));
    assert!(Kind::ALL.iter().all(|&k| mon.face(k).is_some_and(|f| short.contains_rect(&f))));
}

#[test]
fn each_popup_fits_above_the_bar_and_its_button_opens_the_system_monitor() {
    for kind in Kind::ALL {
        let mut h = harness();
        let mon = monitors();
        let bottom = plate(&h).min.y - 10.0;
        let frame = Frame { x: 300.0, bottom, look: Look::of(&Desktop::default()) };
        let mut panel = None;
        h.settle(3, |ui| panel = popup::draw(&mon, ui, kind, &frame).panel);
        let panel = panel.unwrap_or_else(|| panic!("{kind:?}'s popup shows"));
        assert!(panel.max.y <= bottom + 0.5 && panel.min.y >= 0.0 && h.window().contains_rect(&panel), "{kind:?}: {panel:?}");
        // The button is the panel's last part, along its bottom.
        let button = Vec2::new(panel.center().x, panel.max.y - 18.0 * h.scale - 27.0 * h.scale);
        let mut sysmon = false;
        h.click_at(button, |ui| sysmon |= popup::draw(&mon, ui, kind, &frame).sysmon);
        assert!(sysmon, "{kind:?}'s button");
    }
}

#[test]
fn a_click_off_the_popup_or_escape_puts_it_away() {
    let mut h = harness();
    let mon = monitors();
    let frame = Frame { x: 300.0, bottom: plate(&h).min.y - 10.0, look: Look::of(&Desktop::default()) };
    let mut shown = true;
    h.click_at(Vec2::new(1500.0, 40.0), |ui| shown &= popup::draw(&mon, ui, Kind::Ram, &frame).panel.is_some());
    assert!(!shown, "clicked off");
    h.key(lntrn_ui::Key::Escape);
    assert!(once(&mut h, &mon, Kind::Cpu, &frame).is_none(), "Escaped");
}

#[test]
fn an_asleep_card_says_so_and_a_machine_without_one_shows_no_gpu() {
    let mut h = harness();
    let mut mon = monitors();
    let mut s = machine();
    s.gpu = Some(Gpu { name: "NVIDIA GeForce RTX 4060 Laptop GPU".into(), asleep: true, usage_history: vec![0.0; 10], ..Gpu::default() });
    mon.fixed = Some(s.clone());
    let frame = Frame { x: 300.0, bottom: plate(&h).min.y - 10.0, look: Look::of(&Desktop::default()) };
    assert!(once(&mut h, &mon, Kind::Gpu, &frame).is_some());
    s.gpu = None;
    mon.fixed = Some(s);
    assert!(once(&mut h, &mon, Kind::Gpu, &frame).is_none(), "no card, no popup");
    let p = plate(&h);
    h.settle(3, |ui| _ = super::draw(&mut mon, ui, p, p.min.x, Look::of(&Desktop::default()).plate));
    assert_eq!((mon.face(Kind::Gpu), mon.face(Kind::Cpu).is_some()), (None, true));
}

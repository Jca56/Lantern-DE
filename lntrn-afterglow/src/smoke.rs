//! Headless runs on Lantern UI's test harness, no window, no GPU and no
//! helper: the whole app on every page at several window sizes and
//! scales, over a machine that isn't this one, and then used the way a
//! hand would: a slider dragged and applied, a trial kept, a profile
//! saved and applied, a fan curve's point dragged.

use std::sync::{Arc, Once};

use lntrn_afterglowd::PROTOCOL;
use lntrn_afterglowd::hw::cpu::{self, ThreadReading};
use lntrn_afterglowd::hw::nvml::{self, reason};
use lntrn_afterglowd::tune::{Fan, Tune};
use lntrn_afterglowd::wire::{Hello, Status};
use lntrn_kit::{look, probe};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::testing::Harness;
use lntrn_ui::{Action, Host, HostCx, Shell, WidgetId};

use crate::app::{App, Editor};
use crate::nav::Page;
use crate::pages::fans::{PRESETS, preset};
use crate::profiles::Profiles;
use crate::sample::{Frame, Gpu, HISTORY, Helper, Processor};
use crate::worker::Link;

/// Machines for the tests to show.
pub mod fixture {
    use super::*;
    use lntrn_afterglowd::tune::{CpuLimits, CpuTune, GpuLimits, GpuTune, Limits};

    /// The machine Afterglow was built on.
    pub fn limits() -> Limits {
        let stock = Tune {
            gpu: Some(GpuTune { core_offset: 0, memory_offset: 0, power_limit: 400, lock: None, fan: Fan::Auto }),
            cpu: CpuTune { governor: "powersave".into(), epp: "balance_performance".into(), turbo: true, max_percent: 100, pl1: 253, pl2: 253 },
        };
        Limits {
            gpu: Some(GpuLimits { name: "NVIDIA GeForce RTX 3080 Ti".into(), driver: "615.78.08".into(), core_offset: (-1000, 1000), memory_offset: (-2000, 6000), power: (100, 450), power_default: 400, lock: Some((210, 2100)), fans: 2, fan_range: Some((30, 100)), slowdown: 95 }),
            cpu: CpuLimits {
                model: "Intel(R) Core(TM) i7-14700K".into(),
                governors: vec!["performance".into(), "powersave".into()],
                epps: ["default", "performance", "balance_performance", "balance_power", "power"].map(String::from).to_vec(),
                turbo: true,
                pcore_mhz: 5500,
                ecore_mhz: 4300,
                power: Some((35, 253)),
            },
            stock,
        }
    }

    fn wave(base: f32, swing: f32) -> Vec<f32> {
        (0..HISTORY).map(|i| base + swing * ((i as f32) * 0.2).sin()).collect()
    }

    /// A busy machine, its helper answering, running `applied`.
    pub fn running(applied: Tune, trial: Option<u64>) -> Frame {
        let l = limits();
        let now = nvml::Reading { celsius: Some(66), watts: Some(318.0), watts_limit: Some(400.0), core_mhz: Some(1905), memory_mhz: Some(9501), usage: Some(97), memory: Some((3 << 30, 12 << 30)), fans: vec![55, 55], reasons: Some(reason::POWER_CAP), pstate: Some(0) };
        let gpu = Gpu { name: l.gpu.as_ref().unwrap().name.clone(), now, clock: wave(1850.0, 60.0), celsius: wave(64.0, 3.0), watts: wave(300.0, 25.0), fan: wave(52.0, 4.0) };
        let threads = (0..28).map(|id| ThreadReading { id, mhz: if id < 16 { 5300 } else { 4200 }, usage: 37.0, pcore: id < 16 }).collect();
        let cpu = Processor { now: cpu::Reading { usage: 37.0, celsius: Some(64.0), threads }, usage: wave(37.0, 10.0), celsius: wave(62.0, 4.0), watts: wave(120.0, 20.0) };
        let hello = Arc::new(Hello { protocol: PROTOCOL, root: true, dry_run: false, limits: l });
        let status = Status { applied, trial, reverted: 0, package_watts: Some(121.0), fan_percent: None, trouble: None };
        Frame { gpu: Some(gpu), cpu, helper: Helper::Here { hello, status } }
    }

    pub fn frame() -> Frame {
        running(limits().stock, None)
    }

    /// The same machine with its helper not running.
    pub fn away() -> Frame {
        Frame { helper: Helper::Away("the helper isn't running".into()), ..frame() }
    }

    /// A tune on trial, twelve seconds left.
    pub fn trial() -> Frame {
        let mut t = limits().stock;
        t.gpu.as_mut().unwrap().core_offset = 150;
        running(t, Some(12))
    }
}

/// Point the desktop's folders at a scratch tree, so nothing here reads
/// or writes the real profiles or `lantern.toml`.
fn sandbox() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let root = std::env::temp_dir().join(format!("lntrn-afterglow-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // SAFETY: set once, before anything here reads it.
        unsafe { std::env::set_var("LANTERN_HOME", &root) };
    });
}

struct Rig {
    h: Harness,
    shell: Shell<App>,
    app: App,
}

impl Rig {
    fn at(frame: Frame, width: f64, height: f64, scale: f64) -> Rig {
        sandbox();
        probe::watch();
        let mut h = Harness::new(width * scale, height * scale);
        h.scale = scale;
        let mut shell = Shell::new(Editor::Main);
        shell.prefs.theme = look::theme(look::GOLD);
        let mut rig = Rig { h, shell, app: App::new(Link::fixed(frame), Profiles::default()) };
        rig.settle();
        rig
    }

    fn open(frame: Frame, page: Page) -> Rig {
        let mut rig = Rig::at(frame, 1500.0, 1050.0, 1.0);
        rig.app.page = page;
        rig.settle();
        rig
    }

    fn settle(&mut self) -> bool {
        !self.h.shell_settle(&mut self.shell, &mut self.app, 8).rebuild_again
    }

    /// The rect of something `path` deep in the window's one area.
    fn find(&self, path: impl Fn(WidgetId) -> WidgetId) -> Option<Rect> {
        (0..8).find_map(|area| self.h.rect_of(path(WidgetId::ROOT.with_u64(area).with("body"))))
    }

    fn rect(&self, what: &str, path: impl Fn(WidgetId) -> WidgetId) -> Rect {
        self.find(path).unwrap_or_else(|| panic!("{what} is not on screen"))
    }

    /// Scroll the page until what `path` names is wholly on it (the page
    /// ends above the bar), as a hand on the wheel would.
    fn reveal(&mut self, path: impl Fn(WidgetId) -> WidgetId) {
        for _ in 0..40 {
            let Some(r) = self.find(&path) else { return };
            let window = self.h.window();
            let bottom = window.max.y - (crate::bar::HEIGHT + 40.0) * self.h.scale;
            if r.max.y <= bottom && r.min.y >= window.min.y {
                return;
            }
            self.h.move_to(Vec2::new(window.center().x, window.min.y + 300.0));
            self.h.wheel(if r.max.y > bottom { -2.0 } else { 2.0 });
            self.settle();
        }
        panic!("it never scrolled into view");
    }

    fn click(&mut self, what: &str, path: impl Fn(WidgetId) -> WidgetId) {
        let at = self.rect(what, path).center();
        self.h.move_to(at);
        self.h.press();
        self.h.shell_frame(&mut self.shell, &mut self.app);
        self.h.release();
        self.settle();
    }

    fn drag(&mut self, from: Vec2, to: Vec2) {
        self.h.move_to(from);
        self.h.press();
        self.h.shell_frame(&mut self.shell, &mut self.app);
        for i in 1..=6 {
            self.h.move_to(from + (to - from) * (f64::from(i) / 6.0));
            self.h.shell_frame(&mut self.shell, &mut self.app);
        }
        self.h.release();
        self.settle();
    }

    fn run(&mut self, id: &str) {
        let mut requests = Vec::new();
        self.app.run(&Action::new(id), &mut HostCx { pointer: Vec2::ZERO, requests: &mut requests });
        self.settle();
    }

    fn sent(&self) -> Vec<String> {
        self.app.link_sent()
    }
}

/// Inside the page's column: its page id, its scroll area, and the
/// column it centres its content in.
fn page(body: WidgetId, page: Page) -> WidgetId {
    body.with_index(1).with(page.id()).with("page").with_index(1)
}

fn bar(body: WidgetId, action: &str) -> WidgetId {
    body.with_index(1).with("bar").with(action)
}

#[test]
fn every_page_settles_at_every_size_whatever_the_helper_says() {
    for (w, h, scale, roomy) in [(1100.0, 750.0, 1.0, true), (1500.0, 1050.0, 1.0, true), (1920.0, 1160.0, 1.0, true), (1152.0, 750.0, 1.4, true), (760.0, 560.0, 1.0, false)] {
        for (which, frame) in [("running", fixture::frame()), ("away", fixture::away()), ("on trial", fixture::trial())] {
            let mut rig = Rig::at(frame, w, h, scale);
            for p in Page::ALL {
                rig.app.page = p;
                probe::take_clipped();
                probe::take_stacked();
                assert!(rig.settle(), "{p:?} {which} at {w}x{h} @{scale} keeps asking for rebuilds");
                let cut = probe::take_clipped();
                assert!(!roomy || cut.is_empty(), "{p:?} {which} at {w}x{h} @{scale}: labels cut short: {cut:?}");
            }
        }
    }
}

#[test]
fn the_sidebar_and_the_tiles_lead_to_their_pages() {
    let mut rig = Rig::open(fixture::frame(), Page::Overview);
    rig.click("the Fans row", |body| body.with_index(0).with("sidebar").with("fans"));
    assert_eq!(rig.app.page, Page::Fans);
    rig.click("the Overview row", |body| body.with_index(0).with("sidebar").with("overview"));
    rig.click("the core clock tile", |body| page(body, Page::Overview).with("tiles").with("Core clock"));
    assert_eq!(rig.app.page, Page::Graphics);
}

#[test]
fn a_slider_dragged_is_a_change_to_apply() {
    let mut rig = Rig::open(fixture::frame(), Page::Graphics);
    assert!(rig.find(|b| bar(b, "tune.apply")).is_none(), "nothing to apply yet");
    let slider = rig.rect("the core offset slider", |b| page(b, Page::Graphics).with("clocks").with("Core offset"));
    rig.drag(Vec2::new(slider.min.x + slider.width() * 0.45, slider.center().y), Vec2::new(slider.min.x + slider.width() * 0.75, slider.center().y));
    let offset = rig.app.draft.as_ref().unwrap().gpu.as_ref().unwrap().core_offset;
    assert!(offset > 0 && offset % 15 == 0, "dragged right, in bins: {offset}");
    rig.click("the Apply button", |b| bar(b, "tune.apply"));
    assert_eq!(rig.sent(), ["apply"]);
    rig.click("the Discard button", |b| bar(b, "tune.discard"));
    assert_eq!(rig.app.draft.as_ref().unwrap().gpu.as_ref().unwrap().core_offset, 0);
}

#[test]
fn a_trial_is_kept_or_sent_back_from_the_bar() {
    let mut rig = Rig::open(fixture::trial(), Page::Overview);
    assert!(rig.find(|b| bar(b, "tune.apply")).is_none());
    rig.click("Keep", |b| bar(b, "tune.keep"));
    rig.click("Go Back", |b| bar(b, "tune.revert"));
    assert_eq!(rig.sent(), ["keep", "revert"]);
}

#[test]
fn without_the_helper_there_is_nothing_to_tune_and_a_way_to_ask_again() {
    let mut rig = Rig::open(fixture::away(), Page::Graphics);
    assert!(rig.find(|b| page(b, Page::Graphics).with("clocks").with("Core offset")).is_none());
    assert!(rig.app.draft.is_none());
    rig.click("Try Again", |b| bar(b, "helper.retry"));
    assert_eq!(rig.sent(), ["now"]);
}

#[test]
fn a_profile_is_saved_applied_renamed_and_deleted() {
    let mut rig = Rig::open(fixture::frame(), Page::Profiles);
    rig.app.profiles = Profiles::load();
    rig.app.draft.as_mut().unwrap().gpu.as_mut().unwrap().power_limit = 450;
    rig.run("profile.new");
    rig.app.asking.name = "  Gaming ".into();
    assert!(rig.app.dialog_ready(&Action::new("profile.new-go")));
    rig.run("profile.new-go");
    assert_eq!(rig.app.profiles.list.len(), 1);
    assert_eq!(rig.app.profiles.list[0].name, "Gaming");
    assert_eq!(Profiles::load().list, rig.app.profiles.list, "kept on disk");
    // The same name again, whatever its case, is refused.
    rig.app.asking.name = "gaming".into();
    assert!(!rig.app.dialog_ready(&Action::new("profile.new-go")));

    rig.run("tune.discard");
    rig.click("its Apply button", |b| page(b, Page::Profiles).with("profiles").with_index(0).with("Gaming").with("apply"));
    assert_eq!(rig.sent(), ["apply"]);
    assert_eq!(rig.app.draft.as_ref().unwrap().gpu.as_ref().unwrap().power_limit, 450);

    rig.run("profile.rename.0");
    rig.app.asking.name = "Loud".into();
    rig.run("profile.rename-go");
    rig.run("profile.delete.0");
    assert_eq!(Profiles::load().list[0].name, "Loud");
    rig.run("profile.delete-go");
    assert!(Profiles::load().list.is_empty());
}

#[test]
fn a_curve_point_dragged_up_speeds_that_heat_up() {
    let mut rig = Rig::open(fixture::frame(), Page::Fans);
    let curve = preset(&PRESETS[1].1, (30, 100));
    rig.app.draft.as_mut().unwrap().gpu.as_mut().unwrap().fan = Fan::Curve(curve.clone());
    rig.settle();
    let points = |rig: &Rig| match &rig.app.draft.as_ref().unwrap().gpu.as_ref().unwrap().fan {
        Fan::Curve(p) => p.clone(),
        other => panic!("not a curve: {other:?}"),
    };
    let dot = rig.rect("the curve's second point", |b| page(b, Page::Fans).with("mode").with("curve").with_index(1));
    rig.drag(dot.center(), dot.center() - Vec2::new(0.0, 60.0));
    let moved = points(&rig);
    assert!(moved[1].percent > curve[1].percent, "{:?} → {:?}", curve[1], moved[1]);
    assert_eq!(rig.app.fans.picked, Some(1));
    // Picked, it can go: the curve keeps the rest.
    let remove = |b| page(b, Page::Fans).with("mode").with("Remove the picked point");
    rig.reveal(remove);
    rig.click("Remove", remove);
    assert_eq!(points(&rig).len(), curve.len() - 1);
    // Any other mode and back: the curve is remembered.
    rig.app.draft.as_mut().unwrap().gpu.as_mut().unwrap().fan = Fan::Auto;
    rig.settle();
    assert_eq!(rig.app.fans.curve.as_ref().map(Vec::len), Some(curve.len() - 1));
}

#[test]
fn a_trial_that_goes_back_leaves_what_was_tried_on_the_pages() {
    let mut rig = Rig::open(fixture::trial(), Page::Graphics);
    let tried = rig.app.draft.clone().unwrap();
    assert_eq!(tried.gpu.as_ref().unwrap().core_offset, 150);
    // The helper ran the trial out: stock again, one trial gone back.
    let mut back = fixture::frame();
    if let Helper::Here { status, .. } = &mut back.helper {
        status.reverted = 1;
    }
    rig.app.link_show(back);
    rig.settle();
    assert_eq!(rig.app.draft.as_ref(), Some(&tried), "still there to nudge and try again");
    assert!(rig.find(|b| bar(b, "tune.apply")).is_some(), "and it says so");
}

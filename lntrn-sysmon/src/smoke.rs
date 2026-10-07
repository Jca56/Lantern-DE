//! Headless runs on Lantern UI's test harness, no window and no GPU: the
//! whole app on every page at several window sizes and scales, over a
//! machine that isn't this one (so there is always a graphics card, a
//! battery and a browser to show), and the Processes page clicked, typed
//! at and keyed the way a hand would.

use std::process::Command;
use std::sync::{Arc, Once};

use lntrn_kit::chart::{self, Graph, Series};
use lntrn_kit::{look, probe};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::testing::Harness;
use lntrn_ui::{Action, Host, HostCx, Key, Modifiers, Shell, WidgetId};

use crate::app::{App, Editor};
use crate::ffi::{SIGKILL, SIGTERM};
use crate::nav::Page;
use crate::rows::{Column, Pick, Row, Sort};
use crate::sample::procs::{self, Proc};
use crate::sample::{Frame, fixture};
use crate::settings::Settings;
use crate::worker::Link;

/// Point the desktop's folders at a scratch tree, so nothing here reads
/// or writes the real `lantern.toml`.
fn sandbox() -> std::path::PathBuf {
    static ONCE: Once = Once::new();
    let root = std::env::temp_dir().join(format!("lntrn-sysmon-smoke-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // SAFETY: set once, before anything here reads it.
        unsafe { std::env::set_var("LANTERN_HOME", &root) };
    });
    root
}

struct Rig {
    h: Harness,
    shell: Shell<App>,
    app: App,
}

impl Rig {
    /// The app showing `frame` in a window `width` × `height` logical
    /// pixels at `scale`.
    fn at(frame: Frame, width: f64, height: f64, scale: f64) -> Rig {
        sandbox();
        probe::watch();
        let mut h = Harness::new(width * scale, height * scale);
        h.scale = scale;
        let mut shell = Shell::new(Editor::Monitor);
        shell.prefs.theme = look::theme(look::GOLD);
        let mut rig = Rig { h, shell, app: App::new(Link::fixed(frame), Settings::default()) };
        rig.settle();
        rig
    }

    fn open(page: Page) -> Rig {
        let mut rig = Rig::at(fixture::frame(), 1500.0, 1000.0, 1.0);
        rig.app.page = page;
        rig.settle();
        rig
    }

    /// Frames until the layout stops asking for another pass. `true`
    /// when it did stop.
    fn settle(&mut self) -> bool {
        !self.h.shell_settle(&mut self.shell, &mut self.app, 8).rebuild_again
    }

    /// The rect of something `path` deep in the window's one area.
    fn rect(&self, what: &str, path: impl Fn(WidgetId) -> WidgetId) -> Rect {
        (0..8).find_map(|area| self.h.rect_of(path(WidgetId::ROOT.with_u64(area).with("body")))).unwrap_or_else(|| panic!("{what} is not on screen"))
    }

    fn click_at(&mut self, at: Vec2) {
        self.h.move_to(at);
        self.h.press();
        self.h.shell_frame(&mut self.shell, &mut self.app);
        self.h.release();
        self.settle();
    }

    fn click(&mut self, what: &str, path: impl Fn(WidgetId) -> WidgetId) {
        let rect = self.rect(what, path);
        self.click_at(rect.center());
    }

    fn key(&mut self, key: Key, mods: Modifiers) {
        self.h.key_with(key, mods);
        self.settle();
    }

    /// Row `i` of the process list.
    fn row(&self, i: usize) -> Rect {
        self.rect("a row of the list", |body| page(body).with("list").with("row").with_index(i))
    }

    /// What the list shows, each row as words.
    fn listed(&self) -> Vec<String> {
        let procs = &self.app.frame.as_ref().unwrap().procs;
        self.app
            .procs
            .rows
            .iter()
            .map(|r| match r {
                Row::Group { app, count, .. } => format!("{app}×{count}"),
                Row::Proc { index, inside } => format!("{}{}", if *inside { "  " } else { "" }, procs[*index].pid),
            })
            .collect()
    }
}

/// The Processes page's own id: the second column of the window, the
/// page's name, and the one column it lays itself out in.
fn page(body: WidgetId) -> WidgetId {
    body.with_index(1).with("processes").with_index(0)
}

fn toolbar(body: WidgetId) -> WidgetId {
    page(body).with("toolbar")
}

#[test]
fn every_page_settles_at_every_size_on_a_full_machine_and_a_bare_one() {
    // The smallest the window gets, the size it opens at, a big one, a
    // scaled one, and one narrower than it is allowed to be (a tile on a
    // small screen): that last must still lay out, if not prettily.
    for (w, h, scale, roomy) in [(1100.0, 700.0, 1.0, true), (1500.0, 1000.0, 1.0, true), (1920.0, 1160.0, 1.0, true), (1152.0, 720.0, 1.4, true), (760.0, 560.0, 1.0, false)] {
        for (which, frame) in [("full", fixture::frame()), ("bare", fixture::bare())] {
            let mut rig = Rig::at(frame, w, h, scale);
            for page in Page::ALL {
                rig.app.page = page;
                probe::take_clipped();
                probe::take_stacked();
                assert!(rig.settle(), "{page:?} on the {which} machine at {w}x{h} @{scale} keeps asking for rebuilds");
                let (cut, under) = (probe::take_clipped(), probe::take_stacked());
                assert!(!roomy || cut.is_empty(), "{page:?} on the {which} machine at {w}x{h} @{scale}: labels cut short: {cut:?}");
                // At the size it opens at and up, every reading is beside its label.
                assert!(w < 1500.0 || under.is_empty(), "{page:?} on the {which} machine at {w}x{h} @{scale}: readings pushed under their labels: {under:?}");
            }
        }
    }
}

#[test]
fn nothing_is_drawn_from_a_machine_not_yet_looked_at() {
    sandbox();
    let mut h = Harness::new(1500.0, 1000.0);
    let mut shell = Shell::new(Editor::Monitor);
    // A sampler set to look once a minute: no frame yet, whatever page.
    let mut app = App::new(Link::fixed(Frame::default()), Settings::default());
    app.frame = None;
    for page in Page::ALL {
        app.page = page;
        assert!(!h.shell_settle(&mut shell, &mut app, 8).rebuild_again, "{page:?}");
    }
}

#[test]
fn the_sidebar_and_the_tiles_lead_to_their_pages() {
    let mut rig = Rig::open(Page::Overview);
    rig.click("the CPU row", |body| body.with_index(0).with("sidebar").with("cpu"));
    assert_eq!(rig.app.page, Page::Cpu);
    rig.click("the Overview row", |body| body.with_index(0).with("sidebar").with("overview"));
    assert_eq!(rig.app.page, Page::Overview);
    rig.click("the memory tile", |body| body.with_index(1).with("overview").with("page").with_index(1).with("tiles").with("memory"));
    assert_eq!(rig.app.page, Page::Memory);
}

#[test]
fn a_row_is_picked_opened_and_stepped_through() {
    let mut rig = Rig::open(Page::Processes);
    assert_eq!(rig.listed(), ["rustc×2", "firefox×4", "1201", "6000", "1", "900", "1300"]);
    // The header shows how it starts sorted, without having been clicked.
    assert_eq!(rig.app.procs.view.sort, Sort::default());

    let second = rig.row(1);
    rig.click_at(second.center());
    assert_eq!(rig.app.procs.picked, Some(Pick::App(Arc::from("firefox"))));

    // Its chevron opens it, and the pick stays on it.
    rig.click_at(Vec2::new(second.min.x + 12.0, second.center().y));
    assert_eq!(rig.listed()[1..6], ["firefox×4", "  4410", "  4411", "  4412", "  4413"]);
    assert_eq!(rig.app.procs.picked, Some(Pick::App(Arc::from("firefox"))));

    // The arrows walk the rows, in and out of the open program.
    rig.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(rig.app.procs.picked, Some(Pick::Pid(4410)));
    rig.key(Key::ArrowUp, Modifiers::NONE);
    rig.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(rig.app.procs.picked, Some(Pick::App(Arc::from("rustc"))));
    rig.key(Key::End, Modifiers::NONE);
    assert_eq!(rig.app.procs.picked, Some(Pick::Pid(1300)));

    // A header sorts by its column; again, the other way.
    rig.click("the Name header", |body| page(body).with("list").with("col").with_index(0));
    assert_eq!(rig.app.procs.view.sort, Sort { by: Column::Name, ascending: true });
    assert_eq!(rig.listed()[0], "firefox×4");
    rig.click("the Name header", |body| page(body).with("list").with("col").with_index(0));
    assert_eq!(rig.app.procs.view.sort, Sort { by: Column::Name, ascending: false });
    assert_eq!(rig.listed()[0], "1300");
    // A column of amounts starts with its biggest, then turns over.
    rig.click("the Memory header", |body| page(body).with("list").with("col").with_index(3));
    assert_eq!(rig.app.procs.view.sort, Sort { by: Column::Memory, ascending: false });
    assert_eq!(rig.listed()[0], "firefox×4");
    rig.click("the Memory header", |body| page(body).with("list").with("col").with_index(3));
    assert_eq!(rig.app.procs.view.sort, Sort { by: Column::Memory, ascending: true });
    assert_eq!(rig.listed()[0], "1");
    // Numbers count upwards first, like names.
    rig.click("the PID header", |body| page(body).with("list").with("col").with_index(4));
    assert_eq!(rig.app.procs.view.sort, Sort { by: Column::Pid, ascending: true });
}

#[test]
fn the_list_is_searched_and_unfolded_and_that_is_remembered() {
    let mut rig = Rig::open(Page::Cpu);
    // From anywhere: Ctrl+F goes to the list with the keyboard in the field.
    rig.key(Key::Char('f'), Modifiers::CTRL);
    assert_eq!(rig.app.page, Page::Processes);
    rig.h.type_text("Fire");
    rig.settle();
    assert_eq!(rig.app.procs.view.search, "Fire");
    assert_eq!(rig.listed(), ["firefox×4"]);
    rig.key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.app.procs.view.search, "");
    assert_eq!(rig.listed().len(), 7);

    rig.click("Processes, of the two ways to list", |body| toolbar(body).with("fold").with_index(1));
    assert!(!rig.app.procs.view.grouped);
    assert_eq!(rig.listed().len(), 11, "every process a row");
    let saved = std::fs::read_to_string(sandbox().join("config/lantern.toml")).unwrap();
    assert!(saved.contains("group_by_program = false"), "{saved}");
}

#[test]
fn ending_asks_first_and_then_ends_what_was_picked_and_nothing_else() {
    let mut child = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = child.id();
    let born = procs::started(pid).expect("the child is running");
    let mut frame = fixture::frame();
    frame.procs.push(Proc { cpu: 90.0, started: born, ..fixture::proc(pid, "sleep", "sleep", "alva", 0.0, 1 << 20) });
    let mut rig = Rig::at(frame, 1500.0, 1000.0, 1.0);
    rig.app.page = Page::Processes;
    rig.settle();
    assert_eq!(rig.listed()[0], pid.to_string(), "the busiest, so the first");

    // Nothing picked: the buttons are dim and do nothing.
    rig.click("End", |body| toolbar(body).with("end"));
    assert_eq!(rig.app.procs.doomed, None);

    let first = rig.row(0);
    rig.click_at(first.center());
    assert_eq!(rig.app.procs.picked, Some(Pick::Pid(pid)));
    // Force Kill asks, and Enter there cancels: the child lives.
    rig.click("Force Kill", |body| toolbar(body).with("kill"));
    assert_eq!(rig.app.procs.doomed.as_ref().map(|d| (d.signal, d.targets.clone())), Some((SIGKILL, vec![(pid, born)])));
    rig.key(Key::Enter, Modifiers::NONE);
    assert!(child.try_wait().unwrap().is_none(), "cancelled");

    // End asks, and Enter there ends it. Delete is End's key.
    rig.click_at(first.center());
    rig.key(Key::Delete, Modifiers::NONE);
    assert_eq!(rig.app.procs.doomed.as_ref().map(|d| d.signal), Some(SIGTERM));
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.app.procs.doomed, None, "carried out");
    assert!(!child.wait().unwrap().success(), "ended by the signal");
}

#[test]
fn a_right_click_picks_the_row_and_the_menu_changes_how_the_machine_is_watched() {
    let mut rig = Rig::open(Page::Processes);
    let third = rig.row(2);
    rig.h.move_to(third.center());
    rig.h.right_press();
    rig.settle();
    assert_eq!(rig.app.procs.picked, Some(Pick::Pid(1201)));

    // The menu's rows say how things stand, and its actions change them.
    let menu = rig.app.menu("monitor").unwrap();
    let speeds = &menu.items[0].sub;
    assert_eq!(speeds.iter().map(|s| (s.label.as_str(), s.checked)).collect::<Vec<_>>(), [("Fast", Some(false)), ("Normal", Some(true)), ("Slow", Some(false)), ("Very Slow", Some(false))]);
    let mut requests = Vec::new();
    let mut run = |app: &mut App, id: &str| app.run(&Action::new(id), &mut HostCx { pointer: Vec2::ZERO, requests: &mut requests });
    run(&mut rig.app, "monitor.pause");
    assert_eq!((rig.app.paused, rig.app.status()), (true, "Paused".to_owned()));
    run(&mut rig.app, "speed.2");
    assert_eq!((rig.app.settings.interval, rig.app.paused), (2.0, false), "picking a speed is asking to watch");
    run(&mut rig.app, "procs.kernel");
    run(&mut rig.app, "page.sensors");
    assert_eq!((rig.app.procs.view.kernel, rig.app.page), (true, Page::Sensors));
    run(&mut rig.app, "procs.copy-command");
    assert_eq!(rig.app.copy.as_deref(), Some("/usr/bin/lntrn-compositor --pid-1201"));
    // Every command the palette offers is one the app knows.
    for (id, _) in rig.app.palette("") {
        assert!(id.starts_with("shell.") || id.starts_with("page.") || id.starts_with("speed.") || ["monitor.pause", "monitor.refresh", "procs.find", "procs.group", "procs.kernel"].contains(&id.as_str()), "{id}");
    }
}

#[test]
fn a_graph_says_which_moment_the_pointer_is_on() {
    let mut h = Harness::new(600.0, 400.0);
    h.theme = look::theme(look::GOLD);
    let values: Vec<f32> = (0..5).map(|i| i as f32 * 10.0).collect();
    let rect = Rect::from_xywh(50.0, 50.0, 420.0, 200.0);
    let mut found = None;
    let mut look_at = |h: &mut Harness, x: f64| {
        h.move_to(Vec2::new(x, 150.0));
        h.frame(|ui| {
            let id = ui.id("graph");
            found = chart::graph(ui, id, rect, &Graph { series: &[Series::area(&values, look::GOLD)], max: 100.0, slots: 11, top: "100%" });
        });
        found
    };
    // Eleven slots across the 400 inside its frame: 40 apart, the newest
    // on the right edge at 460.
    assert_eq!(look_at(&mut h, 460.0), Some(0));
    assert_eq!(look_at(&mut h, 385.0), Some(2));
    assert_eq!(look_at(&mut h, 300.0), Some(4));
    // Five samples: nothing was measured further back, nor off the graph.
    assert_eq!(look_at(&mut h, 200.0), None);
    assert_eq!(look_at(&mut h, 520.0), None);
}

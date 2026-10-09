//! The page in the whole app on Lantern UI's test harness, with a
//! stand-in for the compositor: laid out at the sizes the window comes
//! in, its tiles clicked and dragged, a setup applied, kept and left to
//! run out.

use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Shell, WidgetId};

use super::fake::{Fake, two_heads};
use crate::app::App;
use crate::nav::Page;
use crate::outputs::{Head, Outcome};
use crate::smoke::{app_at, click, clipped, in_body, stacked};

/// The app on the Monitors page with a stand-in compositor holding
/// `heads`, settled.
fn page_at(width: f64, height: f64, scale: f64, heads: Vec<Head>) -> (Harness, Shell<App>, App, Fake) {
    let (mut h, mut shell, mut app) = app_at(width, height, scale);
    let fake = Fake::with(heads);
    app.monitors.source = Some(Box::new(fake.clone()));
    app.page = Page::Monitors;
    let out = h.shell_settle(&mut shell, &mut app, 8);
    assert!(!out.rebuild_again, "the page keeps asking for rebuilds at {width}x{height} @{scale}");
    (h, shell, app, fake)
}

/// The id of something on the page, from the page's own column down.
fn on_page(h: &Harness, path: impl Fn(WidgetId) -> WidgetId) -> WidgetId {
    in_body(h, |body| path(body.with_index(1).with("monitors").with("page").with_index(1)))
}

fn tile(h: &Harness, i: usize) -> lntrn_math::Rect {
    h.rect_of(on_page(h, |p| p.with("layout").with("tiles").with("tile").with_index(i))).expect("the tile is laid out")
}

/// A button of the page by its id: in the layout card (`apply`, `reset`,
/// `identify`) or on the page itself (`keep`, `go-back`). `None` when it
/// isn't showing.
fn button(h: &Harness, name: &str) -> Option<lntrn_math::Rect> {
    let page = |area: u64| WidgetId::ROOT.with_u64(area).with("body").with_index(1).with("monitors").with("page").with_index(1);
    (0..8).flat_map(|area| [page(area).with(name), page(area).with("layout").with(name)]).find_map(|id| h.rect_of(id))
}

#[test]
fn the_page_lays_out_with_one_monitor_and_with_two() {
    for (w, hgt, scale) in [(1100.0, 700.0, 1.0), (1152.0, 720.0, 1.0), (1500.0, 1000.0, 1.0), (1920.0, 1160.0, 1.0), (1152.0, 720.0, 1.4)] {
        for heads in [two_heads()[..1].to_vec(), two_heads()] {
            let n = heads.len();
            clipped();
            stacked();
            let (h, _shell, app, _fake) = page_at(w, hgt, scale, heads);
            let (cut, under) = (clipped(), stacked());
            assert!(cut.is_empty(), "{n} monitor(s) at {w}x{hgt} @{scale}: labels cut short: {cut:?}");
            assert!(w < 1152.0 || under.is_empty(), "{n} monitor(s) at {w}x{hgt} @{scale}: controls pushed under their labels: {under:?}");
            assert_eq!(app.monitors.draft.len(), n);
            // Every monitor has its tile, inside the window, and none
            // covers another.
            let tiles: Vec<_> = (0..n).map(|i| tile(&h, i)).collect();
            assert!(tiles.iter().all(|t| t.width() > 60.0 * scale && t.height() > 40.0 * scale && t.min.x >= 0.0 && t.max.x <= w * scale), "{tiles:?}");
            assert!(n < 2 || tiles[0].max.x <= tiles[1].min.x + 1.0, "side by side, as on the desk: {tiles:?}");
            // Nothing is on trial: Apply is there, in view without a
            // scroll however many rows the settings run to, and Keep is not.
            let apply = button(&h, "apply").expect("Apply is laid out");
            assert!(apply.max.y <= hgt * scale && apply.height() >= 56.0 * scale, "{apply:?} in a window {hgt} tall");
            assert!(button(&h, "identify").is_some() && button(&h, "keep").is_none());
        }
    }
}

#[test]
fn a_tile_is_picked_by_a_click_and_moved_by_a_drag() {
    let (mut h, mut shell, mut app, _fake) = page_at(1500.0, 1000.0, 1.0, two_heads());
    assert_eq!(app.monitors.selected, 0);
    let second = tile(&h, 1);
    click(&mut h, &mut shell, &mut app, second.center());
    assert_eq!(app.monitors.selected, 1);
    assert!(!app.monitors.dirty(), "a click moves nothing: {:?} vs {:?}", app.monitors.draft, app.monitors.base);

    // Take the second by its middle and carry it past the first's left edge.
    let (first, second) = (tile(&h, 0), tile(&h, 1));
    let (from, to) = (second.center(), Vec2::new(first.min.x - second.width() * 0.4, first.center().y - first.height() * 0.2));
    h.move_to(from);
    h.press();
    h.shell_frame(&mut shell, &mut app);
    for step in 1..=6 {
        h.move_to(from + (to - from) * (f64::from(step) / 6.0));
        h.shell_frame(&mut shell, &mut app);
    }
    h.release();
    let out = h.shell_settle(&mut shell, &mut app, 8);
    assert!(!out.rebuild_again);
    let d = &app.monitors.draft;
    assert_eq!((d[1].x, d[0].x), (0, 1920), "it sits on the left now, the corner at the origin");
    assert_eq!(d[0].y.min(d[1].y), 0);
    assert!(app.monitors.dirty());
    // The picture follows: the second's tile is left of the first's.
    assert!(tile(&h, 1).max.x <= tile(&h, 0).min.x + 1.0);

    // Reset puts it back.
    let reset = button(&h, "reset").unwrap();
    click(&mut h, &mut shell, &mut app, reset.center());
    assert!(!app.monitors.dirty());
    assert_eq!((app.monitors.draft[0].x, app.monitors.draft[1].x), (0, 2743));
}

#[test]
fn apply_puts_a_setup_on_trial_and_keep_writes_it() {
    let (mut h, mut shell, mut app, fake) = page_at(1500.0, 1000.0, 1.0, two_heads());
    app.monitors.draft[0].scale = 1.25;
    app.monitors.edited();
    h.shell_settle(&mut shell, &mut app, 8);
    let apply = button(&h, "apply").unwrap();
    click(&mut h, &mut shell, &mut app, apply.center());
    assert_eq!(fake.asked().len(), 1);
    assert!(button(&h, "keep").is_none(), "nothing to keep until the compositor has done it");

    fake.answer(Outcome::Succeeded);
    let out = h.shell_settle(&mut shell, &mut app, 8);
    assert!(out.wake_after.is_some(), "the trial is counted down without anyone touching the window");
    let keep = button(&h, "keep").expect("the setup is on trial");
    assert!(button(&h, "apply").is_none() && button(&h, "go-back").is_some());
    assert!(app.config.monitors.is_empty(), "on trial is not in the file");

    h.advance(5.0);
    click(&mut h, &mut shell, &mut app, keep.center());
    let kept = &app.config.monitors;
    assert_eq!(kept.iter().map(|m| (m.name.as_str(), m.x, m.scale)).collect::<Vec<_>>(), [("DP-1", 0, Some(1.25)), ("HDMI-A-1", 3072, Some(1.0))]);
    assert_eq!(kept[0].resolution, "3840x2160");
    assert!(button(&h, "apply").is_some() && button(&h, "keep").is_none());
    assert_eq!(fake.asked().len(), 1);
}

#[test]
fn a_trial_runs_out_whatever_page_is_showing() {
    let (mut h, mut shell, mut app, fake) = page_at(1500.0, 1000.0, 1.0, two_heads());
    app.monitors.draft[1].enabled = false;
    app.monitors.edited();
    h.shell_settle(&mut shell, &mut app, 8);
    let apply = button(&h, "apply").unwrap();
    click(&mut h, &mut shell, &mut app, apply.center());
    fake.answer(Outcome::Succeeded);
    h.shell_settle(&mut shell, &mut app, 8);
    assert!(!fake.heads()[1].enabled);

    // Off to look at wallpapers; the clock runs on.
    app.page = Page::Wallpaper;
    h.advance(14.0);
    h.shell_settle(&mut shell, &mut app, 8);
    assert_eq!(fake.asked().len(), 1);
    h.advance(1.5);
    h.shell_settle(&mut shell, &mut app, 8);
    assert_eq!(fake.asked().len(), 2, "time is up: it is put back");
    assert!(fake.asked()[1][1].enabled);
    fake.answer(Outcome::Succeeded);
    h.shell_settle(&mut shell, &mut app, 8);
    assert!(fake.heads()[1].enabled && app.config.monitors.is_empty());
}

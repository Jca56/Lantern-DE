//! The whole bar on Lantern UI's test harness, no window and no GPU: the
//! shell and the app as they run, clicked the way a hand would.

use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Shell, WidgetId};

use crate::app::{App, Editor};
use crate::bar;
use crate::hide::{self, HideOptions, Visibility};
use crate::settings::Settings;

/// The bar as it opens on a 1.3x monitor 2954 logical wide.
fn bar_at() -> (Harness, Shell<App>, App) {
    bar_with(Settings::default())
}

fn bar_with(settings: Settings) -> (Harness, Shell<App>, App) {
    let scale = 1.3;
    let mut h = Harness::new(2954.0 * scale, f64::from(settings.shape.height + settings.shape.below) * scale);
    h.scale = scale;
    let mut shell = Shell::new(Editor::Bar);
    shell.prefs.theme = bar::theme(&settings.desktop);
    (h, shell, App::new(settings, None))
}

/// The id of something `path` deep inside the bar's one area.
fn in_body(h: &Harness, path: impl Fn(WidgetId) -> WidgetId) -> WidgetId {
    (0..8).map(|area| path(WidgetId::ROOT.with_u64(area).with("body"))).find(|id| h.rect_of(*id).is_some()).expect("laid out in the body")
}

#[test]
fn clicking_the_launcher_button_asks_for_the_menu() {
    let (mut h, mut shell, mut app) = bar_at();
    h.shell_settle(&mut shell, &mut app, 4);
    let at = h.rect_of(in_body(&h, |b| b.with("launcher"))).unwrap().center();
    h.move_to(at);
    h.press();
    h.shell_frame(&mut shell, &mut app);
    h.release();
    h.shell_settle(&mut shell, &mut app, 4);
    let opened = shell.take_new_windows();
    assert_eq!(opened.len(), 1, "one menu window asked for");
    assert!(opened[0].layer.is_some());
    let _ = Vec2::ZERO;
}

/// As the live bar gets it: no title bar, no resize edges, never focused
/// or maximized, and a quick click's press and release in one frame.
#[test]
fn a_quick_click_on_an_unfocused_bar_asks_for_the_menu() {
    use lntrn_ui::{Event, Modifiers, MouseButton, WindowState};
    let (mut h, mut shell, mut app) = bar_at();
    (shell.title_bar, shell.resize_edges) = (false, false);
    h.shell_settle(&mut shell, &mut app, 4);
    let at = h.rect_of(in_body(&h, |b| b.with("launcher"))).unwrap().center();
    let ws = WindowState { maximized: false, focused: false };
    let window = h.window();
    let mut frame = |events: &[Event]| shell.frame(&mut app, events, window, 1.3, ws, &mut h.text, &mut h.draw);
    frame(&[Event::PointerMoved(at)]);
    let click = |pressed| Event::Button { button: MouseButton::Left, pressed, pos: at, mods: Modifiers::NONE };
    let out = frame(&[click(true), click(false)]);
    if out.rebuild_again {
        frame(&[]);
    }
    assert_eq!(shell.take_new_windows().len(), 1, "one menu window asked for");
}

/// Auto hide as the live bar runs it, with the compositor's side left out
/// (the harness's window keeps its size): the surface the bar asks for,
/// frame by frame.
#[test]
fn auto_hide_slides_away_to_the_strip_and_back_with_the_windows_giving_way() {
    let hide = HideOptions { visibility: Visibility::AutoHide, delay: 0.5, slide: 0.25, ..HideOptions::default() };
    let (mut h, mut shell, mut app) = bar_with(Settings { hide, ..Settings::default() });
    let at = |h: &mut Harness, shell: &mut Shell<App>, app: &mut App, t: f64| {
        h.advance(t - h.time());
        h.shell_settle(shell, app, 4);
        shell.take_layer_change()
    };
    // Up as it starts, waiting out the delay with the pointer elsewhere.
    assert_eq!(at(&mut h, &mut shell, &mut app, 0.0), None, "the surface it was made with");
    assert_eq!(at(&mut h, &mut shell, &mut app, 0.6), None, "sets off down, still all there");
    let mid = at(&mut h, &mut shell, &mut app, 0.7).expect("the zone shrinks as it slides");
    assert!(mid.exclusive_zone > 0 && mid.exclusive_zone < 72, "{}", mid.exclusive_zone);
    assert_eq!(mid.size, (0, 72), "still full height while it slides");
    let hidden = at(&mut h, &mut shell, &mut app, 0.9).expect("out of sight");
    assert_eq!((hidden.size, hidden.exclusive_zone), ((0, hide::STRIP), 0));
    // The pointer pushed against the bottom edge.
    h.move_to(Vec2::new(1500.0, 92.0));
    let grow = at(&mut h, &mut shell, &mut app, 1.0).expect("room to come up in");
    assert_eq!((grow.size, grow.exclusive_zone), ((0, 72), 0));
    let up = at(&mut h, &mut shell, &mut app, 1.3).expect("all the way up");
    assert_eq!((up.size, up.exclusive_zone), ((0, 72), 72));
    // The pointer resting on the bar keeps it there.
    assert_eq!(at(&mut h, &mut shell, &mut app, 5.0), None);
}

/// A Super tap with the bar out of sight only peeks: up as from the edge,
/// no launcher, held for the peek time rather than the hover delay, then
/// away again. A tap with the bar up opens the launcher.
#[test]
fn a_super_tap_peeks_at_a_hidden_bar_and_a_second_opens_the_launcher() {
    let hide = HideOptions { visibility: Visibility::AutoHide, delay: 0.5, slide: 0.25, peek: 2.0 };
    let (mut h, mut shell, mut app) = bar_with(Settings { hide, ..Settings::default() });
    let at = |h: &mut Harness, shell: &mut Shell<App>, app: &mut App, t: f64| {
        h.advance(t - h.time());
        h.shell_settle(shell, app, 4);
        shell.take_layer_change()
    };
    for t in [0.0, 0.6, 0.7] {
        at(&mut h, &mut shell, &mut app, t);
    }
    let hidden = at(&mut h, &mut shell, &mut app, 0.9).expect("out of sight");
    assert_eq!(hidden.size, (0, hide::STRIP));
    // Super, with the pointer nowhere near.
    app.link().feed("launcher:toggle");
    let grow = at(&mut h, &mut shell, &mut app, 1.0).expect("comes up as from the edge");
    assert_eq!((grow.size, grow.exclusive_zone), ((0, 72), 0));
    let up = at(&mut h, &mut shell, &mut app, 1.3).expect("all the way up");
    assert_eq!(up.exclusive_zone, 72);
    assert!(shell.take_new_windows().is_empty(), "a peek, not the launcher");
    // Held from landing for the peek's 2 s, well past the 0.5 s delay.
    assert_eq!(at(&mut h, &mut shell, &mut app, 3.0), None, "still up for its look");
    at(&mut h, &mut shell, &mut app, 3.4);
    let away = at(&mut h, &mut shell, &mut app, 3.8).expect("away again");
    assert_eq!((away.size, away.exclusive_zone), ((0, hide::STRIP), 0));
    // Peek again, then a second tap while it is up: the launcher.
    app.link().feed("launcher:toggle");
    at(&mut h, &mut shell, &mut app, 4.0);
    at(&mut h, &mut shell, &mut app, 4.3);
    assert!(shell.take_new_windows().is_empty());
    app.link().feed("launcher:toggle");
    at(&mut h, &mut shell, &mut app, 4.4);
    assert_eq!(shell.take_new_windows().len(), 1, "the launcher's menu asked for");
}

/// See-through windows make the plate glass: the compositor is told where
/// to blur behind it. Out of sight with auto hide, it's told the bar waits
/// at the bottom edge (and that there's nothing to blur).
#[test]
fn the_plate_is_glass_for_the_compositor_and_a_hidden_bar_waits_at_the_edge() {
    let hide = HideOptions { visibility: Visibility::AutoHide, delay: 0.5, slide: 0.25, ..HideOptions::default() };
    let desktop = lntrn_kit::desktop::Desktop { opacity: 0.8, ..lntrn_kit::desktop::Desktop::default() };
    let (mut h, mut shell, mut app) = bar_with(Settings { hide, desktop, ..Settings::default() });
    // As the live bar: the surface is the whole window.
    (shell.title_bar, shell.resize_edges) = (false, false);
    h.shell_settle(&mut shell, &mut app, 4);
    let sent = app.link().take_sent();
    let blur = sent.iter().find(|l| l.starts_with("blur:lntrn-control-center\t")).expect("where the plate is");
    let n: Vec<i64> = blur.split('\t').nth(1).unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
    let near = |a: i64, b: i64| (a - b).abs() <= 1;
    assert!(near(n[0], 16) && near(n[1], 0) && near(n[2], 2954 - 32) && near(n[3], 64) && near(n[4], 16), "the floating plate, logical: {n:?}");
    assert!(sent.contains(&"edge:lntrn-control-center:0".to_owned()));
    // The delay waited out, then the slide.
    for step in [0.6, 0.3, 0.3] {
        h.advance(step);
        h.shell_settle(&mut shell, &mut app, 4);
    }
    let sent = app.link().take_sent();
    assert!(sent.contains(&"blur:lntrn-control-center".to_owned()), "out of sight: nothing to blur");
    assert!(sent.contains(&"edge:lntrn-control-center:1".to_owned()));
}

/// The dock as the live bar runs it, the compositor's window feed fed by
/// hand: apps with no desktop entry, so this machine's installed apps
/// don't matter.
#[test]
fn the_dock_lights_running_apps_and_a_click_minimizes_focuses_or_shows_them_all() {
    let (mut h, mut shell, mut app) = bar_at();
    app.link().feed("win:1\t-\ttest-app-a\tFirst");
    app.link().feed("win:2\t-\ttest-app-a\tSecond");
    app.link().feed("win:3\ta\ttest-app-b\tFocused");
    h.shell_settle(&mut shell, &mut app, 4);
    let face = |h: &Harness, key: &str| h.rect_of(in_body(h, |b| b.with("dock").with(key))).expect("a face on the dock");
    let (a, b) = (face(&h, "test-app-a"), face(&h, "test-app-b"));
    assert!(a.max.x <= b.min.x, "in the order their first windows opened");
    let sent = app.link().take_sent();
    assert!(sent.iter().any(|l| l.starts_with("icons:lntrn-control-center\ttest-app-a ")), "the compositor learns where the icons are: {sent:?}");
    let click = |h: &mut Harness, shell: &mut Shell<App>, app: &mut App, at: Vec2| {
        h.move_to(at);
        h.press();
        h.shell_frame(shell, app);
        h.release();
        h.shell_settle(shell, app, 4);
    };
    // The focused app's one window: minimized.
    click(&mut h, &mut shell, &mut app, b.center());
    assert!(app.link().take_sent().contains(&"minimize:3".to_owned()));
    // An app with two windows: its card, in the overlay.
    click(&mut h, &mut shell, &mut app, a.center());
    assert_eq!(shell.take_new_windows().len(), 1, "the overlay opens for the card");
    // Its windows gone, the app leaves the dock.
    app.link().feed("win-gone:1");
    app.link().feed("win-gone:2");
    h.shell_settle(&mut shell, &mut app, 4);
    assert!(h.rect_of(in_body(&h, |b| b.with("dock").with("test-app-b"))).is_some());
    assert!((0..8).all(|area| h.rect_of(WidgetId::ROOT.with_u64(area).with("body").with("dock").with("test-app-a")).is_none()));
}


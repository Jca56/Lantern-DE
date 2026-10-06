//! Headless runs on Lantern UI's test harness, no window and no GPU: the
//! whole app on every page at several window sizes and scales, and the
//! kit's controls clicked, dragged and keyed the way a hand would.

use std::sync::Once;

use lntrn_image::Image;
use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, Shell, Ui, WidgetId};

use crate::app::{App, Editor};
use crate::config::Config;
use crate::kit;
use crate::look;
use crate::nav::Page;
use crate::pages::{effects, notepad, notifications, terminal};

/// Point the desktop's folders at a scratch tree holding two wallpapers
/// (and a film, which is not one), so nothing here reads or writes the
/// real `~/.lantern` or the real thumbnail cache.
fn sandbox() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let root = std::env::temp_dir().join("lntrn-settings-smoke");
        let walls = root.join("home/wallpapers");
        std::fs::create_dir_all(&walls).unwrap();
        let picture = lntrn_image::encode_png(&Image::solid(64, 40, [40, 80, 160, 255]));
        for name in ["Blue Forest.png", "Red_Moon.png", "clip.mp4"] {
            std::fs::write(walls.join(name), &picture).unwrap();
        }
        // SAFETY: set once, before anything here reads them.
        unsafe {
            std::env::set_var("LANTERN_HOME", root.join("home"));
            std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
        }
    });
}

/// The app in a window `width` × `height` logical pixels at `scale`.
fn app_at(width: f64, height: f64, scale: f64) -> (Harness, Shell<App>, App) {
    sandbox();
    kit::probe::watch();
    let mut h = Harness::new(width * scale, height * scale);
    h.scale = scale;
    let mut shell = Shell::new(Editor::Settings);
    shell.prefs.theme = look::theme(look::GOLD);
    (h, shell, App::new(Config::empty()))
}

/// The id of something `path` deep inside the app's one area.
fn in_body(h: &Harness, path: impl Fn(WidgetId) -> WidgetId) -> WidgetId {
    (0..8).map(|area| path(WidgetId::ROOT.with_u64(area).with("body"))).find(|id| h.rect_of(*id).is_some()).expect("laid out in the body")
}

fn click(h: &mut Harness, shell: &mut Shell<App>, app: &mut App, at: Vec2) {
    h.move_to(at);
    h.press();
    h.shell_frame(shell, app);
    h.release();
    h.shell_settle(shell, app, 8);
}

fn clipped() -> Vec<String> {
    kit::probe::take_clipped()
}

fn stacked() -> Vec<String> {
    kit::probe::take_stacked()
}

#[test]
fn every_page_settles_at_every_size() {
    // The first is the smallest the window gets. The second is what the
    // desktop opens it at on a 1920 x 1200 screen (60% of it), so that is
    // the size most looked at. The last is narrower than the window is
    // allowed to be (a tile on a small screen, say): it must still lay
    // out, if not prettily.
    for (w, h, scale, roomy) in [(1100.0, 700.0, 1.0, true), (1152.0, 720.0, 1.0, true), (1500.0, 1000.0, 1.0, true), (1920.0, 1160.0, 1.0, true), (1152.0, 720.0, 1.4, true), (760.0, 560.0, 1.0, false)] {
        let (mut hs, mut shell, mut app) = app_at(w, h, scale);
        for page in Page::ALL {
            app.page = page;
            clipped();
            stacked();
            let out = hs.shell_settle(&mut shell, &mut app, 8);
            assert!(!out.rebuild_again, "{page:?} at {w}x{h} @{scale} keeps asking for rebuilds");
            let (cut, under) = (clipped(), stacked());
            assert!(!roomy || cut.is_empty(), "{page:?} at {w}x{h} @{scale}: labels cut short: {cut:?}");
            // At the size it opens at, every control is beside its label.
            assert!(w < 1152.0 || !roomy || under.is_empty(), "{page:?} at {w}x{h} @{scale}: controls pushed under their labels: {under:?}");
        }
    }
}

#[test]
fn the_sidebar_switches_pages_and_a_tile_puts_a_wallpaper_up() {
    let (mut h, mut shell, mut app) = app_at(1500.0, 1000.0, 1.0);
    h.shell_settle(&mut shell, &mut app, 8);
    assert_eq!(app.page, Page::Wallpaper);

    // Two pictures in the folder; the film is not listed.
    let tile = |i: usize| move |body: WidgetId| body.with_index(1).with("wallpaper").with("page").with_index(1).with("tiles").with("tile").with_index(i);
    assert!((0..8).all(|area| h.rect_of(tile(2)(WidgetId::ROOT.with_u64(area).with("body"))).is_none()));
    let second = h.rect_of(in_body(&h, tile(1))).unwrap();
    assert!(app.config.appearance.wallpaper.is_empty());
    click(&mut h, &mut shell, &mut app, second.center());
    assert!(app.config.appearance.wallpaper.ends_with("wallpapers/Red_Moon.png"), "{}", app.config.appearance.wallpaper);

    let row = h.rect_of(in_body(&h, |body| body.with_index(0).with("sidebar").with("mouse"))).unwrap();
    click(&mut h, &mut shell, &mut app, row.center());
    assert_eq!(app.page, Page::Mouse);
}

/// A harness for kit code alone, wearing the app's look.
fn kit_at(width: f64, height: f64) -> Harness {
    kit::probe::watch();
    let mut h = Harness::new(width, height);
    h.theme = look::theme(look::GOLD);
    h
}

#[derive(Default)]
struct Values {
    on: bool,
    level: f64,
    pick: usize,
    word: String,
    font: usize,
    hex: String,
    last: bool,
}

fn one_of_each(ui: &mut Ui, v: &mut Values) {
    kit::card(ui, "c", |c| {
        c.switch("Switch", "A hint under it.", &mut v.on);
        c.slider("Level", "", &mut v.level, (0.0, 1.0), 0.05, kit::percent);
        c.segmented("Pick", "", &mut v.pick, &["One", "Two", "Three"]);
        c.choice("Word", "", &mut v.word, &[("suspend", "Suspend"), ("lock", "Lock")]);
        c.dropdown("Font", "", &mut v.font, &["Inter", "Mono", "Serif"]);
        c.color("Accent", "", &mut v.hex, look::GOLD);
        c.switch("Last", "", &mut v.last);
    });
}

#[test]
fn a_cards_controls_answer_the_pointer_and_the_keys() {
    let mut h = kit_at(1000.0, 900.0);
    let mut v = Values { level: 0.2, word: "lock".into(), hex: "#2563EB".into(), ..Values::default() };
    let id = |label: &str| WidgetId::ROOT.with("c").with(label);

    // The card learns its height on the first pass and is settled by the
    // second.
    assert!(h.frame(|ui| one_of_each(ui, &mut v)).rebuild_again);
    assert!(!h.frame(|ui| one_of_each(ui, &mut v)).rebuild_again);

    // A switch's whole row is its target: the middle of it is nowhere
    // near the pill.
    h.click_on(id("Switch"), |ui| one_of_each(ui, &mut v));
    assert!(v.on);

    // A slider drags end to end, lands on its steps, and steps by arrow.
    let track = h.rect_of(id("Level")).unwrap();
    let y = track.center().y;
    h.drag(Vec2::new(track.min.x + 20.0, y), Vec2::new(track.max.x + 40.0, y), 4, |ui| one_of_each(ui, &mut v));
    assert_eq!(v.level, 1.0);
    h.drag(Vec2::new(track.max.x - 20.0, y), Vec2::new(track.center().x + 3.0, y), 4, |ui| one_of_each(ui, &mut v));
    assert_eq!(v.level, 0.5);
    h.key(Key::ArrowRight);
    h.settle(4, |ui| one_of_each(ui, &mut v));
    assert_eq!(v.level, 0.55);

    h.click_on(id("Pick").with_index(2), |ui| one_of_each(ui, &mut v));
    assert_eq!(v.pick, 2);
    h.click_on(id("Word").with_index(0), |ui| one_of_each(ui, &mut v));
    assert_eq!(v.word, "suspend");

    // A dropdown opens its list and takes the row that is clicked.
    h.click_on(id("Font"), |ui| one_of_each(ui, &mut v));
    h.click_on(id("Font").with("item").with_index(2), |ui| one_of_each(ui, &mut v));
    assert_eq!(v.font, 2);

    // The colour chip is at the right end of its row (the card runs 10
    // to 990, its rows 25 in), the picker opens from it, and the row
    // after it starts where a row should: the layout came back.
    let chip = id("Accent").with("");
    let swatch = h.rect_of(chip).unwrap();
    assert!((swatch.max.x - 965.0).abs() <= 1.0 && (swatch.width() - 50.0).abs() <= 1.0, "{swatch:?}");
    let after = h.rect_of(id("Last")).unwrap();
    assert!((after.min.y - swatch.center().y - 40.0).abs() <= 1.0, "{swatch:?} then {after:?}");
    h.click_on(chip, |ui| one_of_each(ui, &mut v));
    assert!(*h.state.open(chip), "the picker is open");
    assert_eq!(v.hex, "#2563EB", "opening it changes nothing");
    assert!(clipped().is_empty());
}

#[test]
fn a_narrow_card_puts_a_wide_control_under_its_label() {
    let mut h = kit_at(560.0, 600.0);
    let mut pick = 0;
    let options = ["Suspend", "Hibernate", "Lock", "Nothing"];
    clipped();
    h.settle(4, |ui| kit::card(ui, "c", |c| {
        c.segmented("On battery", "", &mut pick, &options);
    }));
    let id = WidgetId::ROOT.with("c").with("On battery");
    let (first, last) = (h.rect_of(id.with_index(0)).unwrap(), h.rect_of(id.with_index(3)).unwrap());
    // All four inside the card's rows (10 + 25 to 550 - 25), below the
    // line the label is on.
    assert!(first.min.x >= 34.0 && last.max.x <= 526.0, "{first:?} .. {last:?}");
    assert!(first.min.y >= 10.0 + 15.0 + 25.0, "{first:?}");
    assert!(clipped().is_empty());
    h.click_on(id.with_index(3), |ui| kit::card(ui, "c", |c| {
        c.segmented("On battery", "", &mut pick, &options);
    }));
    assert_eq!(pick, 3);
}

#[test]
fn a_page_is_a_capped_centred_column() {
    let column = |width: f64| {
        let mut h = kit_at(width, 800.0);
        let mut seen = (0.0, 0.0);
        h.settle(4, |ui| kit::page(ui, "Title", "A line about it.", |ui| seen = (ui.cursor().x, ui.avail_width())));
        seen
    };
    // A wide window: 980 across, the same room either side (the body is
    // inset 10 and keeps 20 for its scrollbar).
    let (x, w) = column(2000.0);
    assert_eq!(w, 980.0);
    assert!((x - 10.0 - (1960.0 - 980.0) * 0.5).abs() <= 6.0, "{x}");
    // A narrow one: what there is, less a margin of 20 each side.
    assert_eq!(column(700.0).1, 700.0 - 20.0 - 20.0 - 40.0);
}

#[test]
fn the_glow_dots_and_the_corner_picker_set_what_they_show() {
    let mut cfg = Config::empty();
    let mut h = kit_at(1000.0, 2600.0);
    assert!(cfg.appearance.window_gradient_stops.iter().all(String::is_empty));
    let dot = WidgetId::ROOT.with("glow").with("dots").with("dot");
    h.settle(4, |ui| {
        effects::draw(&mut cfg, ui);
    });
    h.click_on(dot.with_index(3), |ui| {
        effects::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.appearance.window_gradient_stops, ["", "", "", "#FFC800", ""], "bottom-right lit in the accent");
    h.click_on(dot.with_index(3), |ui| {
        effects::draw(&mut cfg, ui);
    });
    assert!(cfg.appearance.window_gradient_stops.iter().all(String::is_empty));

    let mut h = kit_at(1000.0, 1400.0);
    let corner = WidgetId::ROOT.with("toasts").with("position");
    assert_eq!(cfg.notifications.position, "top-right");
    h.settle(4, |ui| {
        notifications::draw(&mut cfg, ui);
    });
    h.click_on(corner.with_index(3), |ui| {
        notifications::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.notifications.position, "bottom-left");
}

#[test]
fn the_terminal_page_sets_its_size_cursor_and_bar() {
    let mut cfg = Config::empty();
    let mut h = kit_at(1000.0, 1000.0);
    assert_eq!((cfg.terminal.font_size, cfg.terminal.cursor_style.as_str(), cfg.terminal.open_bar_hidden), (20.0, "block", false));
    clipped();
    h.settle(4, |ui| {
        terminal::draw(&mut cfg, ui);
    });
    // The size drags along its whole range, by halves.
    let track = h.rect_of(WidgetId::ROOT.with("text").with("Text size")).unwrap();
    let y = track.center().y;
    h.drag(Vec2::new(track.center().x, y), Vec2::new(track.max.x + 40.0, y), 4, |ui| {
        terminal::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.terminal.font_size, 40.0);
    h.key(Key::ArrowLeft);
    h.settle(4, |ui| {
        terminal::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.terminal.font_size, 39.5);
    // The biggest text still fits its sample line's well, and nothing on
    // the page is cut short.
    h.click_on(WidgetId::ROOT.with("text").with("Cursor").with_index(2), |ui| {
        terminal::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.terminal.cursor_style, "beam");
    h.click_on(WidgetId::ROOT.with("window").with("Open with the title bar hidden"), |ui| {
        terminal::draw(&mut cfg, ui);
    });
    assert!(cfg.terminal.open_bar_hidden);
    assert!(clipped().is_empty());
}

#[test]
fn the_notepad_page_sets_its_page_and_width() {
    let mut cfg = Config::empty();
    let mut h = kit_at(1000.0, 1000.0);
    assert_eq!((cfg.notepad.theme.as_str(), cfg.notepad.page_width), ("paper", 0.82));
    clipped();
    h.settle(4, |ui| {
        notepad::draw(&mut cfg, ui);
    });
    h.click_on(WidgetId::ROOT.with("page").with("Page").with_index(1), |ui| {
        notepad::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.notepad.theme, "dark");
    // The width drags along its whole range, by hundredths.
    let track = h.rect_of(WidgetId::ROOT.with("page").with("Page width")).unwrap();
    let y = track.center().y;
    h.drag(Vec2::new(track.center().x, y), Vec2::new(track.min.x - 40.0, y), 4, |ui| {
        notepad::draw(&mut cfg, ui);
    });
    assert_eq!(cfg.notepad.page_width, 0.0);
    h.key(Key::ArrowRight);
    h.settle(4, |ui| {
        notepad::draw(&mut cfg, ui);
    });
    assert!((cfg.notepad.page_width - 0.01).abs() < 1e-9, "{}", cfg.notepad.page_width);
    assert!(clipped().is_empty());
}

//! The menu on Lantern UI's test harness, no window and no GPU: typed,
//! keyed and clicked the way a hand would, on apps of our own rather than
//! this machine's.

use std::cell::RefCell;

use lntrn_apps::{App, Group, Icons};
use lntrn_kit::desktop::Desktop;
use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, Ui, WidgetId};

use super::{Launcher, LauncherOptions, Layout, Outcome, Place, PowerAction, View, menu};
use crate::idlist::IdList;
use crate::bar::{self, Look};

const PLACE: Place = Place { left: 16.0, clear: 72.0 };

thread_local! {
    /// One icon cache a test, as the app has one.
    static ICONS: RefCell<Icons> = RefCell::new(Icons::new(96));
}

/// One frame of the menu, as the app draws it.
fn frame(l: &mut Launcher, ui: &mut Ui) -> Outcome {
    ICONS.with_borrow_mut(|icons| menu(l, icons, ui, PLACE, Look::of(&Desktop::default())))
}

fn app(id: &str, name: &str, generic: &str, categories: &[&str]) -> App {
    App { id: id.into(), name: name.into(), generic: generic.into(), categories: categories.iter().map(|c| (*c).to_owned()).collect(), command: id.into(), ..App::default() }
}

/// A launcher open on three apps (by name, as `installed` gives them), on
/// a harness the size of a small screen.
fn open() -> (Harness, Launcher) {
    let mut h = Harness::new(1600.0, 1000.0);
    h.theme = bar::theme(&Desktop::default());
    let mut l = Launcher::new();
    l.load(vec![app("lntrn-code", "Code", "Text Editor", &["Development"]), app("firefox-bin", "Mozilla Firefox", "Web Browser", &["Network", "WebBrowser"]), app("steam", "Steam", "", &["Network", "FileTransfer", "Game"])]);
    (h, l)
}

/// Run frames until the menu settles; what it last asked for besides
/// staying.
fn settle(h: &mut Harness, l: &mut Launcher) -> Outcome {
    let mut got = Outcome::Stay;
    h.settle(6, |ui| {
        let out = frame(l, ui);
        if out != Outcome::Stay {
            got = out;
        }
    });
    got
}

fn panel(path: impl Fn(WidgetId) -> WidgetId) -> WidgetId {
    path(WidgetId::ROOT.with("panel"))
}

/// Click `id` the way a hand would; what the menu asked for besides
/// staying.
fn click(h: &mut Harness, l: &mut Launcher, id: WidgetId) -> Outcome {
    let at = h.rect_of(id).expect("laid out").center();
    let mut got = Outcome::Stay;
    h.click_at(at, |ui| {
        let out = frame(l, ui);
        if out != Outcome::Stay {
            got = out;
        }
    });
    got
}

/// Pick a row of the sidebar.
fn pick(h: &mut Harness, l: &mut Launcher, label: &str) {
    click(h, l, panel(|p| p.with_index(0).with(label)));
    settle(h, l);
}

#[test]
fn typing_searches_at_once_and_enter_starts_the_best_match() {
    let (mut h, mut l) = open();
    assert_eq!(l.groups, vec![Group::Development, Group::Games, Group::Internet]);
    settle(&mut h, &mut l);
    h.type_text("ste");
    assert_eq!(settle(&mut h, &mut l), Outcome::Stay);
    assert_eq!(l.query, "ste", "the search field has the keyboard from the start");
    h.key(Key::Enter);
    assert_eq!(settle(&mut h, &mut l), Outcome::Launch("steam".into()));
}

#[test]
fn the_arrows_pick_and_escape_clears_the_search_then_closes() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    assert_eq!(l.selected, None, "nothing lit till asked");
    h.key(Key::Enter);
    assert_eq!(settle(&mut h, &mut l), Outcome::Stay, "Enter with nothing lit starts nothing");
    // The first press lights the first app, then each moves on.
    for _ in 0..4 {
        h.key(Key::ArrowDown);
    }
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(2), "held at the last");
    h.key(Key::ArrowUp);
    h.key(Key::Enter);
    assert_eq!(settle(&mut h, &mut l), Outcome::Launch("firefox-bin".into()));
    h.type_text("zzz");
    settle(&mut h, &mut l);
    h.key(Key::Escape);
    assert_eq!(settle(&mut h, &mut l), Outcome::Stay);
    assert!(l.query.is_empty());
    h.key(Key::Escape);
    assert_eq!(settle(&mut h, &mut l), Outcome::Close);
}

#[test]
fn typing_after_picking_a_group_still_searches() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "Games");
    h.type_text("fire");
    settle(&mut h, &mut l);
    assert_eq!((l.query.as_str(), l.shown()), ("fire", vec![1]));
}

#[test]
fn a_group_shows_its_apps_and_a_click_off_the_panel_closes() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "Games");
    assert_eq!((l.view, l.shown()), (View::Group(Group::Games), vec![2]));
    let steam = h.rect_of(panel(|p| p.with_index(1).with("app").with_index(2))).expect("Steam's row");
    let mut got = Outcome::Stay;
    h.click_at(steam.center(), |ui| {
        let out = frame(&mut l, ui);
        if out != Outcome::Stay {
            got = out;
        }
    });
    assert_eq!(got, Outcome::Launch("steam".into()));
    // Above the panel, where the screen shows through.
    h.move_to(Vec2::new(1500.0, 20.0));
    h.press();
    assert_eq!(settle(&mut h, &mut l), Outcome::Close);
}

#[test]
fn power_asks_first_but_for_lock() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    assert_eq!(click(&mut h, &mut l, panel(|p| p.with("Restart"))), Outcome::Stay);
    assert_eq!(l.confirm, Some(PowerAction::Restart));
    assert_eq!(click(&mut h, &mut l, panel(|p| p.with("no"))), Outcome::Stay);
    assert_eq!(l.confirm, None);
    click(&mut h, &mut l, panel(|p| p.with("Shut Down")));
    assert_eq!(click(&mut h, &mut l, panel(|p| p.with("yes"))), Outcome::Power(PowerAction::ShutDown));
    assert_eq!(click(&mut h, &mut l, panel(|p| p.with("Lock"))), Outcome::Power(PowerAction::Lock));
}

#[test]
fn it_opens_on_the_favorites_in_their_order_with_a_hint_when_there_are_none() {
    let (mut h, mut l) = open();
    assert_eq!((l.view, l.shown()), (View::Favorites, vec![]));
    assert_eq!(settle(&mut h, &mut l), Outcome::Stay);
    // An app uninstalled since is left out; the rest keep the list's order.
    l.favorites = IdList::in_memory(&["steam", "gone-app", "lntrn-code"]);
    assert_eq!(l.shown(), vec![2, 0]);
    pick(&mut h, &mut l, "All apps");
    assert_eq!(l.shown(), vec![0, 1, 2]);
}

#[test]
fn a_right_click_asks_for_the_apps_menu_and_a_favorite_shows_under_favorites() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    let steam = h.rect_of(panel(|p| p.with_index(1).with("app").with_index(2))).expect("Steam's row");
    h.move_to(steam.center());
    h.right_press();
    assert_eq!(settle(&mut h, &mut l), Outcome::Menu(2));
    assert_eq!(l.selected, Some(2), "the row the menu is about stays lit");
    assert!(!l.is_favorite("steam"));
    l.toggle_favorite("steam");
    assert!(l.is_favorite("steam"));
    pick(&mut h, &mut l, "Favorites");
    assert_eq!(l.shown(), vec![2]);
    l.toggle_favorite("steam");
    assert!(l.shown().is_empty());
}

#[test]
fn quick_kill_asks_to_start_over_and_steps_aside_for_a_power_question() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    assert_eq!(click(&mut h, &mut l, panel(|p| p.with("Quick Kill"))), Outcome::QuickKill);
    click(&mut h, &mut l, panel(|p| p.with("Restart")));
    assert_eq!(l.confirm, Some(PowerAction::Restart));
    assert!(h.rect_of(panel(|p| p.with("Quick Kill"))).is_none(), "the question has the whole row");
}

/// The launcher open on its three apps as a grid, on "All apps".
fn grid() -> (Harness, Launcher) {
    let (mut h, mut l) = open();
    l.follow(LauncherOptions { layout: Layout::Grid, ..LauncherOptions::default() });
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    (h, l)
}

/// The tile of the app at `index`.
fn tile(index: usize) -> WidgetId {
    panel(|p| p.with_index(1).with("tile").with_index(index))
}

#[test]
fn the_grid_lays_the_apps_out_as_tiles_and_a_click_starts_one() {
    let (mut h, mut l) = grid();
    let tiles: Vec<_> = (0..3).map(|i| h.rect_of(tile(i)).expect("a tile")).collect();
    assert!(tiles.iter().all(|t| t.min.y == tiles[0].min.y), "side by side on one row");
    assert!(tiles[0].max.x <= tiles[1].min.x && tiles[1].max.x <= tiles[2].min.x, "left to right in order");
    assert!(h.rect_of(panel(|p| p.with_index(1).with("app").with_index(0))).is_none(), "no rows");
    assert_eq!(click(&mut h, &mut l, tile(2)), Outcome::Launch("steam".into()));
}

#[test]
fn in_the_grid_the_arrows_go_tile_to_tile_and_enter_starts_the_one_picked() {
    let (mut h, mut l) = grid();
    for _ in 0..3 {
        h.key(Key::ArrowRight);
    }
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(2), "the first press lit the first tile");
    h.key(Key::ArrowLeft);
    h.key(Key::Enter);
    assert_eq!(settle(&mut h, &mut l), Outcome::Launch("firefox-bin".into()));
}

#[test]
fn in_the_list_left_and_right_stay_the_search_fields() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    h.key(Key::ArrowRight);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, None, "no move in the list");
}

#[test]
fn the_buttons_by_the_search_switch_the_layout_at_once_and_ask_to_keep_it() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    let button = |word: &'static str| panel(move |p| p.with_index(1).with(word));
    assert_eq!(click(&mut h, &mut l, button("list")), Outcome::Stay, "already the list");
    assert_eq!(click(&mut h, &mut l, button("grid")), Outcome::Layout(Layout::Grid));
    settle(&mut h, &mut l);
    assert_eq!(l.options.layout, Layout::Grid);
    assert!(h.rect_of(tile(0)).is_some(), "the apps as tiles at once");
    // The settings catching up with the file change nothing.
    l.follow(LauncherOptions { layout: Layout::Grid, ..LauncherOptions::default() });
    settle(&mut h, &mut l);
    assert!(h.rect_of(tile(0)).is_some());
    assert_eq!(click(&mut h, &mut l, button("list")), Outcome::Layout(Layout::List));
}

/// The rect of the list row of the app at `index`.
fn row(index: usize) -> WidgetId {
    panel(|p| p.with_index(1).with("app").with_index(index))
}

#[test]
fn the_light_follows_the_pointer_and_leaves_with_it_off_the_apps() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    let firefox = h.rect_of(row(1)).expect("Firefox's row").center();
    h.move_to(firefox);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(1), "lit under the pointer");
    h.key(Key::Enter);
    assert_eq!(settle(&mut h, &mut l), Outcome::Launch("firefox-bin".into()), "Enter starts what the pointer lit");
    let steam = h.rect_of(row(2)).expect("Steam's row").center();
    h.move_to(steam);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(2));
    // Off the apps, onto the sidebar: the pointer's light goes with it.
    let side = h.rect_of(panel(|p| p.with_index(0).with("Favorites"))).expect("the sidebar").center();
    h.move_to(side);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, None);
    // The keyboard's light stays under a pointer at rest, and off the apps.
    h.key(Key::ArrowDown);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(0));
    h.move_to(side + lntrn_math::Vec2::new(0.0, 4.0));
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(0), "a light the keyboard lit isn't the pointer's to put out");
}

#[test]
fn in_the_grid_the_light_follows_the_pointer_too() {
    let (mut h, mut l) = grid();
    assert_eq!(l.selected, None);
    let steam = h.rect_of(tile(2)).expect("Steam's tile").center();
    h.move_to(steam);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(2));
    let code = h.rect_of(tile(0)).expect("Code's tile").center();
    h.move_to(code);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, Some(0), "tile to tile");
    assert_eq!(click(&mut h, &mut l, tile(1)), Outcome::Launch("firefox-bin".into()), "one click starts one");
}

#[test]
fn a_menu_opening_under_a_resting_pointer_lights_nothing() {
    let (mut h, mut l) = open();
    settle(&mut h, &mut l);
    pick(&mut h, &mut l, "All apps");
    let firefox = h.rect_of(row(1)).expect("Firefox's row").center();
    // Open again with the pointer already over Firefox's row: where it
    // first turns up isn't a move.
    l.close();
    l.load(l.apps.clone());
    l.view = View::All;
    h.move_to(firefox);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, None);
}

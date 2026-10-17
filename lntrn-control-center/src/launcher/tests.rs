//! The menu on Lantern UI's test harness, no window and no GPU: typed,
//! keyed and clicked the way a hand would, on apps of our own rather than
//! this machine's.

use lntrn_apps::{App, Group};
use lntrn_kit::desktop::Desktop;
use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, WidgetId};

use super::{Favorites, Launcher, Outcome, Place, PowerAction, View, menu};
use crate::bar::{self, Look};

const PLACE: Place = Place { left: 16.0, clear: 72.0 };

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
        let out = menu(l, ui, PLACE, Look::of(&Desktop::default()));
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
        let out = menu(l, ui, PLACE, Look::of(&Desktop::default()));
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
    h.key(Key::ArrowDown);
    h.key(Key::ArrowDown);
    h.key(Key::ArrowDown);
    settle(&mut h, &mut l);
    assert_eq!(l.selected, 2, "held at the last");
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
        let out = menu(&mut l, ui, PLACE, Look::of(&Desktop::default()));
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
    l.favorites = Favorites::in_memory(&["steam", "gone-app", "lntrn-code"]);
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
    assert_eq!(l.selected, 2, "the row the menu is about stays lit");
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

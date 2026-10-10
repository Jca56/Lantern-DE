//! The page in the whole app on Lantern UI's test harness: its buttons
//! taken off, moved, picked on the little ring and renamed, the file
//! written once the hand rests, and a button added from the search.

use lntrn_math::Rect;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, WidgetId};

use super::*;
use crate::config::radial::{self, defaults};
use crate::nav::Page;
use crate::smoke::{app_at, click, in_body};

/// Where something on the page is, from the page's own column down.
fn on_page(h: &Harness, path: impl Fn(WidgetId) -> WidgetId) -> Option<Rect> {
    (0..8).find_map(|area| h.rect_of(path(WidgetId::ROOT.with_u64(area).with("body").with_index(1).with("radial").with("page").with_index(1))))
}

#[test]
fn buttons_are_moved_renamed_added_and_the_file_follows() {
    // Tall enough that the whole page shows: a click lands on what it aims at.
    let (mut h, mut shell, mut app) = app_at(1500.0, 2000.0, 1.0);
    // The sandbox's file, not the real one: start without it.
    let file = radial::path();
    assert!(file.starts_with(std::env::temp_dir()), "{}", file.display());
    let _ = std::fs::remove_file(&file);
    app.page = Page::Radial;
    h.shell_settle(&mut shell, &mut app, 8);
    let slots = |app: &crate::app::App| app.radial.ring.as_ref().unwrap().slots.clone();
    assert_eq!(slots(&app), defaults());
    let row = |h: &Harness, i: usize, part: &str| on_page(h, |p| p.with("buttons").with_index(i).with(part));
    assert!(row(&h, 5, "pick").is_some() && row(&h, 6, "pick").is_none());
    // Also where `in_body` finds the page, so the path above is the app's.
    assert!(h.rect_of(in_body(&h, |b| b.with_index(1).with("radial").with("page").with_index(1).with("buttons").with_index(0).with("remove"))).is_some());

    // Off comes the terminal; the file manager, now first, moves a place on.
    let at = row(&h, 0, "remove").unwrap().center();
    click(&mut h, &mut shell, &mut app, at);
    assert_eq!(slots(&app).iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["File Manager", "Firefox", "Notepad", "Screenshot", "Settings"]);
    let at = row(&h, 0, "later").unwrap().center();
    click(&mut h, &mut shell, &mut app, at);
    assert_eq!((slots(&app)[1].label.as_str(), app.radial.selected), ("File Manager", None));
    // Earlier from the top goes round to the end.
    let at = row(&h, 0, "earlier").unwrap().center();
    click(&mut h, &mut shell, &mut app, at);
    assert_eq!((slots(&app)[4].label.as_str(), app.radial.selected), ("Firefox", None));

    // A click on its row opens a button up, and one more closes it; open,
    // it stays open through a move.
    let at = row(&h, 3, "pick").unwrap().center();
    click(&mut h, &mut shell, &mut app, at);
    assert_eq!(app.radial.selected, Some(3));
    let at = row(&h, 3, "earlier").unwrap().center();
    click(&mut h, &mut shell, &mut app, at);
    assert_eq!((slots(&app)[2].label.as_str(), app.radial.selected), ("Screenshot", Some(2)));
    let at = row(&h, 2, "pick").unwrap().center();
    click(&mut h, &mut shell, &mut app, at);
    assert_eq!(app.radial.selected, None);

    // A click on the little ring opens one too; its name takes typing.
    let on_ring = on_page(&h, |p| p.with("ring").with("ring").with("button").with_index(3)).unwrap();
    click(&mut h, &mut shell, &mut app, on_ring.center());
    assert_eq!(app.radial.selected, Some(3));
    let fields = app.radial.generation.to_string();
    let name = on_page(&h, |p| p.with("buttons").with_index(3).with(&fields).with("Name")).expect("the name field");
    click(&mut h, &mut shell, &mut app, name.center());
    h.type_text("!");
    h.shell_settle(&mut shell, &mut app, 8);
    let renamed = slots(&app)[3].label.clone();
    assert!(renamed.contains('!') && renamed.replace('!', "") == "Notepad", "{renamed}");

    // Nothing is on disk until the hand has rested.
    assert!(!file.exists());
    h.advance(1.0);
    h.shell_settle(&mut shell, &mut app, 8);
    assert_eq!(radial::Ring::load().slots, slots(&app));

    // The search narrows to what is typed, and Enter takes the first.
    let search = on_page(&h, |p| p.with("buttons").with("add")).expect("the search");
    click(&mut h, &mut shell, &mut app, search.center());
    h.type_text("refresh desk");
    h.shell_settle(&mut shell, &mut app, 8);
    h.key(Key::Enter);
    h.shell_settle(&mut shell, &mut app, 8);
    let after = slots(&app);
    assert_eq!((after.len(), after[5].label.as_str(), after[5].action, app.radial.selected), (6, "Refresh Desktop", Action::Refresh, Some(5)));
    assert!(app.radial.query.is_empty());
    h.advance(1.0);
    h.shell_settle(&mut shell, &mut app, 8);
    assert_eq!(radial::Ring::load().slots, after);
    let _ = std::fs::remove_file(&file);
}

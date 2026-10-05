//! The right-click menu, row by row, and the commands behind every menu.

use lntrn_math::Vec2;
use lntrn_ui::{Host, Key, Modifiers, WindowCommand};

use super::{ROWS, Rig, sandbox, turn};
use crate::app::App;
use crate::menu;
use crate::settings::Settings;

#[test]
fn the_right_click_menu_does_what_its_rows_say() {
    let mut rig = Rig::new();
    rig.line("echo menu-$((4*4))");
    rig.sees("menu-16");

    // Every row is there, big enough to hit, inside the window.
    rig.right_click(0);
    let window = rig.h.window();
    for (i, key) in ROWS {
        let r = rig.menu_rect(Rig::in_row(i, key));
        // (Full width: a menu too tall for the window gives some of it to
        // a scroll bar.)
        assert!(r.height() >= 44.0 && r.width() >= 410.0, "{key}: {r:?}");
        assert!(r.min.x >= window.min.x && r.max.x <= window.max.x && r.min.y >= window.min.y && r.max.y <= window.max.y, "{key} is inside the window: {r:?}");
    }
    // The little title bar is the first thing in the menu: nothing over
    // it but the panel's edge.
    let (panel, name) = (rig.shell.state.popup.expect("the menu").0, rig.menu_rect(Rig::in_row(0, "lntrn")));
    assert!((name.min.y - panel.min.y - rig.h.metrics().pad).abs() <= 1.0, "{name:?} in {panel:?}");
    assert!(panel.height() <= 620.0, "room to spare in a window 720 tall: {panel:?}");
    // Nothing selected: Copy is dim, and a click on it leaves the menu up.
    let copy = rig.menu_rect(Rig::in_row(4, "edit.copy"));
    rig.click_at(copy.center());
    assert!(rig.shell.popup_open());
    // Select All closes the menu; then Copy has something to copy.
    let all = rig.menu_rect(Rig::in_row(6, "edit.select-all"));
    rig.click_at(all.center());
    assert!(!rig.shell.popup_open());
    assert!(rig.front().borrow().selection.is_some());
    rig.choose(0, "edit.copy");
    assert!(rig.shell.state.clipboard.contains("menu-16"), "{:?}", rig.shell.state.clipboard);
    // Paste types the clipboard, and the terminal has the keyboard again
    // once the menu has gone.
    rig.shell.state.clipboard = "echo pasted-$((2+3))".into();
    rig.choose(0, "edit.paste");
    rig.h.key(Key::Enter);
    rig.sees("pasted-5");

    // The text size follows the slider as it is dragged, and is written
    // to lantern.toml once the hand has stopped.
    rig.right_click(0);
    let slider = rig.menu_rect(Rig::in_row(2, "size"));
    assert!(slider.width() > 150.0 && slider.height() >= 44.0, "room to drag: {slider:?}");
    rig.click_at(Vec2::new(slider.min.x + slider.width() * 0.75, slider.center().y));
    let size = rig.app.settings.font_size;
    assert!((28.0..36.0).contains(&size), "three quarters along 8 to 40: {size}");
    assert_eq!(rig.app.look.font_size, size);
    assert!(rig.shell.popup_open(), "the menu stays up under the slider");
    rig.wait(0.6);
    assert_eq!(Settings::reload().map(|s| s.font_size), Some(size));
    rig.h.key(Key::Escape);
    rig.settle();

    // New Tab; then the little title bar has the tabs in it.
    rig.choose(0, "tab.new");
    assert_eq!(rig.tabs_of(0), (2, 1));
    rig.right_click(0);
    let dot = rig.menu_rect(Rig::in_row(0, "dot").with_index(0));
    rig.click_at(dot.center());
    assert_eq!(rig.tabs_of(0), (2, 0), "a dot shows its tab");
    assert!(rig.shell.popup_open(), "and the menu stays for the next one");
    let next = rig.menu_rect(Rig::in_row(0, "next"));
    rig.click_at(next.center());
    assert_eq!(rig.tabs_of(0), (2, 1));
    rig.h.key(Key::Escape);
    rig.settle();
    rig.choose(0, "tab.close");
    assert_eq!(rig.counts(), (1, 1));

    // Close Pane waits for a second pane.
    rig.choose(0, "pane.close");
    assert!(rig.shell.popup_open(), "one pane: nothing to close");
    rig.h.key(Key::Escape);
    rig.settle();
    rig.choose(0, "split.down");
    assert_eq!(rig.counts(), (2, 2));
    let [top, bottom] = rig.areas()[..] else { panic!("two panes") };
    assert!(rig.term_rect(top).max.y <= rig.term_rect(bottom).min.y);
    rig.choose(bottom, "pane.close");
    assert_eq!(rig.counts(), (1, 1));
    rig.choose(0, "split.right");
    assert_eq!(rig.counts(), (2, 2));
    let right = rig.areas()[1];
    rig.choose(right, "pane.close");
    assert_eq!(rig.areas(), [0], "the pane that was clicked is the one that goes");

    // `lntrn` is typed into the terminal with Enter after it (into a
    // `cat` here, which says it back, so nothing of that name runs).
    rig.line("cat");
    rig.right_click(0);
    let name = rig.menu_rect(Rig::in_row(0, "lntrn"));
    rig.click_at(name.center());
    rig.until("lntrn to be typed and said back", |r| r.screen().matches("lntrn").count() >= 2);
    rig.h.key_with(Key::Char('c'), Modifiers::CTRL);
    rig.settle();

    // The window's buttons, for when the title bar is hidden.
    for (key, does) in [("min", WindowCommand::Minimize), ("max", WindowCommand::ToggleMaximize), ("close", WindowCommand::Close)] {
        rig.command = None;
        rig.right_click(0);
        let button = rig.menu_rect(Rig::in_row(0, key));
        assert!(button.width() >= 44.0 && button.height() >= 44.0, "{key}: {button:?}");
        rig.click_at(button.center());
        assert_eq!(rig.command, Some(does), "{key}");
        assert!(!rig.shell.popup_open());
    }
}

#[test]
fn every_command_and_menu_row_is_one_the_app_knows() {
    let _turn = turn();
    sandbox();
    let app = App::new();
    let known: Vec<&str> = menu::COMMANDS.iter().map(|(id, _)| *id).collect();
    for name in ["terminal", "view", "split"] {
        let listed = app.menu(name).unwrap_or_else(|| panic!("the {name} menu"));
        let rows = listed.items.iter().flat_map(|i| std::iter::once(i).chain(i.sub.iter())).filter(|i| !i.separator && i.sub.is_empty());
        for row in rows {
            assert!(row.action.id.starts_with("shell.") || known.contains(&row.action.id.as_str()), "{name}: {:?} runs {:?}", row.label, row.action.id);
        }
    }
    for (_, key) in ROWS {
        assert!(known.contains(&key), "{key}");
    }
    assert_eq!(app.palette("split").len(), 2);
    // Each key the menus promise is one the keymap has.
    for (id, key) in [("tab.new", "Ctrl+Shift+T"), ("tab.close", "Ctrl+Shift+W"), ("split.right", "Ctrl+Shift+D"), ("split.down", "Ctrl+Shift+E"), ("window.new", "Ctrl+Shift+N")] {
        assert_eq!(app.key_hint(&lntrn_ui::Action::new(id)).as_deref(), Some(key));
    }
}

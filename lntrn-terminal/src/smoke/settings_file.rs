//! The settings: read from `lantern.toml`, written back to it, followed
//! while running, carried across from the old file, and the tabs pinned
//! in them.

use std::time::Duration;

use lntrn_data::Doc;
use lntrn_kit::desktop;
use lntrn_math::Color;
use lntrn_term::grid::CursorShape;
use lntrn_ui::{Host, Key, Modifiers};

use super::{Rig, both, sandbox, turn};
use crate::app::Editor;
use crate::settings::{Pinned, Settings};

#[test]
fn settings_are_read_written_and_followed() {
    let config = "[appearance]\naccent = \"#2563EB\"\ntheme = \"fox\"\n\n[terminal]\nfont_size = 26.0\ncursor_style = \"beam\"\nopen_bar_hidden = true\n\n[[monitors]]\nname = \"eDP-1\"\nscale = 1.5\n";
    let mut rig = Rig::start(Some(config), None);
    assert_eq!((rig.app.look.font_size, rig.app.look.cursor, rig.app.look.background), (26.0, CursorShape::Beam, Color::hex(0x181818)));
    assert_eq!(rig.shell.prefs.theme.accent, Color::hex(0x2563EB));
    // Opened with the bar hidden: the terminal is the whole window.
    assert!(rig.app.bar_hidden && !rig.shell.title_bar);
    let term = rig.term_rect(0);
    assert!(term.min.y < 12.0 && term.height() > 690.0, "{term:?}");
    rig.line("echo bare-$((7*7))");
    rig.sees("bare-49");
    // Super+F11 brings the bar back, for this run.
    rig.h.key_with(Key::F(11), Modifiers::SUPER);
    rig.settle();
    assert!(rig.shell.title_bar && rig.term_rect(0).min.y > 30.0);

    // Bigger text from the keyboard: shown at once, written in a moment,
    // and the rest of the file is as it was.
    rig.chord('+');
    assert_eq!((rig.app.settings.font_size, rig.app.look.font_size), (27.0, 27.0));
    rig.wait(0.6);
    let doc = desktop::read_doc();
    assert_eq!(doc.path("terminal.font_size").and_then(Doc::as_f64), Some(27.0));
    assert_eq!(doc.path("terminal.cursor_style").and_then(Doc::as_str), Some("beam"));
    assert_eq!(doc.path("terminal.open_bar_hidden").and_then(Doc::as_bool), Some(true));
    assert_eq!(doc.path("appearance.accent").and_then(Doc::as_str), Some("#2563EB"));
    assert_eq!(doc.get("monitors").and_then(Doc::as_list).and_then(|l| l[0].get("scale")).and_then(Doc::as_f64), Some(1.5));
    rig.act("cursor.underline");
    rig.wait(0.6);
    assert_eq!(Settings::reload().map(|s| s.cursor), Some(CursorShape::Underline));

    // A change made in System Settings is taken up while running.
    rig.wait(1.2);
    std::thread::sleep(Duration::from_millis(20));
    desktop::update("terminal", |t| {
        t.insert("font_size", Doc::Float(14.0));
        t.insert("cursor_style", Doc::Str("block".into()));
    })
    .unwrap();
    rig.wait(1.2);
    assert_eq!((rig.app.settings.font_size, rig.app.look.font_size, rig.app.look.cursor), (14.0, 14.0, CursorShape::Block));
}

#[test]
fn the_old_settings_file_is_carried_across_once() {
    let _turn = turn();
    sandbox();
    let path = desktop::config_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[appearance]\naccent = \"#FFC800\"\n").unwrap();
    let old = path.with_file_name("terminal.toml");
    std::fs::write(&old, "[font]\nfamily = \"monospace\"\nsize = 20.5\n\n[general]\ncursor_style = \"underline\"\nopen_chrome_hidden = false\n\n[[pinned_tabs]]\nname = \"Notes\"\ncwd = \"/srv/notes\"\n").unwrap();
    let carried = Settings::load();
    assert_eq!(carried, Settings { font_size: 20.5, cursor: CursorShape::Underline, open_bar_hidden: false, pinned: vec![Pinned { name: "Notes".into(), cwd: "/srv/notes".into() }] });
    assert_eq!(Settings::reload(), Some(carried.clone()), "written into lantern.toml");
    assert_eq!(desktop::read_doc().path("appearance.accent").and_then(Doc::as_str), Some("#FFC800"));
    // With a section of its own there, the old file is not looked at.
    std::fs::write(&old, "[font]\nsize = 9.0\n").unwrap();
    assert_eq!(Settings::load(), carried);
    let _ = std::fs::remove_file(old);
}

#[test]
fn a_pinned_tab_comes_back_the_next_time() {
    let work = sandbox().join("work").canonicalize().unwrap();
    {
        let mut rig = Rig::new();
        rig.line(&format!("cd {}", work.display()));
        rig.until("the shell to be in the folder", |r| r.front().borrow().cwd_now().as_deref() == Some(work.as_path()));
        rig.act("tab.pin");
        assert!(rig.app.front_pinned);
        assert_eq!(rig.app.settings.pinned, [Pinned { name: String::new(), cwd: work.display().to_string() }]);
        assert_eq!(Settings::reload().map(|s| s.pinned), Some(rig.app.settings.pinned.clone()));
    }
    // The next run: the pinned tab first, a fresh one in front of it.
    let mut rig = Rig::start(None, None);
    assert_eq!((rig.counts(), rig.tabs_of(0)), ((1, 2), (2, 1)));
    assert!(rig.app.shows_header(Editor::Terminal));
    assert!(!rig.app.front_pinned);
    rig.h.key_with(Key::Tab, both());
    rig.settle();
    assert_eq!(rig.tabs_of(0), (2, 0));
    rig.until("the pinned tab's shell to be in its folder", |r| r.front().borrow().cwd_now().as_deref() == Some(work.as_path()));
    assert!(rig.app.front_pinned);
    // Unpinned, it is forgotten.
    rig.act("tab.pin");
    assert!(!rig.app.front_pinned && rig.app.settings.pinned.is_empty());
    assert_eq!(Settings::reload().map(|s| s.pinned), Some(Vec::new()));
}

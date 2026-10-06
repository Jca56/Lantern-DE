//! Files, tabs, and what is kept between runs.

use lntrn_ui::{Key, Modifiers};

use super::{Rig, config_file, sandbox};
use crate::doc::{Doc, Pos, Style};
use crate::io;
use crate::session;
use crate::settings::Settings;

#[test]
fn documents_are_saved_opened_and_closed_without_losing_work() {
    // A folder of this test's own: the others have theirs beside it.
    let docs = sandbox().join("docs/saved");
    let mut rig = Rig::start_after(false, &[], || std::fs::create_dir_all(sandbox().join("docs/saved")).unwrap());
    rig.type_text("# ");
    rig.type_text("Shopping");
    rig.key(Key::Enter, Modifiers::NONE);
    rig.type_text("milk");
    let front = rig.front();
    // Ctrl+S on a document with no file asks where, starting from its
    // first words in the documents folder.
    rig.chord('s', false);
    assert!(rig.shell.popup_open(), "a file dialog");
    rig.key(Key::Escape, Modifiers::NONE);
    let note = docs.join("Shopping.lnote");
    rig.act("file.save-path", Some(&docs.join("Shopping")));
    assert!(note.exists(), "saved with our extension when none was given");
    assert_eq!((front.borrow().ed.modified(), rig.tabs().0), (false, vec!["Shopping.lnote".to_owned()]));
    assert_eq!(io::load(&note).unwrap().paras, front.borrow().ed.doc.paras);
    // Ctrl+S after that just saves.
    rig.type_text(" and eggs");
    assert_eq!(rig.tabs().0, ["Shopping.lnote \u{25CF}"]);
    rig.chord('s', false);
    assert_eq!((rig.shell.popup_open(), io::load(&note).unwrap().text()), (false, "Shopping\nmilk and eggs".to_owned()));
    // Saved as text, a document with a heading asks first; saved
    // anyway, the words are there and the heading is not.
    let text = docs.join("list.txt");
    rig.act("file.save-as", None);
    rig.key(Key::Escape, Modifiers::NONE);
    rig.act("file.save-path", Some(&text));
    assert!(rig.shell.popup_open() && !text.exists(), "asked before anything is lost");
    rig.act("file.save-lossy", None);
    assert_eq!(std::fs::read_to_string(&text).unwrap(), "Shopping\nmilk and eggs\n");
    assert_eq!(rig.tabs().0, ["list.txt"]);
    // Opening: a file in a new tab; the same file again is the same
    // tab; one that isn't there says so and opens nothing.
    rig.act("file.open-path", Some(&note));
    assert_eq!(rig.tabs(), (vec!["list.txt".to_owned(), "Shopping.lnote".to_owned()], 1));
    assert_eq!(rig.front().borrow().ed.doc.para(0).attrs.style, Style::Title);
    rig.act("file.open-path", Some(&text));
    rig.act("file.open-path", Some(&note));
    assert_eq!(rig.tabs().1, 1);
    rig.act("file.open-path", Some(&docs.join("nowhere.lnote")));
    assert!(rig.shell.popup_open() && rig.tabs().0.len() == 2);
    rig.key(Key::Escape, Modifiers::NONE);
    // Closing a tab with nothing unsaved just closes it. With unsaved
    // work it asks; Discard closes, Save saves first.
    rig.chord('w', false);
    assert_eq!(rig.tabs().0, ["list.txt"]);
    rig.type_text("!");
    rig.chord('w', false);
    assert!(rig.shell.popup_open() && rig.tabs().0.len() == 1);
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!((rig.shell.popup_open(), std::fs::read_to_string(&text).unwrap().contains('!')), (false, true), "Enter is Save");
    assert_eq!(rig.tabs().0, ["Untitled"], "the last tab leaves an empty page behind");
    rig.type_text("scrap");
    rig.chord('w', false);
    rig.act("tab.close-discard", None);
    assert_eq!((rig.text(), rig.tabs().0.len()), (String::new(), 1));
    // A tab closed from the strip itself is not asked about by the
    // shell: one with unsaved work comes back, with the question.
    rig.chord('n', false);
    rig.type_text("precious");
    let (area, tab) = (rig.app.front_area.unwrap(), rig.tabs().1);
    rig.shell.screen.close_tab_at(area, tab);
    rig.settle();
    assert!(rig.shell.popup_open() && rig.tabs().0.iter().any(|name| name.starts_with("Untitled")), "{:?}", rig.tabs());
    assert_eq!(rig.front().borrow().ed.doc.text(), "precious");
    // A file dropped on the window opens; a picture dropped goes in.
    rig.key(Key::Escape, Modifiers::NONE);
    let picture = docs.join("dot.png");
    std::fs::write(&picture, lntrn_image::encode_png(&lntrn_image::Image::solid(4, 4, [1, 2, 3, 255]))).unwrap();
    let mut requests = Vec::new();
    lntrn_ui::Host::dropped(&mut rig.app, &[picture, note.clone()], None, None, &mut lntrn_ui::HostCx { pointer: lntrn_math::Vec2::ZERO, requests: &mut requests });
    rig.settle();
    assert!(rig.tabs().0.contains(&"Shopping.lnote".to_owned()));
    let _ = std::fs::remove_dir_all(&docs);
}

#[test]
fn unsaved_work_and_open_tabs_come_back_with_the_next_window() {
    let saved = sandbox().join("docs/kept.lnote");
    {
        let mut rig = Rig::start_after(true, &[], || {
            let _ = std::fs::remove_dir_all(session::dir());
            io::save(&sandbox().join("docs/kept.lnote"), &Doc::from_text("on disk"), 24.0).unwrap();
        });
        assert!(rig.app.primary);
        // A note never saved, a file with changes, and a file as it is.
        rig.type_text("a thought");
        rig.act("file.open-path", Some(&saved));
        rig.chord('n', false);
        rig.type_text("another");
        rig.act("file.open-path", Some(&saved));
        rig.key(Key::End, Modifiers::CTRL);
        rig.type_text(" and more");
        assert_eq!(rig.tabs().0, ["Untitled \u{25CF}", "kept.lnote \u{25CF}", "Untitled 2 \u{25CF}"]);
        // Nothing is written while the typing goes on; a pause, and the
        // drafts are there.
        rig.app.saver.flush();
        assert!(std::fs::read_dir(session::dir().join("drafts")).map_or(true, |d| d.count() == 0));
        rig.wait(1.6);
        rig.app.saver.flush();
        assert_eq!(std::fs::read_dir(session::dir().join("drafts")).unwrap().count(), 3);
        assert!(!std::fs::read_to_string(&saved).unwrap().contains("and more"), "the file itself is not touched until it is saved");
        // The window closes without a question.
        let mut requests = Vec::new();
        assert!(lntrn_ui::Host::close_requested(&mut rig.app, true, &mut lntrn_ui::HostCx { pointer: lntrn_math::Vec2::ZERO, requests: &mut requests }));
        assert!(requests.is_empty());
    }
    {
        // The next one has all three, as they were left, still unsaved,
        // the one that was in front in front.
        let mut rig = Rig::start(true, &[]);
        assert_eq!(rig.tabs(), (vec!["Untitled \u{25CF}".to_owned(), "kept.lnote \u{25CF}".to_owned(), "Untitled 2 \u{25CF}".to_owned()], 1));
        assert_eq!(rig.text(), "on disk and more");
        rig.chord('s', false);
        assert!(std::fs::read_to_string(&saved).unwrap().contains("and more"));
        // Saved, its draft goes; a note thrown away takes its draft too.
        rig.key(Key::Tab, Modifiers::CTRL);
        assert_eq!(rig.text(), "another");
        rig.chord('w', false);
        rig.act("tab.close-discard", None);
        rig.wait(1.6);
        rig.app.saver.flush();
        assert_eq!(std::fs::read_dir(session::dir().join("drafts")).unwrap().count(), 1);
        rig.app.remember_all();
    }
    {
        // Started for a file, only unsaved work comes back with it.
        let other = sandbox().join("docs/other.txt");
        std::fs::write(&other, "asked for").unwrap();
        let mut rig = Rig::start(true, &[other]);
        assert_eq!(rig.tabs().0, ["Untitled \u{25CF}", "other.txt"]);
        assert_eq!(rig.text(), "asked for");
        // A draft nobody remembers (a crash before the list was written)
        // comes back as well, next time.
        std::fs::write(session::dir().join("drafts/00000000000000aa.999999999.lnote"), io::lnote::write(&Doc::from_text("orphan"))).unwrap();
        rig.app.remember_all();
    }
    let mut rig = Rig::start(true, &[]);
    let names = rig.tabs().0;
    assert!(names.contains(&"Recovered \u{25CF}".to_owned()) && names.contains(&"other.txt".to_owned()), "{names:?}");
    rig.app.remembers = false;
    let _ = std::fs::remove_dir_all(session::dir());
}

#[test]
fn settings_are_kept_and_followed() {
    let mut rig = Rig::start_after(false, &[], || {
        // The old file is carried across the first time.
        let config = config_file();
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&config);
        std::fs::write(config.with_file_name("notepad.toml"), "theme = \"dark\"\npage_width = 0.5\n").unwrap();
        assert_eq!(Settings::load(), Settings { paper: false, page_width: 0.5 });
        assert_eq!(Settings::reload(), Some(Settings { paper: false, page_width: 0.5 }));
    });
    assert_eq!((rig.app.settings.paper, rig.app.inks().page), (false, 0x1D1914));
    // The page's look from the View menu: shown at once, written in a
    // moment, the rest of the file as it was.
    lntrn_kit::desktop::update("appearance", |t| t.insert("accent", lntrn_data::Doc::Str("#2563EB".into()))).unwrap();
    rig.act("view.paper", None);
    assert_eq!(rig.app.inks().page, 0xFCFBF9);
    rig.wait(0.6);
    let doc = lntrn_kit::desktop::read_doc();
    assert_eq!((doc.path("notepad.theme").and_then(lntrn_data::Doc::as_str), doc.path("appearance.accent").and_then(lntrn_data::Doc::as_str)), (Some("paper"), Some("#2563EB")));
    // A change made in System Settings is taken up while running.
    rig.wait(1.2);
    std::thread::sleep(std::time::Duration::from_millis(20));
    lntrn_kit::desktop::update("notepad", |t| t.insert("page_width", lntrn_data::Doc::Float(0.1))).unwrap();
    rig.wait(1.2);
    rig.wait(0.1);
    assert_eq!(rig.app.settings.page_width, 0.1);
    let config = config_file();
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(config.with_file_name("notepad.toml"));
    assert_eq!(rig.front().borrow().ed.caret, Pos::default());
}

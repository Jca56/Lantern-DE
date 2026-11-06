//! Spelling in a running Notepad: a small dictionary in the sandbox,
//! words it doesn't know found as they are typed, and the right-click
//! menu's fixes put in.

use lntrn_props::Value;
use lntrn_ui::Action;

use super::{Rig, sandbox};
use crate::doc::Pos;

/// A dictionary of a few words where Notepad looks for US English.
fn dictionary() {
    let dir = lntrn_spell::Speller::dictionaries().unwrap();
    assert!(dir.starts_with(sandbox()), "{} is not in the sandbox", dir.display());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("en_US.aff"), "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n\nSFX S Y 1\nSFX S 0 s .\n").unwrap();
    std::fs::write(dir.join("en_US.dic"), "4\nthe\ncat/S\nsat\non\n").unwrap();
    let _ = std::fs::remove_file(lntrn_spell::Speller::personal_path().unwrap());
}

fn with_word(id: &str, word: &str) -> Action {
    Action::new(id).with("word", Value::Str(word.to_owned()))
}

#[test]
fn misspellings_are_found_and_put_right_from_the_menu() {
    let mut rig = Rig::start_after(false, &[], dictionary);
    for _ in 0..400 {
        if !rig.app.spelling.loading() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        rig.frame();
    }
    assert_eq!(rig.app.spelling.missing(), None, "the dictionary loaded");
    rig.type_text("the cta sat on teh mat ");
    let misspelled = |rig: &mut Rig| {
        let text = rig.text();
        rig.app.spelling.misspelled(&text).iter().map(|r| text[r.clone()].to_owned()).collect::<Vec<_>>()
    };
    assert_eq!(misspelled(&mut rig), ["cta", "teh", "mat"]);

    // Right-clicked in the middle of a word: what it may have meant.
    rig.front().borrow_mut().ed.move_to(Pos::new(0, 5), false);
    let front = rig.front();
    let (word, suggestions) = rig.app.misspelling(&front.borrow().ed).expect("a misspelled word at the caret");
    assert_eq!((word.as_str(), suggestions.first().map(String::as_str)), ("cta", Some("cat")));
    rig.run(with_word("spell.use", "cat"));
    assert_eq!(rig.text(), "the cat sat on teh mat ");

    // Ignored for now; added for good, in every app.
    rig.run(with_word("spell.ignore", "teh"));
    rig.run(with_word("spell.add", "mat"));
    assert_eq!(misspelled(&mut rig), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(lntrn_spell::Speller::personal_path().unwrap()).unwrap(), "mat\n");

    // Spelling can be turned off from the View menu.
    rig.act("view.spelling", None);
    assert!(!rig.app.settings.spelling);
    rig.front().borrow_mut().ed.move_to(Pos::new(0, 1), false);
    let front = rig.front();
    assert!(rig.app.misspelling(&front.borrow().ed).is_none());
}

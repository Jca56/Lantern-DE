//! A Save name that is taken is asked about before it is returned; a
//! picker for one item returns one; a path the caller's framing cannot
//! carry is not returned.

use super::*;
use crate::fs::FileEntry;
use crate::{PickConfig, PickType};

/// A folder of its own per test (they run in parallel).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fox-pick-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn picker(mode: PickType, multiple: bool, dir: &Path) -> App {
    let mut app = App::new();
    app.pick = Some(PickConfig {
        mode,
        multiple,
        title: None,
        start_dir: None,
        filters: Vec::new(),
        active_filter: 0,
        save_name: None,
        print0: false,
    });
    app.current_dir = dir.to_path_buf();
    app
}

fn entry(path: PathBuf, selected: bool) -> FileEntry {
    FileEntry {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        path,
        is_dir: false,
        size: 0,
        modified: None,
        is_symlink: false,
        selected,
        folder_icon: None,
        folder_color: None,
    }
}

fn result(app: &App) -> Option<Vec<PathBuf>> {
    match &app.pick_result {
        Some(PickResult::Selected(paths)) => Some(paths.clone()),
        _ => None,
    }
}

fn front_pick_dialog(app: &App) -> Option<&PickDialog> {
    match app.op_dialogs.front() {
        Some(OpDialog::Pick(dialog)) => Some(dialog),
        _ => None,
    }
}

#[test]
fn the_disk_says_what_a_save_name_points_at() {
    let dir = scratch("look");
    std::fs::write(dir.join("taken.txt"), b"x").unwrap();
    std::fs::create_dir(dir.join("folder")).unwrap();
    std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("dangling")).unwrap();
    std::os::unix::fs::symlink(dir.join("folder"), dir.join("to-folder")).unwrap();

    assert_eq!(look(&dir.join("new.txt")), SaveTarget::Free);
    assert_eq!(look(&dir.join("taken.txt")), SaveTarget::File);
    assert_eq!(look(&dir.join("folder")), SaveTarget::Folder);
    assert_eq!(look(&dir.join("to-folder")), SaveTarget::Folder);
    // A write would go through the link and create its target: not free.
    assert_eq!(look(&dir.join("dangling")), SaveTarget::File);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_free_save_name_is_returned_at_once() {
    let dir = scratch("free");
    let mut app = picker(PickType::Save, false, &dir);
    app.save_name_buf = "Untitled.lnote".into();
    app.confirm_pick();
    assert_eq!(result(&app), Some(vec![dir.join("Untitled.lnote")]));
    assert!(app.op_dialogs.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_taken_save_name_is_asked_about_and_enter_does_not_replace() {
    let dir = scratch("taken");
    std::fs::write(dir.join("Untitled.lnote"), b"the first note").unwrap();
    let mut app = picker(PickType::Save, false, &dir);
    app.save_name_buf = "Untitled.lnote".into();

    // Enter in the name field, or the Save button: a question, no result.
    app.confirm_pick();
    assert_eq!(result(&app), None);
    let dialog = front_pick_dialog(&app).expect("the Replace question");
    assert_eq!(dialog.title, "Replace \u{201C}Untitled.lnote\u{201D}?");
    assert_eq!(dialog.act_label(), Some("Replace"));
    assert!(dialog.lines.iter().any(|l| l.contains("cannot be undone")));

    // Enter again (a habitual double tap) is Cancel: back in the picker.
    app.op_dialog_enter();
    assert!(app.op_dialogs.is_empty());
    assert_eq!(result(&app), None);

    // Esc and a click outside are Cancel too.
    app.confirm_pick();
    app.op_dialog_choose(false);
    assert!(app.op_dialogs.is_empty());
    assert_eq!(result(&app), None);

    // Only the Replace button returns the path, and the question was asked
    // exactly once for it.
    app.confirm_pick();
    assert_eq!(app.op_dialogs.len(), 1);
    app.op_dialog_choose(true);
    assert!(app.op_dialogs.is_empty());
    assert_eq!(result(&app), Some(vec![dir.join("Untitled.lnote")]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_save_name_that_is_a_folder_goes_into_it() {
    let dir = scratch("folder");
    std::fs::create_dir(dir.join("Notes")).unwrap();
    let mut app = picker(PickType::Save, false, &dir);
    app.save_name_buf = "Notes".into();
    app.confirm_pick();
    assert_eq!(result(&app), None);
    assert!(app.op_dialogs.is_empty());
    assert_eq!(app.current_dir, dir.join("Notes"));
    // The name is selected: typing the file's name replaces it.
    assert_eq!(app.save_name_selection, Some((0, "Notes".len())));
    assert!(app.save_name_editing);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_save_name_is_checked_as_the_caller_will_read_it() {
    // Callers that read lines trim them: "report " is written as "report".
    let dir = scratch("trim");
    std::fs::write(dir.join("report"), b"x").unwrap();
    let mut app = picker(PickType::Save, false, &dir);
    app.save_name_buf = "report ".into();
    app.confirm_pick();
    assert_eq!(result(&app), None);
    assert!(front_pick_dialog(&app).is_some(), "the existing file is asked about");
    app.op_dialog_choose(true);
    assert_eq!(result(&app), Some(vec![dir.join("report")]));

    // With the NUL framing the name is taken as typed.
    let mut app = picker(PickType::Save, false, &dir);
    app.pick.as_mut().unwrap().print0 = true;
    app.save_name_buf = "report ".into();
    app.confirm_pick();
    assert_eq!(result(&app), Some(vec![dir.join("report ")]));

    // Nothing but spaces is no name.
    let mut app = picker(PickType::Save, false, &dir);
    app.save_name_buf = "   ".into();
    app.confirm_pick();
    assert_eq!(result(&app), None);
    assert!(app.op_dialogs.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_answer_of_a_slow_device_is_used_only_while_it_is_waited_for() {
    let dir = scratch("slow");
    let path = dir.join("a.txt");
    let mut app = picker(PickType::Save, false, &dir);

    // The device says the name is taken: the wait becomes the question.
    app.save_check = Some((path.clone(), Task::spawn("fox-test", || SaveTarget::File)));
    app.op_dialogs
        .push_back(OpDialog::Pick(PickDialog::new(Kind::Checking, path.clone())));
    assert_eq!(front_pick_dialog(&app).unwrap().act_label(), None);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !app.poll_save_check() {
        assert!(std::time::Instant::now() < deadline, "the answer never came");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(result(&app), None);
    assert_eq!(front_pick_dialog(&app).unwrap().act_label(), Some("Replace"));
    app.op_dialog_choose(false);

    // Cancelled while waiting: the answer, when it comes, does nothing.
    app.save_check = Some((path.clone(), Task::spawn("fox-test", || SaveTarget::Free)));
    app.op_dialogs
        .push_back(OpDialog::Pick(PickDialog::new(Kind::Checking, path.clone())));
    app.op_dialog_enter();
    assert!(app.save_check.is_none() && app.op_dialogs.is_empty());
    assert!(!app.poll_save_check());
    assert_eq!(result(&app), None);

    // A worker that died: the user is asked instead of nothing happening.
    app.save_check = Some((path.clone(), Task::spawn("fox-test", || panic!("worker died"))));
    app.op_dialogs
        .push_back(OpDialog::Pick(PickDialog::new(Kind::Checking, path.clone())));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !app.poll_save_check() {
        assert!(std::time::Instant::now() < deadline, "the loss was never noticed");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(front_pick_dialog(&app).unwrap().act_label(), Some("Save Anyway"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_picker_for_one_item_never_returns_two() {
    let dir = scratch("single");
    let mut app = picker(PickType::Open, false, &dir);
    app.entries = vec![
        entry(dir.join("a.txt"), true),
        entry(dir.join("b.txt"), true),
        entry(dir.join("c.txt"), false),
    ];
    // Two selected (however that came about): refused, with a reason.
    app.confirm_pick();
    assert_eq!(result(&app), None);
    assert!(matches!(app.op_dialogs.front(), Some(OpDialog::Notice { .. })));
    app.op_dialog_choose(false);

    // The ways to select several select one.
    app.select_all();
    assert_eq!(app.entries.iter().filter(|e| e.selected).count(), 2, "Ctrl+A changes nothing");
    app.select_range(0, 2);
    let selected: Vec<bool> = app.entries.iter().map(|e| e.selected).collect();
    assert_eq!(selected, [false, false, true], "a Shift+click selects the item clicked");
    // A mark on a nested Tree row goes when another item is selected.
    app.pick_tree_selection.insert(dir.join("sub/nested.txt"));
    app.select_item(0);
    assert!(app.pick_tree_selection.is_empty());
    app.pick_tree_selection.insert(dir.join("sub/nested.txt"));
    app.select_only(1);
    assert!(app.pick_tree_selection.is_empty());

    app.confirm_pick();
    assert_eq!(result(&app), Some(vec![dir.join("b.txt")]));

    // With --pick-multiple the same selection comes back whole.
    let mut app = picker(PickType::Open, true, &dir);
    app.entries = vec![entry(dir.join("a.txt"), true), entry(dir.join("b.txt"), true)];
    app.select_all();
    app.confirm_pick();
    assert_eq!(result(&app), Some(vec![dir.join("a.txt"), dir.join("b.txt")]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_ctrl_click_in_a_picker_for_one_item_keeps_only_its_row() {
    let dir = scratch("ctrl");
    let nested = dir.join("sub/nested.txt");
    let mut app = picker(PickType::Open, false, &dir);
    app.entries = vec![entry(dir.join("a.txt"), true), entry(dir.join("b.txt"), false)];
    app.pick_tree_selection.insert(nested.clone());

    // On b: a and the nested row let go (the click itself then toggles b).
    app.unselect_others(&dir.join("b.txt"));
    assert!(app.entries.iter().all(|e| !e.selected));
    assert!(app.pick_tree_selection.is_empty());

    // On a nested row that is marked: it stays, for the toggle to clear.
    app.entries[0].selected = true;
    app.pick_tree_selection.insert(nested.clone());
    app.unselect_others(&nested);
    assert!(app.entries.iter().all(|e| !e.selected));
    assert!(app.pick_tree_selection.contains(&nested));

    // With --pick-multiple (and outside a picker) a Ctrl+click adds.
    let mut app = picker(PickType::Open, true, &dir);
    app.entries = vec![entry(dir.join("a.txt"), true), entry(dir.join("b.txt"), false)];
    app.unselect_others(&dir.join("b.txt"));
    assert!(app.entries[0].selected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_path_with_a_line_break_is_only_returned_through_the_nul_framing() {
    let dir = scratch("newline");
    let odd = dir.join("two\nlines.txt");
    let mut app = picker(PickType::Open, false, &dir);
    app.entries = vec![entry(odd.clone(), true)];
    app.confirm_pick();
    assert_eq!(result(&app), None);
    assert!(matches!(app.op_dialogs.front(), Some(OpDialog::Notice { .. })));

    let mut app = picker(PickType::Open, false, &dir);
    app.pick.as_mut().unwrap().print0 = true;
    app.entries = vec![entry(odd.clone(), true)];
    app.confirm_pick();
    assert_eq!(result(&app), Some(vec![odd]));

    // The folder picker's fallback, the folder shown, is checked as well.
    let inside = dir.join("bad\nfolder");
    let mut app = picker(PickType::Directory, false, &inside);
    app.confirm_pick();
    assert_eq!(result(&app), None);
    let _ = std::fs::remove_dir_all(&dir);
}

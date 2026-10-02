//! The stack's own bookkeeping: order, cancel, an undo overtaken by a new
//! action, renames that wait for root, item identity and the summaries.

use std::path::PathBuf;

use super::tests::{exec_job, replace_with, scratch, step};
use super::*;

#[test]
fn a_cancelled_job_leaves_its_items_on_their_stack() {
    let (d, td) = scratch("cancel");
    std::fs::write(d.join("copy"), b"c").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Copy(vec![(d.join("src"), d.join("copy"))]));
    stack.request(Direction::Undo);
    let Some(Next::Job(job)) = stack.next() else {
        panic!()
    };
    let out = exec_job(job, &|p| crate::trash::trash_in(&td, p), true);
    assert!(out.cancelled && out.done.is_none());
    let report = stack.finish(out);
    assert_eq!(report.summary.as_deref(), Some("Undo stopped"));
    assert!(d.join("copy").exists());
    assert!(stack.can_undo() && !stack.can_redo());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn requests_run_one_at_a_time_in_the_order_asked() {
    let (d, td) = scratch("order");
    std::fs::write(d.join("one"), b"1").unwrap();
    std::fs::write(d.join("two"), b"2").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Create(vec![(d.join("one"), false)]));
    stack.push(UndoAction::Create(vec![(d.join("two"), false)]));
    for _ in 0..3 {
        stack.request(Direction::Undo);
    }
    let Some(Next::Job(first)) = stack.next() else {
        panic!()
    };
    assert_eq!(first.entry.action, UndoAction::Create(vec![(d.join("two"), false)]));
    // The second waits for the first.
    assert!(stack.next().is_none());
    stack.finish(exec_job(first, &|p| crate::trash::trash_in(&td, p), false));
    let Some(Next::Job(second)) = stack.next() else {
        panic!()
    };
    assert_eq!(second.entry.action, UndoAction::Create(vec![(d.join("one"), false)]));
    stack.finish(exec_job(second, &|p| crate::trash::trash_in(&td, p), false));
    // The third press finds an empty stack.
    assert!(matches!(stack.next(), Some(Next::Nothing(Direction::Undo))));
    assert!(stack.next().is_none());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_undo_overtaken_by_a_new_action_is_not_offered_for_redo() {
    let (d, td) = scratch("overtaken");
    std::fs::write(d.join("made"), b"").unwrap();
    std::fs::write(d.join("later"), b"").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Create(vec![(d.join("made"), false)]));
    stack.request(Direction::Undo);
    let Some(Next::Job(job)) = stack.next() else {
        panic!()
    };
    // While the undo is on the worker the user does something new.
    stack.push(UndoAction::Create(vec![(d.join("later"), false)]));
    stack.finish(exec_job(job, &|p| crate::trash::trash_in(&td, p), false));
    assert!(!d.join("made").exists());
    assert!(!stack.can_redo());
    assert!(stack.can_undo());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_rename_that_needs_root_is_recorded_only_once_it_was_made() {
    use std::os::unix::fs::PermissionsExt;
    // Root is not stopped by a read-only folder.
    if unsafe { libc::getuid() } == 0 {
        return;
    }
    let (d, td) = scratch("root");
    let locked = d.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("b"), b"x").unwrap();
    let mut stack = UndoStack::new();
    let lock = |mode: u32| {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(mode)).unwrap()
    };

    for made in [false, true] {
        stack.push(UndoAction::Rename {
            from: locked.join("a"),
            to: locked.join("b"),
        });
        lock(0o555);
        let report = step(&mut stack, Direction::Undo, &td);
        lock(0o755);
        // Not done, not failed: handed to sudo, and nothing else starts.
        assert!(report.failures.is_empty() && report.summary.is_none());
        assert_eq!(report.root_moves, vec![(locked.join("b"), locked.join("a"))]);
        assert!(stack.waiting_for_root() && !stack.can_redo());
        let (b, a) = (locked.join("b"), locked.join("a"));
        assert!(stack.root_wait_is(&[(b.as_path(), a.as_path())]));
        stack.request(Direction::Undo);
        assert!(stack.next().is_none());

        if made {
            std::fs::rename(locked.join("b"), locked.join("a")).unwrap();
        }
        let line = stack.settle_root(|from, to| !from.exists() && to.exists());
        assert_eq!(line.is_some(), made);
        assert_eq!(stack.can_redo(), made);
        assert!(!stack.waiting_for_root());
        // The queued press runs now and finds nothing left.
        assert!(matches!(stack.next(), Some(Next::Nothing(Direction::Undo))));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_file_edited_in_place_is_the_same_item_and_a_rewritten_one_is_not() {
    let (d, _td) = scratch("stamp");
    let f = d.join("f");
    std::fs::write(&f, b"one").unwrap();
    let then = Stamp::of(&f).unwrap();
    std::fs::write(&f, b"one and more").unwrap();
    assert!(then.same_item(&Stamp::of(&f).unwrap()));
    replace_with(&f, b"entirely new");
    assert!(!then.same_item(&Stamp::of(&f).unwrap()));
    // A folder is never mistaken for the file it replaced.
    std::fs::remove_file(&f).unwrap();
    std::fs::create_dir(&f).unwrap();
    assert!(!then.same_item(&Stamp::of(&f).unwrap()));
    assert!(Stamp::of(&d.join("missing")).is_none());

    // An inode number handed out again to a file made later is not the
    // same item; a FAT entry that got a new number but is untouched is.
    std::fs::remove_dir(&f).unwrap();
    std::fs::write(&f, b"x").unwrap();
    let base = Stamp::of(&f).unwrap();
    let later = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(7);
    let reused = Stamp {
        born: Some(later),
        ..base
    };
    let unborn = Stamp { born: None, ..base };
    let reborn = Stamp {
        born: Some(later + std::time::Duration::from_secs(1)),
        ..base
    };
    assert!(!reused.same_item(&reborn));
    assert!(unborn.same_item(&reused) && reused.same_item(&unborn));
    let renumbered = Stamp {
        ino: base.ino + 1,
        ..base
    };
    assert!(base.same_item(&renumbered));
    let rewritten = Stamp {
        ino: base.ino + 1,
        len: base.len + 1,
        ..base
    };
    assert!(!base.same_item(&rewritten));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn summaries_say_what_happened_in_plain_words() {
    let entry = |action: UndoAction| Entry {
        notes: vec![Note::default(); action.len()],
        action,
    };
    let p = |s: &str| PathBuf::from(s);
    let moves = entry(UndoAction::Move(vec![
        (p("/a/x"), p("/b/x")),
        (p("/a/y"), p("/b/y")),
        (p("/a/z"), p("/b/z")),
    ]));
    assert_eq!(words::describe(Direction::Undo, &moves), "Undo: 3 items moved back");
    assert_eq!(words::describe(Direction::Redo, &moves), "Redo: 3 items moved again");
    let copies = entry(UndoAction::Copy(vec![
        (p("/a/x"), p("/b/x")),
        (p("/a/y"), p("/b/y")),
    ]));
    assert_eq!(
        words::describe(Direction::Undo, &copies),
        "Undo: 2 copies moved to the Trash"
    );
    assert_eq!(words::describe(Direction::Redo, &copies), "Redo: 2 copies put back");
    let trashed = entry(UndoAction::Trash(vec![(p("/a/x"), p("/t/x"), p("/t/x.i"))]));
    assert_eq!(
        words::describe(Direction::Undo, &trashed),
        "Undo: \u{201C}x\u{201D} restored from the Trash"
    );
    assert_eq!(Direction::Undo.verb(), "undone");
    assert_eq!(Direction::Redo.verb(), "redone");
}

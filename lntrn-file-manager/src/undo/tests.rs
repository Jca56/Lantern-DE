use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::*;
use crate::trash::{TrashDir, TrashError};

/// A scratch folder with a trash of its own, so nothing here touches the
/// user's real Trash.
pub(super) fn scratch(name: &str) -> (PathBuf, TrashDir) {
    let dir = std::env::temp_dir().join(format!("lntrn-fm-undo-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let td = TrashDir {
        dir: dir.join("the-trash"),
        top: Some(dir.clone()),
    };
    (dir, td)
}

pub(super) type Trasher<'a> = &'a dyn Fn(&Path) -> Result<Trashed, TrashError>;

/// Run a job on this thread, the way the worker does.
pub(super) fn exec_job(job: Job, trash: Trasher, cancelled: bool) -> Outcome {
    let cancel = AtomicBool::new(cancelled);
    let copy = |src: &Path, dst: &Path| match crate::copy_tree::copy_item(src, dst, &cancel) {
        Ok(report) => (true, report.failures),
        Err(_) => (false, Vec::new()),
    };
    let mut progress = |_: usize, _: usize, _: &str| {};
    exec::run(
        job,
        &mut exec::Env {
            cancel: &cancel,
            trash,
            copy: &copy,
            progress: &mut progress,
        },
    )
}

/// Ask for one undo or redo and run it to the end.
pub(super) fn step(stack: &mut UndoStack, dir: Direction, td: &TrashDir) -> Report {
    stack.request(dir);
    let Some(Next::Job(job)) = stack.next() else {
        panic!("nothing to {dir:?}");
    };
    let out = exec_job(job, &|p| crate::trash::trash_in(td, p), false);
    stack.finish(out)
}

/// Put other content under `path` the way an editor's safe save does: a
/// new file renamed over the name, so it is a different file on disk.
pub(super) fn replace_with(path: &Path, content: &[u8]) {
    let tmp = path.with_extension("swap-tmp");
    std::fs::write(&tmp, content).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

pub(super) fn in_trash(td: &TrashDir) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(td.files())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn undo_of_a_copy_moves_it_to_the_trash_and_redo_brings_it_back() {
    let (d, td) = scratch("copy");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("a.txt"), b"original").unwrap();
    std::fs::copy(d.join("a.txt"), d.join("dest/a.txt")).unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Copy(vec![(d.join("a.txt"), d.join("dest/a.txt"))]));

    // The copy was worked on since; undo must not destroy that.
    std::fs::write(d.join("dest/a.txt"), b"edited in place").unwrap();
    let report = step(&mut stack, Direction::Undo, &td);
    assert!(report.failures.is_empty());
    assert_eq!(
        report.summary.as_deref(),
        Some("Undo: the copy \u{201C}a.txt\u{201D} moved to the Trash")
    );
    assert!(!d.join("dest/a.txt").exists());
    assert_eq!(in_trash(&td), vec!["a.txt"]);
    assert_eq!(std::fs::read(td.files().join("a.txt")).unwrap(), b"edited in place");
    assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"original");
    assert!(stack.can_redo() && !stack.can_undo());

    // Redo takes that very file out of the Trash again; nothing is copied.
    let report = step(&mut stack, Direction::Redo, &td);
    assert!(report.failures.is_empty());
    assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"edited in place");
    assert!(in_trash(&td).is_empty());
    assert!(stack.can_undo() && !stack.can_redo());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn undo_leaves_alone_an_item_that_replaced_what_was_made() {
    let (d, td) = scratch("replaced");
    std::fs::write(d.join("src.txt"), b"src").unwrap();
    std::fs::write(d.join("copy.txt"), b"src").unwrap();
    std::fs::write(d.join("New File"), b"").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Create(vec![(d.join("New File"), false)]));
    stack.push(UndoAction::Copy(vec![(d.join("src.txt"), d.join("copy.txt"))]));

    // Something else has both names now.
    replace_with(&d.join("copy.txt"), b"a different file altogether");
    replace_with(&d.join("New File"), b"a day of work");

    let report = step(&mut stack, Direction::Undo, &td);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("not the item that was copied"));
    assert!(report.summary.is_none());
    assert_eq!(
        std::fs::read(d.join("copy.txt")).unwrap(),
        b"a different file altogether"
    );

    let report = step(&mut stack, Direction::Undo, &td);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("not the item that was created"));
    assert_eq!(std::fs::read(d.join("New File")).unwrap(), b"a day of work");

    // Nothing went to the Trash, and a reversal that did not happen is
    // not something to redo.
    assert!(in_trash(&td).is_empty());
    assert!(!stack.can_redo() && !stack.can_undo());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn undo_of_new_folder_keeps_what_was_put_into_it() {
    let (d, td) = scratch("create");
    std::fs::create_dir(d.join("New Folder")).unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Create(vec![(d.join("New Folder"), true)]));
    // Photos were moved in (a move from a phone leaves no undo entry).
    std::fs::write(d.join("New Folder/photo.jpg"), b"jpeg").unwrap();

    let report = step(&mut stack, Direction::Undo, &td);
    assert!(report.failures.is_empty());
    assert_eq!(
        report.summary.as_deref(),
        Some("Undo: \u{201C}New Folder\u{201D} moved to the Trash")
    );
    assert!(!d.join("New Folder").exists());
    assert_eq!(
        std::fs::read(td.files().join("New Folder/photo.jpg")).unwrap(),
        b"jpeg"
    );

    // Redo puts the folder back with everything in it.
    let report = step(&mut stack, Direction::Redo, &td);
    assert!(report.failures.is_empty());
    assert_eq!(std::fs::read(d.join("New Folder/photo.jpg")).unwrap(), b"jpeg");
    assert!(in_trash(&td).is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn without_a_trash_only_an_empty_created_folder_is_removed() {
    let (d, _td) = scratch("no-trash");
    let no_trash: Trasher = &|_| Err(TrashError::NoTrash("the drive is read-only".into()));
    std::fs::create_dir(d.join("empty")).unwrap();
    std::fs::create_dir(d.join("full")).unwrap();
    std::fs::write(d.join("file"), b"").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Create(vec![
        (d.join("empty"), true),
        (d.join("full"), true),
        (d.join("file"), false),
    ]));
    std::fs::write(d.join("full/kept.txt"), b"kept").unwrap();

    stack.request(Direction::Undo);
    let Some(Next::Job(job)) = stack.next() else {
        panic!()
    };
    let report = stack.finish(exec_job(job, no_trash, false));
    assert!(!d.join("empty").exists());
    assert_eq!(std::fs::read(d.join("full/kept.txt")).unwrap(), b"kept");
    assert!(d.join("file").exists());
    assert_eq!(report.failures.len(), 2);
    assert_eq!(report.summary.as_deref(), Some("Undo: \u{201C}empty\u{201D} removed"));

    // Redo makes the empty folder again, and only that one.
    stack.request(Direction::Redo);
    let Some(Next::Job(job)) = stack.next() else {
        panic!()
    };
    assert_eq!(job.len(), 1);
    let report = stack.finish(exec_job(job, no_trash, false));
    assert!(report.failures.is_empty() && d.join("empty").is_dir());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn undo_of_rename_trash_and_move_never_overwrites_a_newer_item() {
    let (d, td) = scratch("no-overwrite");
    std::fs::create_dir_all(d.join("moved-to")).unwrap();
    let mut stack = UndoStack::new();

    // Trash notes.txt, then an editor saves a new notes.txt.
    std::fs::write(d.join("notes.txt"), b"old notes").unwrap();
    let t = crate::trash::trash_in(&td, &d.join("notes.txt")).unwrap();
    stack.push(UndoAction::Trash(vec![(
        t.original.clone(),
        t.trashed.clone(),
        t.info.clone(),
    )]));
    std::fs::write(d.join("notes.txt"), b"new notes").unwrap();
    let report = step(&mut stack, Direction::Undo, &td);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("still in the Trash"));
    assert_eq!(std::fs::read(d.join("notes.txt")).unwrap(), b"new notes");
    assert_eq!(std::fs::read(&t.trashed).unwrap(), b"old notes");
    assert!(t.info.exists() && !stack.can_redo());

    // Rename a.txt -> b.txt, then a new a.txt appears.
    std::fs::write(d.join("b.txt"), b"renamed").unwrap();
    stack.push(UndoAction::Rename {
        from: d.join("a.txt"),
        to: d.join("b.txt"),
    });
    std::fs::write(d.join("a.txt"), b"newer a").unwrap();
    let report = step(&mut stack, Direction::Undo, &td);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("nothing was overwritten"));
    assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"newer a");
    assert_eq!(std::fs::read(d.join("b.txt")).unwrap(), b"renamed");

    // Move of two items; one original name is taken again.
    std::fs::write(d.join("moved-to/one"), b"1").unwrap();
    std::fs::write(d.join("moved-to/two"), b"2").unwrap();
    stack.push(UndoAction::Move(vec![
        (d.join("one"), d.join("moved-to/one")),
        (d.join("two"), d.join("moved-to/two")),
    ]));
    std::fs::write(d.join("one"), b"newer one").unwrap();
    let report = step(&mut stack, Direction::Undo, &td);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(std::fs::read(d.join("one")).unwrap(), b"newer one");
    assert_eq!(std::fs::read(d.join("moved-to/one")).unwrap(), b"1");
    assert_eq!(std::fs::read(d.join("two")).unwrap(), b"2");
    assert_eq!(
        report.summary.as_deref(),
        Some("Undo: \u{201C}two\u{201D} moved back")
    );

    // Redo covers the one item that was moved back, nothing else.
    let report = step(&mut stack, Direction::Redo, &td);
    assert!(report.failures.is_empty());
    assert_eq!(std::fs::read(d.join("moved-to/two")).unwrap(), b"2");
    assert_eq!(std::fs::read(d.join("one")).unwrap(), b"newer one");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn undo_and_redo_of_rename_and_trash_round_trip() {
    let (d, td) = scratch("round-trip");
    std::fs::write(d.join("b.txt"), b"x").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Rename {
        from: d.join("a.txt"),
        to: d.join("b.txt"),
    });
    let report = step(&mut stack, Direction::Undo, &td);
    assert_eq!(
        report.summary.as_deref(),
        Some("Undo: renamed \u{201C}b.txt\u{201D} back to \u{201C}a.txt\u{201D}")
    );
    assert!(d.join("a.txt").exists() && !d.join("b.txt").exists());
    let report = step(&mut stack, Direction::Redo, &td);
    assert_eq!(
        report.summary.as_deref(),
        Some("Redo: renamed \u{201C}a.txt\u{201D} to \u{201C}b.txt\u{201D}")
    );
    assert!(d.join("b.txt").exists() && !d.join("a.txt").exists());

    let t = crate::trash::trash_in(&td, &d.join("b.txt")).unwrap();
    stack.push(UndoAction::Trash(vec![(t.original, t.trashed, t.info)]));
    step(&mut stack, Direction::Undo, &td);
    assert!(d.join("b.txt").exists() && in_trash(&td).is_empty());
    // Redo trashes it under a fresh record, which the next undo follows.
    let report = step(&mut stack, Direction::Redo, &td);
    assert_eq!(
        report.summary.as_deref(),
        Some("Redo: \u{201C}b.txt\u{201D} moved to the Trash")
    );
    assert!(!d.join("b.txt").exists());
    assert_eq!(in_trash(&td), vec!["b.txt"]);
    step(&mut stack, Direction::Undo, &td);
    assert_eq!(std::fs::read(d.join("b.txt")).unwrap(), b"x");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn redo_follows_the_same_rules() {
    let (d, td) = scratch("redo-rules");
    let mut stack = UndoStack::new();

    // Undo a trash, then another file takes the name: redo must not trash
    // the stranger.
    std::fs::write(d.join("doc.txt"), b"mine").unwrap();
    let t = crate::trash::trash_in(&td, &d.join("doc.txt")).unwrap();
    stack.push(UndoAction::Trash(vec![(t.original, t.trashed, t.info)]));
    step(&mut stack, Direction::Undo, &td);
    replace_with(&d.join("doc.txt"), b"someone else's newer file");
    let report = step(&mut stack, Direction::Redo, &td);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("another item has this name"));
    assert_eq!(
        std::fs::read(d.join("doc.txt")).unwrap(),
        b"someone else's newer file"
    );
    assert!(in_trash(&td).is_empty() && !stack.can_undo());

    // Undo a rename, then the new name is taken: redo must not overwrite.
    std::fs::write(d.join("b"), b"item").unwrap();
    stack.push(UndoAction::Rename {
        from: d.join("a"),
        to: d.join("b"),
    });
    step(&mut stack, Direction::Undo, &td);
    std::fs::write(d.join("b"), b"newer b").unwrap();
    let report = step(&mut stack, Direction::Redo, &td);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(std::fs::read(d.join("a")).unwrap(), b"item");
    assert_eq!(std::fs::read(d.join("b")).unwrap(), b"newer b");

    // Undo a copy, then the name is taken: the copy stays in the Trash.
    std::fs::write(d.join("src"), b"s").unwrap();
    std::fs::write(d.join("copy"), b"s").unwrap();
    stack.push(UndoAction::Copy(vec![(d.join("src"), d.join("copy"))]));
    step(&mut stack, Direction::Undo, &td);
    std::fs::write(d.join("copy"), b"newer copy").unwrap();
    let report = step(&mut stack, Direction::Redo, &td);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].reason.contains("still in the Trash"));
    assert_eq!(std::fs::read(d.join("copy")).unwrap(), b"newer copy");
    assert_eq!(in_trash(&td), vec!["copy"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn redo_of_a_copy_copies_again_only_when_its_trashed_copy_is_gone() {
    let (d, td) = scratch("recopy");
    std::fs::create_dir_all(d.join("tree/sub")).unwrap();
    std::fs::write(d.join("tree/sub/f"), b"data").unwrap();
    std::fs::create_dir_all(d.join("dest")).unwrap();
    let never = AtomicBool::new(false);
    crate::copy_tree::copy_item(&d.join("tree"), &d.join("dest/tree"), &never).unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Copy(vec![(d.join("tree"), d.join("dest/tree"))]));

    step(&mut stack, Direction::Undo, &td);
    assert!(!d.join("dest/tree").exists());
    // The Trash is emptied before the redo.
    std::fs::remove_dir_all(td.files().join("tree")).unwrap();
    let report = step(&mut stack, Direction::Redo, &td);
    assert!(report.failures.is_empty());
    assert_eq!(std::fs::read(d.join("dest/tree/sub/f")).unwrap(), b"data");

    // The fresh copy is a new item, and undo recognises it as such.
    let report = step(&mut stack, Direction::Undo, &td);
    assert!(report.failures.is_empty());
    assert!(!d.join("dest/tree").exists());
    let names = in_trash(&td);
    assert_eq!(names.len(), 1);
    assert!(td.files().join(&names[0]).join("sub/f").exists());
    let _ = std::fs::remove_dir_all(&d);
}

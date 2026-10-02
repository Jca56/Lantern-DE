use super::worker::run;
use super::*;
use crate::trash::{TrashDir, TrashError};

/// A scratch folder with a trash of its own, so nothing here touches the
/// user's real Trash.
fn scratch(name: &str) -> (PathBuf, TrashDir) {
    let dir = std::env::temp_dir().join(format!("lntrn-fm-ops-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let td = TrashDir {
        dir: dir.join("the-trash"),
        top: Some(dir.clone()),
    };
    (dir, td)
}

fn item(src: &Path, target: &Path, replace: bool) -> OpItem {
    OpItem {
        src: src.to_path_buf(),
        target: target.to_path_buf(),
        replace,
    }
}

/// Run a job on this thread and return its outcome.
fn run_sync(kind: OpKind, items: Vec<OpItem>, td: &TrashDir, cancelled: bool) -> OpOutcome {
    let (tx, rx) = mpsc::channel();
    let cancel = AtomicBool::new(cancelled);
    run(kind, items, tx, &cancel, &|p| crate::trash::trash_in(td, p));
    loop {
        if let OpProgress::Done(out) = rx.recv().unwrap() {
            return out;
        }
    }
}

/// Names left in `dir`, sorted; staging leftovers would show up here.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn replace_trashes_the_old_item_only_once_the_new_one_is_in() {
    let (d, td) = scratch("replace");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("a.txt"), b"new").unwrap();
    std::fs::write(d.join("dest/a.txt"), b"old").unwrap();

    let out = run_sync(
        OpKind::Copy,
        vec![item(&d.join("a.txt"), &d.join("dest/a.txt"), true)],
        &td,
        false,
    );
    assert!(out.failures.is_empty() && out.perm_fails.is_empty());
    assert_eq!(out.created, vec![(d.join("a.txt"), d.join("dest/a.txt"))]);
    assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"new");
    // The old file is in the trash, restorable, not deleted.
    assert_eq!(out.replaced.len(), 1);
    assert_eq!(std::fs::read(&out.replaced[0].trashed).unwrap(), b"old");
    assert!(out.replaced[0].info.exists());
    assert_eq!(names(&d.join("dest")), vec!["a.txt"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failed_or_cancelled_replace_leaves_the_old_item_untouched() {
    let (d, td) = scratch("failed-replace");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("dest/a.txt"), b"old").unwrap();

    // The source is gone (a phone that disconnected): nothing to copy.
    let out = run_sync(
        OpKind::Copy,
        vec![item(&d.join("missing.txt"), &d.join("dest/a.txt"), true)],
        &td,
        false,
    );
    assert_eq!(out.failures.len(), 1);
    assert!(out.created.is_empty() && out.replaced.is_empty());
    assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"old");
    assert_eq!(names(&d.join("dest")), vec!["a.txt"]);

    // Cancelled before it started.
    std::fs::write(d.join("a.txt"), b"new").unwrap();
    let out = run_sync(
        OpKind::Copy,
        vec![item(&d.join("a.txt"), &d.join("dest/a.txt"), true)],
        &td,
        true,
    );
    assert!(out.cancelled && out.created.is_empty());
    assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"old");

    // No Trash for the old item: it stays, and the new copy is removed.
    let (tx, rx) = mpsc::channel();
    run(
        OpKind::Copy,
        vec![item(&d.join("a.txt"), &d.join("dest/a.txt"), true)],
        tx,
        &AtomicBool::new(false),
        &|_| Err(TrashError::NoTrash("the drive is read-only".into())),
    );
    let out = rx
        .iter()
        .find_map(|e| match e {
            OpProgress::Done(out) => Some(out),
            _ => None,
        })
        .unwrap();
    assert_eq!(out.failures.len(), 1);
    assert!(out.failures[0].reason.contains("read-only"));
    assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"old");
    assert_eq!(names(&d.join("dest")), vec!["a.txt"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn without_replace_an_existing_target_is_never_overwritten() {
    let (d, td) = scratch("noclobber");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("a.txt"), b"new").unwrap();
    std::fs::write(d.join("dest/a.txt"), b"old").unwrap();
    for kind in [OpKind::Copy, OpKind::Move] {
        let out = run_sync(
            kind,
            vec![item(&d.join("a.txt"), &d.join("dest/a.txt"), false)],
            &td,
            false,
        );
        assert_eq!(out.failures.len(), 1);
        assert!(out.created.is_empty() && out.renamed.is_empty());
        assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"new");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn replacing_a_folder_moves_the_whole_old_folder_to_the_trash() {
    let (d, td) = scratch("folder");
    std::fs::create_dir_all(d.join("Photos")).unwrap();
    std::fs::write(d.join("Photos/new.jpg"), b"n").unwrap();
    std::fs::create_dir_all(d.join("dest/Photos")).unwrap();
    std::fs::write(d.join("dest/Photos/only-here.jpg"), b"o").unwrap();

    let out = run_sync(
        OpKind::Copy,
        vec![item(&d.join("Photos"), &d.join("dest/Photos"), true)],
        &td,
        false,
    );
    assert!(out.failures.is_empty());
    // Not merged: the new folder holds only the new files...
    assert_eq!(names(&d.join("dest/Photos")), vec!["new.jpg"]);
    // ...and the old one is intact in the trash.
    assert_eq!(
        std::fs::read(out.replaced[0].trashed.join("only-here.jpg")).unwrap(),
        b"o"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_move_on_one_filesystem_is_a_rename_and_replace_goes_through_the_trash() {
    let (d, td) = scratch("move");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("a.txt"), b"new").unwrap();
    std::fs::write(d.join("b.txt"), b"plain").unwrap();
    std::fs::write(d.join("dest/a.txt"), b"old").unwrap();

    let out = run_sync(
        OpKind::Move,
        vec![
            item(&d.join("a.txt"), &d.join("dest/a.txt"), true),
            item(&d.join("b.txt"), &d.join("dest/b.txt"), false),
        ],
        &td,
        false,
    );
    assert!(out.failures.is_empty());
    assert_eq!(out.renamed.len(), 2);
    assert!(out.created.is_empty());
    assert!(!d.join("a.txt").exists() && !d.join("b.txt").exists());
    assert_eq!(std::fs::read(d.join("dest/a.txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(d.join("dest/b.txt")).unwrap(), b"plain");
    assert_eq!(std::fs::read(&out.replaced[0].trashed).unwrap(), b"old");

    // A move whose source is gone puts the old item back.
    std::fs::write(d.join("dest/c.txt"), b"old-c").unwrap();
    let out = run_sync(
        OpKind::Move,
        vec![item(&d.join("missing.txt"), &d.join("dest/c.txt"), true)],
        &td,
        false,
    );
    assert_eq!(out.failures.len(), 1);
    assert!(out.replaced.is_empty());
    assert_eq!(std::fs::read(d.join("dest/c.txt")).unwrap(), b"old-c");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_folder_with_a_bad_entry_lands_with_that_entry_reported() {
    let (d, td) = scratch("bad-entry");
    use std::os::unix::ffi::OsStrExt;
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("src/a"), b"a").unwrap();
    let fifo = std::ffi::CString::new(d.join("src/pipe").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::create_dir_all(d.join("dest")).unwrap();

    let out = run_sync(
        OpKind::Copy,
        vec![item(&d.join("src"), &d.join("dest/src"), false)],
        &td,
        false,
    );
    assert_eq!(out.created.len(), 1);
    assert_eq!(out.failures.len(), 1);
    assert_eq!(out.failures[0].path, d.join("src/pipe"));
    assert_eq!(names(&d.join("dest/src")), vec!["a"]);

    // The same folder as a Replace: an incomplete copy does not push the
    // existing folder out.
    std::fs::write(d.join("dest/src/mine"), b"m").unwrap();
    let out = run_sync(
        OpKind::Copy,
        vec![item(&d.join("src"), &d.join("dest/src"), true)],
        &td,
        false,
    );
    assert!(out.created.is_empty() && out.replaced.is_empty());
    assert_eq!(names(&d.join("dest/src")), vec!["a", "mine"]);
    assert_eq!(names(&d.join("dest")), vec!["src"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn delete_removes_files_folders_and_links_but_not_link_targets() {
    let (d, td) = scratch("delete");
    std::fs::create_dir_all(d.join("tree/sub")).unwrap();
    std::fs::write(d.join("tree/sub/f"), b"x").unwrap();
    std::fs::write(d.join("file"), b"x").unwrap();
    std::fs::create_dir_all(d.join("kept")).unwrap();
    std::fs::write(d.join("kept/f"), b"x").unwrap();
    std::os::unix::fs::symlink(d.join("kept"), d.join("link")).unwrap();

    let out = run_sync(
        OpKind::Delete,
        ["tree", "file", "link", "already-gone"]
            .iter()
            .map(|n| OpItem::delete(d.join(n)))
            .collect(),
        &td,
        false,
    );
    assert!(out.failures.is_empty() && out.perm_fails.is_empty());
    assert_eq!(names(&d), vec!["kept"]);
    assert!(d.join("kept/f").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn queued_operations_run_one_after_another_and_each_is_finalised() {
    let (d, _td) = scratch("queue");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    let mut queue = OpQueue::new();
    for n in ["one", "two", "three"] {
        std::fs::write(d.join(n), n.as_bytes()).unwrap();
        queue.push(OpRequest {
            kind: OpKind::Copy,
            label: "Copying",
            items: vec![item(&d.join(n), &d.join("dest").join(n), false)],
            dest: d.join("dest"),
            rearm_clipboard: None,
        });
    }
    // Names still on their way count as taken.
    assert!(queue.is_reserved(&d.join("dest/three")));
    assert!(!queue.is_reserved(&d.join("dest/four")));

    let mut finished = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while queue.is_busy() && std::time::Instant::now() < deadline {
        finished.extend(queue.poll().1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(finished.len(), 3);
    assert!(finished.iter().all(|f| f.outcome.created.len() == 1));
    assert_eq!(queue.others(), 0);
    assert_eq!(names(&d.join("dest")), vec!["one", "three", "two"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn renames_and_local_deletes_do_not_wait_behind_a_copy() {
    let (d, _td) = scratch("lanes");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("a"), b"a").unwrap();
    let req = |kind, items| OpRequest {
        kind,
        label: "x",
        items,
        dest: d.join("dest"),
        rearm_clipboard: None,
    };
    // Copies always take the queue.
    assert!(!runs_beside(&req(
        OpKind::Copy,
        vec![item(&d.join("a"), &d.join("dest/a"), false)]
    )));
    // A move within one filesystem is a rename; a local delete is local.
    assert!(runs_beside(&req(
        OpKind::Move,
        vec![item(&d.join("a"), &d.join("dest/a"), false)]
    )));
    assert!(runs_beside(&req(OpKind::Delete, vec![OpItem::delete(d.join("a"))])));
    // A move to another filesystem streams data: it queues.
    if let Some((a, b)) = two_filesystems("lanes-x") {
        assert!(!runs_beside(&req(
            OpKind::Move,
            vec![item(&a.join("f"), &b.join("f"), false)]
        )));
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Two scratch folders on different filesystems, when this machine has
/// them: one under the temp dir (often a tmpfs), one next to the test
/// binary in the build directory.
fn two_filesystems(name: &str) -> Option<(PathBuf, PathBuf)> {
    use std::os::unix::fs::MetadataExt;
    let (a, _) = scratch(name);
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let b = exe_dir.join(format!("lntrn-fm-ops-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&b).ok()?;
    let dev = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
    if dev(&a) == dev(&b) {
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
        return None;
    }
    Some((a, b.canonicalize().ok()?))
}

#[test]
fn a_cross_device_move_deletes_from_the_source_only_what_arrived() {
    let Some((a, b)) = two_filesystems("xdev") else {
        eprintln!("skipped: temp dir and build dir are on one filesystem");
        return;
    };
    use std::os::unix::ffi::OsStrExt;
    let td = TrashDir {
        dir: b.join("the-trash"),
        top: Some(b.clone()),
    };
    // A folder with something that cannot be copied in the middle of it.
    std::fs::create_dir_all(a.join("album/sub")).unwrap();
    std::fs::write(a.join("album/one.jpg"), b"1111").unwrap();
    std::fs::write(a.join("album/sub/two.jpg"), b"22").unwrap();
    let fifo = std::ffi::CString::new(a.join("album/sub/pipe").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    // The target name is taken; the user chose Replace... which an
    // incomplete copy must not carry out.
    std::fs::create_dir_all(b.join("album")).unwrap();
    std::fs::write(b.join("album/old.jpg"), b"old").unwrap();

    let out = run_sync(
        OpKind::Move,
        vec![item(&a.join("album"), &b.join("album"), true)],
        &td,
        false,
    );
    assert!(out.created.is_empty() && out.replaced.is_empty());
    assert_eq!(names(&b.join("album")), vec!["old.jpg"]);
    // Nothing was deleted from the source.
    assert_eq!(std::fs::read(a.join("album/one.jpg")).unwrap(), b"1111");
    assert_eq!(std::fs::read(a.join("album/sub/two.jpg")).unwrap(), b"22");

    // Keep Both instead: the folder lands, minus the pipe. The files that
    // arrived leave the source; the pipe and the folders holding it stay.
    let out = run_sync(
        OpKind::Move,
        vec![item(&a.join("album"), &b.join("album (2)"), false)],
        &td,
        false,
    );
    assert_eq!(out.created.len(), 1);
    assert!(out.renamed.is_empty());
    assert_eq!(std::fs::read(b.join("album (2)/one.jpg")).unwrap(), b"1111");
    assert_eq!(std::fs::read(b.join("album (2)/sub/two.jpg")).unwrap(), b"22");
    assert!(!a.join("album/one.jpg").exists());
    assert!(!a.join("album/sub/two.jpg").exists());
    assert!(std::fs::symlink_metadata(a.join("album/sub/pipe")).is_ok());
    assert!(out.failures.iter().any(|f| f.path == a.join("album/sub/pipe")));
    // No staging leftovers on the destination.
    assert_eq!(names(&b), vec!["album", "album (2)"]);

    // A clean file move with Replace: old to the trash, source gone.
    std::fs::write(a.join("song.mp3"), b"new song").unwrap();
    std::fs::write(b.join("song.mp3"), b"old song").unwrap();
    let out = run_sync(
        OpKind::Move,
        vec![item(&a.join("song.mp3"), &b.join("song.mp3"), true)],
        &td,
        false,
    );
    assert!(out.failures.is_empty());
    assert_eq!(out.created.len(), 1);
    assert_eq!(std::fs::read(b.join("song.mp3")).unwrap(), b"new song");
    assert_eq!(std::fs::read(&out.replaced[0].trashed).unwrap(), b"old song");
    assert!(!a.join("song.mp3").exists());

    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn an_undo_runs_on_a_worker_and_reports_like_any_operation() {
    use crate::undo::{Direction, Next, UndoAction, UndoStack};
    let (d, _td) = scratch("history");
    std::fs::write(d.join("b.txt"), b"x").unwrap();
    let mut stack = UndoStack::new();
    stack.push(UndoAction::Rename {
        from: d.join("a.txt"),
        to: d.join("b.txt"),
    });
    stack.request(Direction::Undo);
    let Some(Next::Job(job)) = stack.next() else {
        panic!("nothing to undo");
    };

    let mut queue = OpQueue::new();
    queue.push_history(job);
    assert!(queue.is_busy());
    assert_eq!(queue.shown().map(|h| h.label), Some("Undoing"));
    let mut finished = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while queue.is_busy() && std::time::Instant::now() < deadline {
        finished.extend(queue.poll().1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(finished.len(), 1);
    let done = finished.pop().unwrap();
    assert_eq!(done.kind, OpKind::History);
    let report = stack.finish(done.outcome.history.expect("the job's result"));
    assert!(report.failures.is_empty());
    assert!(d.join("a.txt").exists() && !d.join("b.txt").exists());
    assert!(stack.can_redo());
    let _ = std::fs::remove_dir_all(&d);
}

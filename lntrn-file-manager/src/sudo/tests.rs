//! The privileged copy and move commands, run the way sudo would run them
//! (minus sudo) against a scratch folder.

use super::*;
use std::path::Path;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lntrn-fm-sudo-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run a built command the way sudo would, minus sudo.
fn run(argv: &[OsString]) -> bool {
    Command::new(&argv[0])
        .args(&argv[1..])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn item(src: &std::path::Path, target: &std::path::Path) -> PrivItem {
    PrivItem {
        src: src.to_path_buf(),
        target: target.to_path_buf(),
        old_to_trash: None,
    }
}

#[test]
fn privileged_copy_and_move_use_the_exact_target_and_never_overwrite() {
    let d = scratch("exact");
    std::fs::write(d.join("report.txt"), b"new").unwrap();
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("dest/report.txt"), b"kept").unwrap();

    // Keep Both: lands at the chosen name, the original is untouched.
    assert!(run(&copy_argv(&item(&d.join("report.txt"), &d.join("dest/report (2).txt")))));
    assert_eq!(std::fs::read(d.join("dest/report (2).txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(d.join("dest/report.txt")).unwrap(), b"kept");

    // An existing target without a resolved Replace is refused.
    assert!(!run(&copy_argv(&item(&d.join("report.txt"), &d.join("dest/report.txt")))));
    assert!(!run(&move_argv(&item(&d.join("report.txt"), &d.join("dest/report.txt")))));
    assert_eq!(std::fs::read(d.join("dest/report.txt")).unwrap(), b"kept");
    assert!(d.join("report.txt").exists());

    // A folder never lands inside an existing folder of the same name.
    std::fs::create_dir_all(d.join("conf")).unwrap();
    std::fs::write(d.join("conf/a"), b"a").unwrap();
    std::fs::create_dir_all(d.join("dest/conf")).unwrap();
    assert!(!run(&copy_argv(&item(&d.join("conf"), &d.join("dest/conf")))));
    assert!(!d.join("dest/conf/conf").exists());
    assert!(run(&copy_argv(&item(&d.join("conf"), &d.join("dest/conf (2)")))));
    assert!(d.join("dest/conf (2)/a").exists());
    // No staging leftovers.
    let stray = std::fs::read_dir(d.join("dest"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".fox-part-"));
    assert!(!stray);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn privileged_replace_moves_the_old_item_to_its_trash_slot() {
    let d = scratch("replace");
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::create_dir_all(d.join("trash")).unwrap();
    std::fs::write(d.join("new.txt"), b"new").unwrap();
    std::fs::write(d.join("dest/new.txt"), b"old").unwrap();
    let slot = crate::trash::Trashed {
        original: d.join("dest/new.txt"),
        trashed: d.join("trash/new.txt"),
        info: d.join("trash/new.txt.trashinfo"),
    };
    let mut it = item(&d.join("new.txt"), &d.join("dest/new.txt"));
    it.old_to_trash = Some(slot.clone());
    assert!(run(&copy_argv(&it)));
    assert_eq!(std::fs::read(d.join("dest/new.txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(&slot.trashed).unwrap(), b"old");

    // A failed replace (source missing) puts the old item back.
    std::fs::write(d.join("dest/b.txt"), b"old-b").unwrap();
    let slot_b = crate::trash::Trashed {
        original: d.join("dest/b.txt"),
        trashed: d.join("trash/b.txt"),
        info: d.join("trash/b.txt.trashinfo"),
    };
    for build in [copy_argv as fn(&PrivItem) -> Vec<OsString>, move_argv] {
        let mut it = item(&d.join("missing.txt"), &d.join("dest/b.txt"));
        it.old_to_trash = Some(slot_b.clone());
        assert!(!run(&build(&it)));
        assert_eq!(std::fs::read(d.join("dest/b.txt")).unwrap(), b"old-b");
        assert!(!slot_b.trashed.exists());
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_replace_move_stages_first_and_never_deletes_what_landed() {
    let d = scratch("move-replace");
    std::fs::create_dir_all(d.join("dest/conf")).unwrap();
    std::fs::create_dir_all(d.join("trash")).unwrap();
    std::fs::write(d.join("dest/conf/old"), b"old").unwrap();
    std::fs::create_dir_all(d.join("conf/sub")).unwrap();
    std::fs::write(d.join("conf/a"), b"a").unwrap();
    std::fs::write(d.join("conf/sub/b"), b"b").unwrap();
    let mut it = item(&d.join("conf"), &d.join("dest/conf"));
    it.old_to_trash = Some(crate::trash::Trashed {
        original: d.join("dest/conf"),
        trashed: d.join("trash/conf"),
        info: d.join("trash/conf.trashinfo"),
    });
    assert!(run(&move_argv(&it)));
    // The new folder took the name whole, the old one is in its slot, the
    // source is gone and no staging name is left.
    assert_eq!(std::fs::read(d.join("dest/conf/a")).unwrap(), b"a");
    assert_eq!(std::fs::read(d.join("dest/conf/sub/b")).unwrap(), b"b");
    assert!(!d.join("dest/conf/old").exists());
    assert_eq!(std::fs::read(d.join("trash/conf/old")).unwrap(), b"old");
    assert!(!d.join("conf").exists());
    let stray = std::fs::read_dir(d.join("dest"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".fox-part-"));
    assert!(!stray);
    // The script has no way to delete the real name.
    assert!(!MOVE_REPLACE.contains("rm -rf -- \"$2\""));
    assert!(!MOVE_REPLACE.contains("rm -rf -- \"$4\""));
    let _ = std::fs::remove_dir_all(&d);
}

/// The same across two filesystems, where a rename cannot do it: the copy
/// path of the script. (Skipped where /dev/shm is not a second filesystem.)
#[test]
fn a_replace_move_across_filesystems_lands_whole_and_removes_the_source_last() {
    use std::os::unix::fs::MetadataExt;
    let d = scratch("move-replace-far");
    let far_root = Path::new("/dev/shm");
    let dev = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
    if dev(far_root).is_none() || dev(far_root) == dev(&d) {
        eprintln!("no second filesystem to test with: skipped");
        return;
    }
    let far = far_root.join(format!("lntrn-fm-sudo-{}-far", std::process::id()));
    let _ = std::fs::remove_dir_all(&far);
    std::fs::create_dir_all(far.join("conf/sub")).unwrap();
    std::fs::write(far.join("conf/a"), b"a").unwrap();
    std::fs::write(far.join("conf/sub/b"), b"b").unwrap();
    std::fs::create_dir_all(d.join("dest/conf")).unwrap();
    std::fs::create_dir_all(d.join("trash")).unwrap();
    std::fs::write(d.join("dest/conf/old"), b"old").unwrap();

    let mut it = item(&far.join("conf"), &d.join("dest/conf"));
    it.old_to_trash = Some(crate::trash::Trashed {
        original: d.join("dest/conf"),
        trashed: d.join("trash/conf"),
        info: d.join("trash/conf.trashinfo"),
    });
    assert!(run(&move_argv(&it)));
    assert_eq!(std::fs::read(d.join("dest/conf/a")).unwrap(), b"a");
    assert_eq!(std::fs::read(d.join("dest/conf/sub/b")).unwrap(), b"b");
    assert_eq!(std::fs::read(d.join("trash/conf/old")).unwrap(), b"old");
    assert!(!far.join("conf").exists(), "the source is removed once the copy is in place");
    let stray = std::fs::read_dir(d.join("dest"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".fox-part-"));
    assert!(!stray);

    // A source that is not there: nothing is trashed, nothing is left.
    let mut it = item(&far.join("missing"), &d.join("dest/conf"));
    it.old_to_trash = Some(crate::trash::Trashed {
        original: d.join("dest/conf"),
        trashed: d.join("trash/conf-2"),
        info: d.join("trash/conf-2.trashinfo"),
    });
    assert!(!run(&move_argv(&it)));
    assert_eq!(std::fs::read(d.join("dest/conf/a")).unwrap(), b"a");
    assert!(!d.join("trash/conf-2").exists());
    let _ = std::fs::remove_dir_all(&far);
    let _ = std::fs::remove_dir_all(&d);
}

/// "/" is a mount point: everything about it stands for "a folder a device
/// is mounted on". No privileged op may move, rename, replace or delete it.
#[test]
fn an_op_never_keeps_an_item_that_is_or_replaces_a_mount_point() {
    let d = scratch("mounts");
    std::fs::write(d.join("plain"), b"x").unwrap();
    let plain = || item(&d.join("plain"), &d.join("elsewhere"));

    // Moving or renaming a mount point away.
    let moving_root = item(Path::new("/"), &d.join("root-copy"));
    let (op, refused) = PendingPrivOp::Move {
        items: vec![moving_root.clone(), plain()],
    }
    .without_mounts();
    assert!(matches!(op, Some(PendingPrivOp::Move { items }) if items.len() == 1));
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].path, Path::new("/"));
    let (op, refused) = PendingPrivOp::Rename {
        item: moving_root.clone(),
        case_only: false,
    }
    .without_mounts();
    assert!(op.is_none() && refused.len() == 1);
    // Copying FROM one is an ordinary copy.
    let (op, refused) = PendingPrivOp::Copy {
        items: vec![moving_root],
    }
    .without_mounts();
    assert!(op.is_some() && refused.is_empty());

    // Replacing one: refused before a Trash slot is claimed.
    let onto_root = crate::ops::OpItem {
        src: d.join("plain"),
        target: PathBuf::from("/"),
        replace: true,
    };
    let err = PrivItem::from_op(&onto_root).unwrap_err();
    assert!(err.reason.contains("mounted"), "{}", err.reason);

    // Deleting one.
    let (op, refused) =
        PendingPrivOp::Remove(vec![PathBuf::from("/"), d.join("plain")]).without_mounts();
    assert!(matches!(op, Some(PendingPrivOp::Remove(paths)) if paths == [d.join("plain")]));
    assert_eq!(refused.len(), 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_op_names_every_folder_it_changes() {
    let it = item(Path::new("/home/a/x"), Path::new("/etc/x"));
    let folders = |op: PendingPrivOp| -> Vec<PathBuf> {
        op.folders().into_iter().map(Path::to_path_buf).collect()
    };
    assert_eq!(
        folders(PendingPrivOp::Copy { items: vec![it.clone()] }),
        [PathBuf::from("/etc")]
    );
    // A move puts into one folder and takes from another.
    let moved = PendingPrivOp::Move { items: vec![it] };
    assert_eq!(moved.source_folders(), [Path::new("/home/a")]);
    assert_eq!(folders(moved), [PathBuf::from("/etc")]);
    assert_eq!(
        folders(PendingPrivOp::NewFile(PathBuf::from("/opt/app/New File"))),
        [PathBuf::from("/opt/app")]
    );
    assert_eq!(
        folders(PendingPrivOp::Remove(vec![PathBuf::from("/srv/a/b")])),
        [PathBuf::from("/srv/a")]
    );
}

#[test]
fn password_trouble_is_told_from_a_command_that_failed() {
    let need = classify_failure("sudo: a password is required\n", false, Some(1));
    assert!(matches!(need, Ran::NeedPassword));
    let wrong = classify_failure(
        "Sorry, try again.\nsudo: no password was provided\nsudo: 1 incorrect password attempt\n",
        true,
        Some(1),
    );
    assert!(matches!(wrong, Ran::WrongPassword));
    // The command's own failure is passed on as it is.
    match classify_failure("mkdir: cannot create directory '/x': File exists\n", true, Some(1)) {
        Ran::Failed(msg) => assert_eq!(msg, "mkdir: cannot create directory '/x': File exists"),
        _ => panic!("a command failure is not a password problem"),
    }
    match classify_failure("", false, Some(3)) {
        Ran::Failed(msg) => assert_eq!(msg, "Command failed (exit 3)"),
        _ => panic!("a silent failure is still a failure"),
    }
}

#[test]
fn a_run_resumes_where_the_ticket_ran_out_and_one_failure_does_not_stop_the_rest() {
    let commands: Vec<Vec<OsString>> = (0..4).map(|i| vec![format!("c{i}").into()]).collect();
    let mut ran: Vec<String> = Vec::new();

    // The cached ticket is good for the first command only.
    let report = run_each(&commands, 0, |argv| {
        ran.push(argv[0].to_string_lossy().into_owned());
        if ran.len() > 1 {
            Ran::NeedPassword
        } else {
            Ran::Ok
        }
    });
    assert_eq!(report.stop, Stop::NeedPassword);
    assert_eq!(report.next, 1);

    // With the password: continue at c1, never c0 again. c2 fails, c3
    // still runs.
    ran.clear();
    let report = run_each(&commands, report.next, |argv| {
        let name = argv[0].to_string_lossy().into_owned();
        ran.push(name.clone());
        if name == "c2" {
            Ran::Failed("c2 broke".into())
        } else {
            Ran::Ok
        }
    });
    assert_eq!(ran, vec!["c1", "c2", "c3"]);
    assert_eq!(report.stop, Stop::Finished);
    assert_eq!(report.next, 4);
    assert_eq!(report.errors, vec!["c2 broke"]);

    // A wrong password stops at once and runs nothing further.
    ran.clear();
    let report = run_each(&commands, 0, |argv| {
        ran.push(argv[0].to_string_lossy().into_owned());
        Ran::WrongPassword
    });
    assert_eq!((report.stop, report.next, ran.len()), (Stop::WrongPassword, 0, 1));
}

#[test]
fn only_a_permanent_delete_asks_first_and_it_says_what_goes() {
    let doomed = vec![PathBuf::from("/etc/a"), PathBuf::from("/etc/-rf")];
    let remove = PendingPrivOp::Remove(doomed.clone());
    assert!(remove.asks_first());
    assert_eq!(remove.doomed(), &doomed[..]);
    // The paths follow `--`: a name like "-rf" is a path.
    let argv = build_argv(&remove);
    assert_eq!(argv.len(), 1);
    let dashes = argv[0].iter().position(|a| a == "--").unwrap();
    assert_eq!(argv[0][..2], ["rm", "-rf"].map(OsString::from));
    // Never across into a mounted drive.
    assert!(argv[0][..dashes].contains(&OsString::from("--one-file-system")));
    assert_eq!(
        argv[0][dashes + 1..],
        doomed.iter().map(|p| p.clone().into_os_string()).collect::<Vec<_>>()[..]
    );

    for op in [
        PendingPrivOp::NewFile(PathBuf::from("/etc/x")),
        PendingPrivOp::Copy { items: Vec::new() },
        PendingPrivOp::Extract(Vec::new()),
    ] {
        assert!(!op.asks_first());
        assert!(op.doomed().is_empty());
    }
}

#[test]
fn privileged_rename_never_lands_on_another_item() {
    let d = scratch("rename");
    std::fs::write(d.join("a.conf"), b"a").unwrap();
    std::fs::write(d.join("b.conf"), b"b").unwrap();
    std::fs::create_dir_all(d.join("dir")).unwrap();
    let rename = |from: &str, to: &str| PendingPrivOp::Rename {
        item: item(&d.join(from), &d.join(to)),
        case_only: false,
    };
    // Onto an existing file: refused, both as they were.
    let argv = build_argv(&rename("a.conf", "b.conf"));
    assert_eq!(argv.len(), 1);
    assert!(!run(&argv[0]));
    assert_eq!(std::fs::read(d.join("a.conf")).unwrap(), b"a");
    assert_eq!(std::fs::read(d.join("b.conf")).unwrap(), b"b");
    // Onto an existing folder: refused, not moved inside it.
    assert!(!run(&build_argv(&rename("a.conf", "dir"))[0]));
    assert!(!d.join("dir/a.conf").exists());
    // A free name: renamed.
    assert!(run(&build_argv(&rename("a.conf", "c.conf"))[0]));
    assert_eq!(std::fs::read(d.join("c.conf")).unwrap(), b"a");
    assert!(!d.join("a.conf").exists());
    assert_eq!(
        rename("c.conf", "d.conf").description(),
        "Rename \u{201C}c.conf\u{201D} to \u{201C}d.conf\u{201D}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

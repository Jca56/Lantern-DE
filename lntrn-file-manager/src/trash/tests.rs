use super::*;

/// The raw `Path=` value of a sidecar, still percent-encoded.
fn recorded_path(info: &Path) -> String {
    std::fs::read_to_string(info)
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("Path=").map(str::to_string))
        .unwrap()
}

/// A scratch "drive": `top/` with a volume trash inside it.
fn scratch(name: &str) -> (PathBuf, TrashDir) {
    let top = std::env::temp_dir()
        .join(format!("lntrn-fm-trash-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).unwrap();
    // Records are relative to the canonical top folder.
    let top = top.canonicalize().unwrap();
    let td = TrashDir {
        dir: top.join(format!(".Trash-{}", uid())),
        top: Some(top.clone()),
    };
    (top, td)
}

fn put(td: &TrashDir, path: &Path) -> Result<Trashed, String> {
    trash_in(td, path).map_err(|e| e.reason())
}

#[test]
fn volume_trash_records_relative_paths_and_restores() {
    let (top, td) = scratch("volume");
    std::fs::create_dir_all(top.join("Docs/My Stuff")).unwrap();
    let file = top.join("Docs/My Stuff/report 100%.txt");
    std::fs::write(&file, b"hello").unwrap();

    let t = put(&td, &file).unwrap();
    assert!(!file.exists());
    assert_eq!(t.trashed, td.files().join("report 100%.txt"));
    assert_eq!(std::fs::read(&t.trashed).unwrap(), b"hello");
    // Relative to the drive's top folder, percent-encoded.
    assert_eq!(recorded_path(&t.info), "Docs/My%20Stuff/report%20100%25.txt");
    assert!(std::fs::read_to_string(&t.info).unwrap().starts_with("[Trash Info]\n"));

    // The Trash view's restore reads the record back.
    let landed = restore_item(&t.trashed).unwrap();
    assert_eq!(landed, file);
    assert_eq!(std::fs::read(&file).unwrap(), b"hello");
    assert!(!t.info.exists());
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn same_names_get_their_own_slots_and_undo_never_overwrites() {
    let (top, td) = scratch("slots");
    std::fs::create_dir_all(top.join("a")).unwrap();
    std::fs::create_dir_all(top.join("b")).unwrap();
    std::fs::write(top.join("a/note.txt"), b"first").unwrap();
    std::fs::write(top.join("b/note.txt"), b"second").unwrap();
    let t1 = put(&td, &top.join("a/note.txt")).unwrap();
    let t2 = put(&td, &top.join("b/note.txt")).unwrap();
    assert_ne!(t1.trashed, t2.trashed);
    assert_eq!(t2.trashed.file_name().unwrap(), "note.1.txt");
    assert_eq!(std::fs::read(&t1.trashed).unwrap(), b"first");
    assert_eq!(std::fs::read(&t2.trashed).unwrap(), b"second");

    // Something new took the old name: undo must leave it alone.
    std::fs::write(top.join("a/note.txt"), b"newer").unwrap();
    assert!(restore(&t1).is_err());
    assert_eq!(std::fs::read(top.join("a/note.txt")).unwrap(), b"newer");
    assert!(t1.trashed.exists() && t1.info.exists());
    // The Trash view's restore lands beside it instead.
    let landed = restore_item(&t1.trashed).unwrap();
    assert_eq!(landed, top.join("a/note (restored 1).txt"));

    // Plain undo and redo round-trip.
    restore(&t2).unwrap();
    assert_eq!(std::fs::read(top.join("b/note.txt")).unwrap(), b"second");
    assert!(!t2.info.exists());
    let t2 = put(&td, &top.join("b/note.txt")).unwrap();
    assert!(!top.join("b/note.txt").exists());
    assert_eq!(recorded_path(&t2.info), "b/note.txt");
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn a_link_is_trashed_as_a_link_and_folders_go_whole() {
    let (top, td) = scratch("links");
    std::fs::create_dir_all(top.join("real/inner")).unwrap();
    std::fs::write(top.join("real/inner/f"), b"x").unwrap();
    std::os::unix::fs::symlink(top.join("real"), top.join("link")).unwrap();
    let t = put(&td, &top.join("link")).unwrap();
    assert!(std::fs::symlink_metadata(&t.trashed).unwrap().file_type().is_symlink());
    assert!(top.join("real/inner/f").exists());
    let t = put(&td, &top.join("real")).unwrap();
    assert!(t.trashed.join("inner/f").exists());
    assert!(!top.join("real").exists());
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn locate_recognises_trash_paths_by_shape() {
    let uid = uid();
    let own = PathBuf::from(format!("/mnt/stick/.Trash-{uid}/files/Photos/a.jpg"));
    let loc = locate(&own).unwrap();
    assert_eq!(loc.trash.top.as_deref(), Some(Path::new("/mnt/stick")));
    assert_eq!(loc.rel, Path::new("Photos/a.jpg"));
    let shared = PathBuf::from(format!("/mnt/stick/.Trash/{uid}/files"));
    let loc = locate(&shared).unwrap();
    assert_eq!(loc.trash.dir, Path::new(&format!("/mnt/stick/.Trash/{uid}")));
    assert!(loc.rel.as_os_str().is_empty());
    let home = home_trash();
    assert!(locate(&home.files().join("x")).is_some_and(|l| l.trash.top.is_none()));
    // Someone else's trash, the info half, and ordinary folders are not it.
    assert!(locate(Path::new("/mnt/stick/.Trash-0/files/x")).is_none() || uid == 0);
    assert!(locate(&PathBuf::from(format!("/mnt/stick/.Trash-{uid}/info/x"))).is_none());
    assert!(locate(Path::new("/home/a/files/x")).is_none());
}

#[test]
fn a_planted_trash_folder_is_refused() {
    let (top, td) = scratch("planted");
    // `.Trash-N` is a link to somewhere else: must not be followed.
    std::fs::create_dir_all(top.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(top.join("elsewhere"), &td.dir).unwrap();
    std::fs::write(top.join("secret.txt"), b"s").unwrap();
    assert!(put(&td, &top.join("secret.txt")).is_err());
    assert!(top.join("secret.txt").exists());
    // A shared `.Trash` counts only as a real directory with the sticky bit.
    use std::os::unix::fs::PermissionsExt;
    let shared = top.join(".Trash");
    std::fs::create_dir_all(&shared).unwrap();
    assert!(!shared_trash_is_valid(&shared));
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
    assert!(shared_trash_is_valid(&shared));
    assert_eq!(volume_trashes(&top)[0].dir, shared.join(uid().to_string()));
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn trashinfo_round_trip_and_old_raw_entries() {
    let (top, _) = scratch("trashinfo");
    let info = top.join("x.trashinfo");
    // New build: encoded on write, decoded on read.
    let original = Path::new("/home/a/My Report 100%.pdf");
    std::fs::write(
        &info,
        crate::file_ops::trashinfo_contents(original, "2026-10-02T00:00:00"),
    )
    .unwrap();
    assert_eq!(read_trashinfo_path(&info, "My Report 100%.pdf").unwrap(), original);
    // Old build: written raw; "%fa" must not be decoded into a stray byte.
    std::fs::write(&info, "[Trash Info]\nPath=/home/a/25%fat.txt\nDeletionDate=x\n").unwrap();
    assert_eq!(
        read_trashinfo_path(&info, "25%fat.txt").unwrap(),
        Path::new("/home/a/25%fat.txt")
    );
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn top_dir_stays_on_the_same_filesystem() {
    let (top, _) = scratch("topdir");
    let found = top_dir(&top);
    assert!(top.starts_with(&found));
    let dev = |p: &Path| std::fs::metadata(p).unwrap().dev();
    assert_eq!(dev(&found), dev(&top));
    let _ = std::fs::remove_dir_all(&top);
}

/// The whole of `trash()` against a scratch home trash. Run it with
/// `XDG_DATA_HOME=<scratch folder under the temp dir> cargo test -- --ignored
/// real_trash`; without that it would use the real Trash, so it refuses.
#[test]
#[ignore = "needs XDG_DATA_HOME pointed at a scratch folder"]
fn real_trash_call_uses_the_home_trash_and_round_trips() {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let tmp = std::env::temp_dir();
    let Some(data) = data.filter(|d| d.starts_with(&tmp)) else {
        panic!("XDG_DATA_HOME must point below {}", tmp.display());
    };
    let work = data.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("doc.txt"), b"d").unwrap();

    let t = trash(&work.join("doc.txt")).unwrap();
    assert_eq!(t.trashed, home_trash().files().join("doc.txt"));
    // The home trash records the absolute path.
    let work_canon = work.canonicalize().unwrap();
    assert_eq!(
        read_trashinfo_path(&t.info, "doc.txt").unwrap(),
        work_canon.join("doc.txt")
    );
    assert!(all_trash_dirs().contains(&home_trash()));
    // Already in the Trash: refused, untouched.
    assert!(matches!(trash(&t.trashed), Err(TrashError::NoTrash(_))));
    assert!(t.trashed.exists());
    restore(&t).unwrap();
    assert_eq!(std::fs::read(work.join("doc.txt")).unwrap(), b"d");
    // A slot claimed for a privileged move and not used leaves nothing.
    let slot = reserve_in_home(&work.join("doc.txt")).unwrap();
    assert!(slot.info.exists());
    release(&slot);
    assert!(!slot.info.exists());
}

/// A drive's trash only ever sends things back onto that drive. A record
/// on a prepared stick can name a link that leads off it, or an absolute
/// place elsewhere: neither is followed, and no folder is made for it.
#[test]
fn a_record_cannot_send_an_item_off_its_drive() {
    let (top, td) = scratch("escape");
    let outside = top.with_extension("outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(td.files()).unwrap();
    std::fs::create_dir_all(td.info()).unwrap();
    std::os::unix::fs::symlink(&outside, top.join("lnk")).unwrap();

    let plant = |name: &str, recorded: &str| {
        std::fs::write(td.files().join(name), b"payload").unwrap();
        std::fs::write(
            td.info().join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath={recorded}\nDeletionDate=2026-10-02T00:00:00\n"),
        )
        .unwrap();
        td.files().join(name)
    };
    let absolute = format!("{}/deep/b.desktop", outside.display());
    for (name, recorded) in [
        ("a.desktop", "lnk/a.desktop"),
        ("b.desktop", absolute.as_str()),
        ("c.desktop", "../escape.outside/c.desktop"),
    ] {
        let item = plant(name, recorded);
        assert!(restore_item(&item).is_err(), "{recorded} was followed");
        assert!(item.exists(), "{name} left the trash");
    }
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "something landed outside");

    // An ordinary record still restores, folders made as needed.
    let item = plant("d.txt", "Docs/New Folder/d.txt");
    assert_eq!(restore_item(&item).unwrap(), top.join("Docs/New Folder/d.txt"));
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&top);
}

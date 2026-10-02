use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lntrn-fm-copy-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn mkfifo(path: &Path) {
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
}

#[test]
fn folder_copy_keeps_links_and_survives_bad_entries() {
    let d = scratch("links");
    let src = d.join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha").unwrap();
    std::fs::write(src.join("sub/b.txt"), b"beta").unwrap();
    std::os::unix::fs::symlink("sub", src.join("dir-link")).unwrap();
    std::os::unix::fs::symlink("a.txt", src.join("file-link")).unwrap();
    std::os::unix::fs::symlink("nowhere", src.join("dangling")).unwrap();
    mkfifo(&src.join("pipe"));
    std::fs::write(src.join("z-last.txt"), b"omega").unwrap();

    let dst = d.join("dst");
    let report = copy_item(&src, &dst, &no_cancel()).unwrap();

    // The pipe is reported and skipped; it did not block and did not
    // stop the entries around it.
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, src.join("pipe"));
    assert!(std::fs::symlink_metadata(dst.join("pipe")).is_err());
    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(dst.join("sub/b.txt")).unwrap(), b"beta");
    assert_eq!(std::fs::read(dst.join("z-last.txt")).unwrap(), b"omega");
    // Links are links, with their target text untouched.
    for (name, target) in [("dir-link", "sub"), ("file-link", "a.txt"), ("dangling", "nowhere")] {
        let p = dst.join(name);
        assert!(std::fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&p).unwrap(), Path::new(target));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn existing_destination_is_never_overwritten() {
    let d = scratch("noclobber");
    std::fs::write(d.join("new.txt"), b"new").unwrap();
    std::fs::write(d.join("old.txt"), b"old").unwrap();
    assert!(matches!(
        copy_item(&d.join("new.txt"), &d.join("old.txt"), &no_cancel()),
        Err(CopyError::Failed(_))
    ));
    assert_eq!(std::fs::read(d.join("old.txt")).unwrap(), b"old");

    // A folder is not merged into either.
    std::fs::create_dir_all(d.join("a")).unwrap();
    std::fs::write(d.join("a/x"), b"1").unwrap();
    std::fs::create_dir_all(d.join("b")).unwrap();
    std::fs::write(d.join("b/keep"), b"2").unwrap();
    assert!(copy_item(&d.join("a"), &d.join("b"), &no_cancel()).is_err());
    assert!(d.join("b/keep").exists());
    assert!(!d.join("b/x").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cancel_leaves_nothing_behind() {
    let d = scratch("cancel");
    let src = d.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..5 {
        std::fs::write(src.join(format!("f{i}")), b"data").unwrap();
    }
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        copy_item(&src, &d.join("dst"), &cancel),
        Err(CopyError::Cancelled)
    ));
    assert!(std::fs::symlink_metadata(d.join("dst")).is_err());
    // Same for a single file.
    assert!(matches!(
        copy_item(&src.join("f0"), &d.join("one"), &cancel),
        Err(CopyError::Cancelled)
    ));
    assert!(std::fs::symlink_metadata(d.join("one")).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn file_copy_is_exact_and_keeps_mode_and_mtime() {
    let d = scratch("exact");
    // Bigger than one read buffer, and not a multiple of it.
    let data: Vec<u8> = (0..3_000_017u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(d.join("big"), &data).unwrap();
    std::fs::set_permissions(d.join("big"), std::fs::Permissions::from_mode(0o640)).unwrap();
    copy_item(&d.join("big"), &d.join("copy"), &no_cancel()).unwrap();
    assert_eq!(std::fs::read(d.join("copy")).unwrap(), data);
    let (a, b) = (
        std::fs::metadata(d.join("big")).unwrap(),
        std::fs::metadata(d.join("copy")).unwrap(),
    );
    assert_eq!(b.permissions().mode() & 0o777, 0o640);
    assert_eq!(a.mtime(), b.mtime());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn moved_source_is_deleted_only_where_the_copy_is_verified() {
    let d = scratch("moved");
    let (src, dst) = (d.join("src"), d.join("dst"));
    std::fs::create_dir_all(src.join("ok")).unwrap();
    std::fs::create_dir_all(src.join("part")).unwrap();
    std::fs::write(src.join("ok/a"), b"aaaa").unwrap();
    std::fs::write(src.join("part/good"), b"gg").unwrap();
    std::fs::write(src.join("part/short"), b"full length").unwrap();
    std::fs::write(src.join("part/missing"), b"m").unwrap();
    std::fs::create_dir_all(dst.join("ok")).unwrap();
    std::fs::create_dir_all(dst.join("part")).unwrap();
    std::fs::write(dst.join("ok/a"), b"aaaa").unwrap();
    std::fs::write(dst.join("part/good"), b"gg").unwrap();
    std::fs::write(dst.join("part/short"), b"cut").unwrap();

    let mut failures = Vec::new();
    assert!(!remove_moved_source(&src, &dst, &mut failures));
    // Verified files and the folder they emptied are gone...
    assert!(!src.join("ok").exists());
    assert!(!src.join("part/good").exists());
    // ...the truncated and the missing one are still at the source.
    assert_eq!(std::fs::read(src.join("part/short")).unwrap(), b"full length");
    assert_eq!(std::fs::read(src.join("part/missing")).unwrap(), b"m");
    assert_eq!(failures.len(), 2);

    // A complete copy removes the source entirely.
    let (s2, d2) = (d.join("s2"), d.join("d2"));
    std::fs::create_dir_all(s2.join("x")).unwrap();
    std::fs::write(s2.join("x/f"), b"1").unwrap();
    copy_item(&s2, &d2, &no_cancel()).unwrap();
    let mut failures = Vec::new();
    assert!(remove_moved_source(&s2, &d2, &mut failures));
    assert!(!s2.exists() && failures.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rename_noreplace_refuses_an_existing_name() {
    let d = scratch("rename");
    std::fs::write(d.join("a"), b"a").unwrap();
    std::fs::write(d.join("b"), b"b").unwrap();
    let err = rename_noreplace(&d.join("a"), &d.join("b")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(d.join("b")).unwrap(), b"b");
    rename_noreplace(&d.join("a"), &d.join("c")).unwrap();
    assert_eq!(std::fs::read(d.join("c")).unwrap(), b"a");
    // Staging names are hidden, sit next to the target and are free.
    let t = temp_sibling(&d.join("report.pdf"));
    assert_eq!(t.parent(), Some(d.as_path()));
    assert!(t.file_name().unwrap().to_string_lossy().starts_with(".fox-part-"));
    assert!(std::fs::symlink_metadata(&t).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_staging_item_nobody_is_working_on_is_recognised_by_its_name() {
    let mine = temp_sibling(Path::new("/some/where/Holiday Photos"));
    let name = mine.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(staging_pid(&name), Some(std::process::id()));
    // This process is running: its own staging items are in use.
    assert!(!is_stale_staging(&name));
    // A process that does not exist (pids stop far below this), or one that
    // is not a Fox: stale.
    assert!(is_stale_staging(".fox-part-4294967294-0-film.mkv.fox-tmp"));
    assert!(is_stale_staging(".fox-part-1-3-a-b-c.fox-tmp"), "pid 1 is not Fox");
    // Anything else is not a staging item at all.
    for other in [
        "film.mkv",
        ".fox-part-notes.txt",
        ".fox-part-12-x-name.fox-tmp",
        ".fox-part-12-0-name",
        "photo.jpg.fox-tmp",
    ] {
        assert!(!is_stale_staging(other), "{other}");
    }
}

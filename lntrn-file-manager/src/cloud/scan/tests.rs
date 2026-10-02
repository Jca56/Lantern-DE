use super::super::TestDir;
use super::*;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;

fn scratch(name: &str) -> TestDir {
    TestDir::new(&format!("scan-{name}"))
}

fn manifest() -> Manifest {
    Manifest::default()
}

#[test]
fn a_missing_root_is_an_error_and_is_not_created() {
    let dir = scratch("missing");
    let root = dir.join("Cloud");
    assert_eq!(scan_local(&root, &manifest(), Duration::ZERO).unwrap_err(), RootError::Missing);
    assert!(!root.exists());
    // A plain file where the folder should be is not a sync root either.
    std::fs::write(&root, b"x").unwrap();
    assert_eq!(scan_local(&root, &manifest(), Duration::ZERO).unwrap_err(), RootError::Missing);
}

#[test]
fn finds_files_and_skips_ignored_ones() {
    let root = scratch("plain");
    std::fs::create_dir_all(root.join("a/.git")).unwrap();
    std::fs::write(root.join("top.txt"), b"1").unwrap();
    std::fs::write(root.join("a/in.txt"), b"22").unwrap();
    std::fs::write(root.join("a/.git/config"), b"x").unwrap();
    std::fs::write(root.join("a/part.fox-tmp"), b"x").unwrap();
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    let mut keys: Vec<&str> = scan.files.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["a/in.txt", "top.txt"]);
    assert_eq!(scan.unknown.failures, 0);
    assert_eq!(scan.files["a/in.txt"].size, 2);
    assert_eq!(scan.files["a/in.txt"].sha, super::super::hash::sha256_bytes(b"22"));
}

#[test]
fn an_unreadable_folder_is_unknown_not_empty() {
    // Root can read anything; the test would prove nothing.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = scratch("locked");
    std::fs::create_dir_all(root.join("ok")).unwrap();
    std::fs::create_dir_all(root.join("locked/deep")).unwrap();
    std::fs::write(root.join("ok/a.txt"), b"a").unwrap();
    std::fs::write(root.join("locked/deep/b.txt"), b"b").unwrap();
    std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000))
        .unwrap();
    let scan = scan_local(&root, &manifest(), Duration::ZERO);
    std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o755))
        .unwrap();
    let scan = scan.unwrap();
    assert!(scan.files.contains_key("ok/a.txt"));
    assert_eq!(scan.unknown.failures, 1);
    assert!(scan.unknown.covers("locked/deep/b.txt"));
    assert_eq!(scan.unreadable.len(), 1);
    assert_eq!(scan.unreadable[0].0, "locked");
    assert!(scan.unknown.covers("locked"));
    assert!(!scan.unknown.covers("locked-not/x"));
    assert!(!scan.unknown.covers("ok/gone.txt"));
    assert_eq!(scan.notes.len(), 1);
}

#[test]
fn an_unreadable_file_is_unknown() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = scratch("nofile");
    std::fs::write(root.join("secret.txt"), b"s").unwrap();
    std::fs::set_permissions(root.join("secret.txt"), std::fs::Permissions::from_mode(0o000))
        .unwrap();
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    assert!(scan.files.is_empty());
    assert!(scan.unknown.covers("secret.txt"));
    assert_eq!(scan.unknown.failures, 1);
}

#[test]
fn names_are_never_rewritten() {
    let root = scratch("names");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a\\b.txt"), b"backslash").unwrap();
    let bad = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    std::fs::write(root.join(bad), b"latin1").unwrap();
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();

    // The backslash stays a backslash...
    assert!(scan.files.contains_key("a\\b.txt"));
    assert_eq!(scan.files.len(), 1);
    // ...and the key an older build made up for it is shielded, so a
    // manifest entry under it can never read as "deleted locally".
    assert!(scan.unknown.covers("a/b.txt"));

    // The non-UTF-8 name is skipped, reported, and its lossy key shielded.
    assert!(scan.unknown.covers("caf\u{fffd}.txt"));
    assert_eq!(scan.notes.len(), 1);
    // Neither is a read failure: the scan is complete.
    assert_eq!(scan.unknown.failures, 0);
}

#[test]
fn a_real_file_at_the_legacy_key_is_not_shielded() {
    let root = scratch("legacy");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a\\b.txt"), b"1").unwrap();
    std::fs::write(root.join("a/b.txt"), b"2").unwrap();
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    assert_eq!(scan.files.len(), 2);
    assert!(!scan.unknown.covers("a/b.txt"));
}

#[test]
fn a_file_still_being_written_is_left_for_later() {
    let root = scratch("busy");
    std::fs::write(root.join("copying.bin"), b"first part").unwrap();
    std::fs::write(root.join("settled.txt"), b"done").unwrap();
    // settled.txt was last written a minute ago.
    let c_path = std::ffi::CString::new(root.join("settled.txt").as_os_str().as_bytes()).unwrap();
    let minute_ago = now_secs() as libc::time_t - 60;
    let times = [
        libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_OMIT },
        libc::timespec { tv_sec: minute_ago, tv_nsec: 0 },
    ];
    assert_eq!(
        unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) },
        0
    );

    let scan = scan_local(&root, &manifest(), Duration::from_secs(10)).unwrap();
    assert_eq!(scan.busy, 1);
    assert!(scan.files.contains_key("settled.txt"));
    assert!(!scan.files.contains_key("copying.bin"));
    // Not a deletion, not a read failure: just not looked at yet.
    assert!(scan.unknown.covers("copying.bin"));
    assert_eq!(scan.unknown.failures, 0);

    // With no window everything is hashed.
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    assert_eq!((scan.busy, scan.files.len()), (0, 2));
}

#[test]
fn a_file_too_big_for_the_cloud_is_set_aside_unread() {
    let root = scratch("toobig");
    std::fs::write(root.join("small.txt"), b"s").unwrap();
    // Sparse: a gigabyte long, no blocks on disk. Hashing it would take
    // seconds; the scan must not even open it.
    let big = std::fs::File::create(root.join("movie.mkv")).unwrap();
    big.set_len(MAX_BLOB_BYTES).unwrap();
    let almost = std::fs::File::create(root.join("almost.bin")).unwrap();
    almost.set_len(4096).unwrap();

    let started = std::time::Instant::now();
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(scan.too_big, ["movie.mkv"]);
    assert!(scan.unknown.covers("movie.mkv"));
    assert_eq!(scan.unknown.failures, 0);
    assert_eq!(scan.files.len(), 2);
    assert_eq!(scan.notes.len(), 1);
}

#[test]
fn notices_a_change_after_the_scan() {
    let root = scratch("recheck");
    let path = root.join("doc.txt");
    std::fs::write(&path, b"first").unwrap();
    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    let snap = &scan.files["doc.txt"];
    assert!(unchanged_since_scan(Some(snap), &path));

    // Nothing there at scan time, nothing there now.
    let other = root.join("new.txt");
    assert!(unchanged_since_scan(None, &other));
    std::fs::write(&other, b"appeared").unwrap();
    assert!(!unchanged_since_scan(None, &other));

    // Same size, same mtime, different bytes: only the hash can tell.
    let times = [
        libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_OMIT },
        libc::timespec {
            tv_sec: (snap.mtime_ns / 1_000_000_000) as libc::time_t,
            tv_nsec: (snap.mtime_ns % 1_000_000_000) as libc::c_long,
        },
    ];
    {
        use std::io::{Seek, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.seek(std::io::SeekFrom::Start(0)).unwrap();
        f.write_all(b"FIRST").unwrap();
    }
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
    assert!(!unchanged_since_scan(Some(snap), &path));

    // Replaced by a save (new inode, new stamp), then removed.
    std::fs::write(root.join("doc.tmp"), b"second version").unwrap();
    std::fs::rename(root.join("doc.tmp"), &path).unwrap();
    assert!(!unchanged_since_scan(Some(snap), &path));
    std::fs::remove_file(&path).unwrap();
    assert!(!unchanged_since_scan(Some(snap), &path));
}

#[test]
fn a_link_is_not_synced_and_nothing_below_it_reads_as_gone() {
    let dir = scratch("link");
    let root = dir.join("Cloud");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
    std::fs::write(dir.join("elsewhere/song.mp3"), b"la").unwrap();
    std::fs::write(root.join("docs/a.txt"), b"a").unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere"), root.join("Music")).unwrap();
    std::os::unix::fs::symlink(root.join("docs/a.txt"), root.join("alias.txt")).unwrap();

    let scan = scan_local(&root, &manifest(), Duration::ZERO).unwrap();
    let keys: Vec<&str> = scan.files.keys().map(String::as_str).collect();
    assert_eq!(keys, ["docs/a.txt"]);
    // Unknown, so a pass neither downloads through the link nor reads what
    // was synced below that name as deleted...
    assert!(scan.unknown.covers("Music"));
    assert!(scan.unknown.covers("Music/song.mp3"));
    assert!(scan.unknown.covers("alias.txt"));
    assert!(!scan.unknown.covers("docs/a.txt"));
    // ...but not a read failure: it holds no deletion back elsewhere.
    assert_eq!(scan.unknown.failures, 0);
    assert!(scan.notes.iter().any(|n| n.contains("symbolic link")));
}

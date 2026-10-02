// Merging, files that change while a pass is running, and odd file names.

use super::super::manifest::Manifest;
use super::super::remote_index::RemoteIndex;
use super::{cloud, synced_pair, Machine};

#[test]
fn first_sync_merges_both_sides() {
    let cloud = cloud("alice");
    let mut a = Machine::new("first", "a", &cloud);
    let mut b = Machine::new("first", "b", &cloud);
    a.write("only-a.txt", b"a");
    a.write("both.txt", b"same");
    b.write("only-b.txt", b"b");
    b.write("both.txt", b"same");
    a.sync();
    b.sync();
    a.sync();
    assert_eq!(a.files(), ["both.txt", "only-a.txt", "only-b.txt"]);
    assert_eq!(a.files(), b.files());
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!((a.trashed(), b.trashed()), (0, 0));
}

#[test]
fn own_writes_are_not_read_back_to_decide() {
    let cloud = cloud("alice");
    let mut a = Machine::new("own", "a", &cloud);
    a.write("a.txt", b"one");
    a.sync();
    assert_eq!(cloud.uploads(), 1);
    // No fetch: the mirror must already know what this machine just wrote.
    let r = a.try_pass_with(&mut || false).ok().unwrap();
    assert!(r.failures.is_empty());
    assert_eq!((cloud.uploads(), cloud.downloads()), (1, 0));
    assert_eq!(a.read("a.txt").unwrap(), b"one");
}

#[test]
fn a_download_does_not_replace_a_file_saved_during_the_pass() {
    let (cloud, mut a, mut b) = synced_pair("midpass", 3);
    b.write("docs/f00.txt", b"edited on b");
    b.sync();

    // While the blob travels, the user saves the same file on machine a.
    let target = a.path("docs/f00.txt");
    cloud.on_download(Some(Box::new(move || {
        std::fs::write(&target, b"edited on a, mid-pass").unwrap();
    })));
    let r = a.sync();
    cloud.on_download(None);
    assert_eq!(r.deferred, 1);
    assert!(r.failures.is_empty());
    assert_eq!(a.read("docs/f00.txt").unwrap(), b"edited on a, mid-pass");
    assert!(!a.files().iter().any(|f| f.ends_with(".fox-tmp")));

    // The next pass sees two diverged versions and keeps both.
    a.sync();
    assert_eq!(a.read("docs/f00.txt").unwrap(), b"edited on b");
    let copy = a
        .files()
        .into_iter()
        .find(|f| f.contains("conflict from"))
        .expect("a conflict copy");
    assert_eq!(a.read(&copy).unwrap(), b"edited on a, mid-pass");
}

#[test]
fn a_remote_deletion_spares_a_file_edited_after_the_scan() {
    let (cloud, mut a, mut b) = synced_pair("spare", 20);
    std::fs::remove_file(b.path("docs/f05.txt")).unwrap();
    b.sync();
    assert_eq!(cloud.tombstones(), 1);

    // The scan has run; before the pass gets to act, the file is saved.
    a.fetch();
    let target = a.path("docs/f05.txt");
    let mut edited = false;
    let r = a
        .try_pass_with(&mut || {
            if !edited {
                std::fs::write(&target, b"saved just now").unwrap();
                edited = true;
            }
            false
        })
        .ok()
        .unwrap();
    assert_eq!(r.deferred, 1);
    assert_eq!(a.read("docs/f05.txt").unwrap(), b"saved just now");
    assert_eq!(a.trashed(), 0);

    // Next pass: edited here, deleted there. The edit wins and is uploaded.
    a.sync();
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!(a.read("docs/f05.txt").unwrap(), b"saved just now");
    assert_eq!(a.trashed(), 0);
}

#[test]
fn odd_names_are_synced_verbatim_or_not_at_all() {
    use std::os::unix::ffi::OsStrExt;
    let cloud = cloud("alice");
    let mut a = Machine::new("names", "a", &cloud);
    let mut b = Machine::new("names", "b", &cloud);
    a.write("a\\b.txt", b"backslash");
    std::fs::write(
        a.places.root.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt")),
        b"latin1",
    )
    .unwrap();
    let r = a.sync();
    assert_eq!(cloud.live(), ["a\\b.txt"]);
    assert_eq!(r.notes.len(), 1);
    assert_eq!(r.unreadable, 0);

    // The other machine gets one file with a backslash in its name, not a
    // folder "a" with "b.txt" in it.
    b.sync();
    assert_eq!(b.files(), ["a\\b.txt"]);
}

#[test]
fn a_folder_in_the_way_is_a_failure_not_a_retry_loop() {
    let cloud = cloud("alice");
    let mut a = Machine::new("inway", "a", &cloud);
    let mut b = Machine::new("inway", "b", &cloud);
    b.write("notes", b"now a file");
    b.sync();
    std::fs::create_dir_all(a.path("notes")).unwrap();

    let r = a.sync();
    assert_eq!(r.failures.len(), 1);
    assert_eq!(r.deferred, 0);
    assert_eq!(cloud.downloads(), 0);
    assert!(a.path("notes").is_dir());
    assert!(a.files().is_empty());
}

#[test]
fn a_folder_that_vanishes_mid_pass_is_not_recreated() {
    let cloud = cloud("alice");
    let mut a = Machine::new("vanish", "a", &cloud);
    let mut b = Machine::new("vanish", "b", &cloud);
    b.write("deep/er/file.txt", b"x");
    b.sync();

    let root = a.places.root.clone();
    cloud.on_download(Some(Box::new(move || {
        let _ = std::fs::remove_dir_all(&root);
    })));
    let r = a.sync();
    cloud.on_download(None);
    assert_eq!(r.failures.len(), 1);
    assert!(!a.places.root.exists());
}

#[test]
fn a_cancelled_pass_keeps_what_it_did() {
    let cloud = cloud("alice");
    let mut a = Machine::new("cancel", "a", &cloud);
    for i in 0..5 {
        a.write(&format!("f{i}.txt"), format!("{i}").as_bytes());
    }
    a.fetch();
    let mut asked = 0;
    let r = a
        .try_pass_with(&mut || {
            asked += 1;
            asked > 2
        })
        .ok()
        .unwrap();
    assert!(r.cancelled);
    assert_eq!(cloud.uploads(), 2);
    let (m, _) = Manifest::load_from(&a.places.manifest, "alice");
    assert_eq!(m.entries.len(), 2);
    // The mirror on disk has them too, so a restart does not see "remote
    // changed" for files this machine uploaded itself.
    let idx = RemoteIndex::load_from(&a.places.index, "alice");
    assert_eq!(idx.docs.len(), 2);

    let r = a.sync();
    assert!(!r.cancelled);
    assert_eq!(cloud.uploads(), 5);
}

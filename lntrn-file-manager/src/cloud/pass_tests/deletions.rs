// Deletions: the one thing sync can do that destroys files on the other
// machine. Every way a file can merely LOOK deleted has a scenario here.

use super::super::guard::HoldReason;
use super::super::hash::sha256_bytes;
use super::super::manifest::{Manifest, Origin};
use super::super::reconcile::PassError;
use super::super::remote_index::RemoteIndex;
use super::super::scan::RootError;
use super::super::firestore::Expect;
use super::super::store::Store;
use super::{cloud, synced_pair, Machine};

#[test]
fn a_missing_folder_blocks_the_pass_and_is_not_created() {
    let (cloud, mut a, _b) = synced_pair("missing", 12);
    std::fs::rename(&a.places.root, a.places.root.with_extension("bak")).unwrap();
    a.fetch();
    let r = a.try_pass_with(&mut || false);
    assert!(matches!(r, Err(PassError::Root(RootError::Missing))));
    assert!(!a.places.root.exists());
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!(cloud.live().len(), 12);
}

#[test]
fn an_emptied_folder_is_held_and_heals_when_the_files_return() {
    let (cloud, mut a, mut b) = synced_pair("emptied", 12);
    // The folder is there but empty: an unmounted volume, a fresh install
    // with a copied state directory, or a real "delete everything".
    let aside = a.places.root.with_extension("aside");
    std::fs::rename(a.path("docs"), &aside).unwrap();
    let r = a.sync();
    let held = r.held.expect("deletions must be held");
    assert_eq!((held.paths.len(), held.total), (12, 12));
    assert_eq!(held.reason, HoldReason::EmptyFolder);
    assert_eq!(cloud.tombstones(), 0);

    // The other machine sees nothing of it.
    b.sync();
    assert_eq!(b.files().len(), 12);
    assert_eq!(b.trashed(), 0);

    // The volume comes back: the hold is gone, nothing was deleted anywhere.
    std::fs::rename(&aside, a.path("docs")).unwrap();
    let r = a.sync();
    assert!(r.held.is_none());
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!(cloud.uploads(), 12);
}

/// The folder is an empty stand-in while the other machine edits and adds
/// files. Nothing may be done with the stand-in until the user has answered:
/// a download into it would move pivots, and the real folder's older copies
/// would then be uploaded over the other machine's edits when it returns.
#[test]
fn a_stand_in_folder_is_left_alone_until_the_question_is_answered() {
    let (cloud, mut a, mut b) = synced_pair("standin", 8);
    b.write("docs/f03.txt", b"edited on b");
    b.write("docs/new-on-b.txt", b"made on b");
    b.sync();

    let aside = a.places.root.with_extension("aside");
    std::fs::rename(a.path("docs"), &aside).unwrap();
    // A stray file in the stand-in does not make it the synced folder.
    a.write("stray.txt", b"dropped in by accident");
    let uploads = cloud.uploads();
    for _ in 0..2 {
        let r = a.sync();
        let held = r.held.expect("held on every pass, not only the first");
        assert_eq!(held.reason, HoldReason::EmptyFolder);
        // (The file b edited is one to download, not one to delete.)
        assert_eq!((held.paths.len(), held.total), (7, 8));
    }
    // Eight files or fewer used to go out unasked on the second pass.
    assert_eq!(cloud.tombstones(), 0);
    // Nothing was downloaded into the stand-in, nothing uploaded from it.
    assert_eq!(a.files(), vec!["stray.txt".to_string()]);
    assert_eq!(cloud.uploads(), uploads);

    // The real folder is back: b's edit arrives, a's old copy is not sent
    // over it, and b's new file is not read as deleted.
    std::fs::rename(&aside, a.path("docs")).unwrap();
    let r = a.sync();
    assert!(r.held.is_none() && r.failures.is_empty());
    assert_eq!(a.read("docs/f03.txt").unwrap(), b"edited on b");
    assert_eq!(a.read("docs/new-on-b.txt").unwrap(), b"made on b");
    assert_eq!(cloud.tombstones(), 0);
    b.sync();
    assert_eq!(b.read("docs/f03.txt").unwrap(), b"edited on b");
    assert_eq!(b.trashed(), 0);
}

#[test]
fn confirmed_deletions_go_out_once_and_land_in_the_other_trash() {
    let (cloud, mut a, mut b) = synced_pair("confirm", 12);
    std::fs::remove_dir_all(a.path("docs")).unwrap();
    let r = a.sync();
    assert_eq!(cloud.tombstones(), 0);

    a.confirm(&r);
    let r = a.sync();
    assert!(r.held.is_none());
    assert_eq!(cloud.tombstones(), 12);
    assert!(cloud.live().is_empty());

    // On the other machine the files leave ~/Cloud for the Trash; they are
    // not unlinked.
    let r = b.sync();
    assert!(r.failures.is_empty());
    assert!(b.files().is_empty());
    assert_eq!(b.trashed(), 12);
    assert!(b.places.trash.join("info/f00.txt.trashinfo").exists());
}

#[test]
fn declined_deletions_are_downloaded_again() {
    let (cloud, mut a, _b) = synced_pair("decline", 12);
    std::fs::remove_dir_all(a.path("docs")).unwrap();
    let r = a.sync();
    assert!(r.held.is_some());

    a.decline(&r);
    let r = a.sync();
    assert!(r.held.is_none() && r.failures.is_empty());
    assert_eq!(a.files().len(), 12);
    assert_eq!(a.read("docs/f03.txt").unwrap(), b"content 3");
    assert_eq!(cloud.tombstones(), 0);
}

#[test]
fn a_few_deletions_need_no_question() {
    let (cloud, mut a, mut b) = synced_pair("few", 20);
    std::fs::remove_file(a.path("docs/f01.txt")).unwrap();
    std::fs::remove_file(a.path("docs/f02.txt")).unwrap();
    let r = a.sync();
    assert!(r.held.is_none());
    assert_eq!(cloud.tombstones(), 2);
    b.sync();
    assert_eq!(b.files().len(), 18);
    assert_eq!(b.trashed(), 2);
}

#[test]
fn an_unreadable_folder_is_not_a_deleted_folder() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let cloud = cloud("alice");
    let mut a = Machine::new("locked", "a", &cloud);
    for i in 0..6 {
        a.write(&format!("open/o{i}.txt"), b"o");
        a.write(&format!("locked/l{i}.txt"), b"l");
    }
    a.sync();
    assert_eq!(cloud.live().len(), 12);

    std::fs::remove_file(a.path("open/o0.txt")).unwrap();
    let locked = a.path("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let r = a.sync();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(r.unreadable, 1);
    // It is named, for "these cannot be synced".
    assert_eq!(r.stuck.len(), 1);
    assert_eq!(
        (r.stuck[0].path.as_str(), r.stuck[0].kind),
        ("locked", super::super::failures::StuckKind::Unreadable)
    );
    // Nothing under the unreadable folder is touched, and with the scan
    // incomplete not even the one real deletion goes out unasked.
    assert_eq!(cloud.tombstones(), 0);
    let held = r.held.unwrap();
    assert_eq!(held.paths, ["open/o0.txt"]);
    assert_eq!(held.reason, HoldReason::ScanFailed);

    // Readable again: one honest deletion out of twelve files.
    let r = a.sync();
    assert!(r.held.is_none());
    assert_eq!(cloud.tombstones(), 1);
    assert_eq!(cloud.live().len(), 11);
}

#[test]
fn a_corrupt_manifest_is_set_aside_and_deletes_nothing() {
    let (cloud, mut a, mut b) = synced_pair("corrupt", 6);
    // Deleted on b (propagated), and deleted locally on a (not yet synced).
    std::fs::remove_file(b.path("docs/f00.txt")).unwrap();
    b.sync();
    std::fs::remove_file(a.path("docs/f01.txt")).unwrap();
    std::fs::write(&a.places.manifest, b"{\"uid\":\"alice\",\"entr").unwrap();

    let r = a.sync();
    assert!(r.failures.is_empty());
    assert!(a.places.manifest.with_extension("json.corrupt").exists());
    // Without pivots nothing can be told apart from "new": nothing is
    // deleted on either side. f00 is revived, f01 comes back.
    assert_eq!(a.trashed(), 0);
    assert_eq!(a.files().len(), 6);
    assert_eq!(cloud.tombstones(), 0);
    // And the manifest was rebuilt for the next pass.
    let (m, origin) = Manifest::load_from(&a.places.manifest, "alice");
    assert_eq!(origin, Origin::Loaded);
    assert_eq!(m.entries.len(), 6);
}

#[test]
fn another_accounts_pivots_are_not_applied_to_this_account() {
    let alice = cloud("alice");
    let mut a = Machine::new("accounts", "a", &alice);
    a.write("notes.txt", b"alice's notes");
    a.write("gone-in-bob.txt", b"x");
    a.sync();

    // Sign out, sign in as bob: same ~/Cloud, same state files, bob's cloud.
    let bob = cloud("bob");
    let mut elsewhere = Machine::new("accounts", "bobs-other-machine", &bob);
    elsewhere.write("notes.txt", b"bob's notes");
    elsewhere.write("gone-in-bob.txt", b"x");
    elsewhere.write("pad1.txt", b"1");
    elsewhere.write("pad2.txt", b"2");
    elsewhere.sync();
    std::fs::remove_file(elsewhere.path("gone-in-bob.txt")).unwrap();
    elsewhere.sync();
    assert_eq!(bob.tombstones(), 1);

    a.cloud = bob.clone();
    a.index = RemoteIndex::load_from(&a.places.index, "bob");
    assert!(!a.index.is_seeded());
    let r = a.sync();
    assert!(r.failures.is_empty());
    assert!(a.places.manifest.with_extension("json.other-account").exists());
    // With alice's pivots this was "remote changed only" (overwrite) and
    // "deleted remotely, unchanged here" (delete). Now: both versions kept,
    // nothing removed.
    assert_eq!(a.read("notes.txt").unwrap(), b"bob's notes");
    let copy = a.files().into_iter().find(|f| f.contains("conflict from")).unwrap();
    assert_eq!(a.read(&copy).unwrap(), b"alice's notes");
    assert_eq!(a.read("gone-in-bob.txt").unwrap(), b"x");
    assert_eq!(a.trashed(), 0);
}

#[test]
fn a_path_once_keyed_lossily_is_not_read_as_deleted() {
    let cloud = cloud("alice");
    let mut a = Machine::new("lossy", "a", &cloud);
    for i in 0..12 {
        a.write(&format!("pad{i}.txt"), b"p");
    }
    a.write("a\\b.txt", b"backslash");
    a.sync();
    // What an earlier build left behind: the same file under the key
    // "a/b.txt", in the cloud and in the manifest.
    let sha = sha256_bytes(b"backslash");
    let mut legacy = cloud.doc("a\\b.txt");
    legacy.path = "a/b.txt".to_string();
    cloud.put_doc(&legacy, &Expect::Absent).unwrap();
    let (mut m, _) = Manifest::load_from(&a.places.manifest, "alice");
    m.set("a/b.txt".to_string(), sha);
    m.save_to(&a.places.manifest).unwrap();

    let r = a.sync();
    assert!(r.held.is_none());
    assert_eq!(cloud.tombstones(), 0);
    assert!(!a.path("a").exists());
}

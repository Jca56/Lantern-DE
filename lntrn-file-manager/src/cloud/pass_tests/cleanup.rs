// What sync removes to keep the cloud and the folder tidy: tombstones past
// their keep time, blobs nothing refers to, folders its own deletions
// emptied. Each of them must not take anything that is still needed.

use std::sync::atomic::Ordering::SeqCst;

use super::super::gc::{ORPHAN_GRACE, TOMBSTONE_KEEP};
use super::super::hash::sha256_bytes;
use super::super::stamp::{DAY, HOUR};
use super::{cloud, synced_pair, Machine};

#[test]
fn tombstones_are_removed_after_their_keep_time_and_not_before() {
    let (cloud, mut a, mut b) = synced_pair("purge", 20);
    std::fs::remove_file(a.path("docs/f01.txt")).unwrap();
    std::fs::remove_file(a.path("docs/f02.txt")).unwrap();
    a.sync();
    b.sync();
    assert_eq!(cloud.tombstones(), 2);

    cloud.advance(TOMBSTONE_KEEP - DAY);
    let r = a.sync();
    assert_eq!(r.cleaned.tombstones, 0);
    assert_eq!(cloud.tombstones(), 2);

    cloud.advance(2 * DAY);
    let r = a.sync();
    assert!(r.failures.is_empty());
    assert_eq!(r.cleaned.tombstones, 2);
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!(cloud.docs.lock().unwrap().len(), 18);

    // The other machine, which had seen them, is none the wiser and none
    // the worse: nothing comes back, nothing is deleted.
    let r = b.sync();
    assert!(r.failures.is_empty());
    assert_eq!((b.files().len(), a.files().len()), (18, 18));
    assert_eq!(cloud.live().len(), 18);
}

#[test]
fn a_tombstone_this_machine_has_not_acted_on_is_kept() {
    let (cloud, mut a, mut b) = synced_pair("purge-pending", 20);
    std::fs::remove_file(a.path("docs/f01.txt")).unwrap();
    a.sync();
    // b cannot carry the deletion out: its Trash is not usable.
    std::fs::write(&b.places.trash, b"not a directory").unwrap();
    let r = b.sync();
    assert_eq!(r.failures.len(), 1);
    assert!(b.path("docs/f01.txt").exists());

    // Long after, with b's copy still there, b's clean-up must leave the
    // tombstone: without it the file would read as new and be uploaded,
    // undoing the deletion for everyone.
    cloud.advance(TOMBSTONE_KEEP + DAY);
    b.retry_failed();
    let r = b.sync();
    assert_eq!(r.cleaned.tombstones, 0);
    assert_eq!(cloud.tombstones(), 1);
    assert_eq!(cloud.live().len(), 19);
    assert!(b.path("docs/f01.txt").exists());
}

#[test]
fn a_blob_nothing_refers_to_is_deleted_after_a_day_and_not_before() {
    let cloud = cloud("alice");
    let mut a = Machine::new("orphans", "a", &cloud);
    a.write("a.txt", b"version one");
    a.write("twin-1.txt", b"same content");
    a.write("twin-2.txt", b"same content");
    a.sync();
    a.write("a.txt", b"version two");
    std::fs::remove_file(a.path("twin-1.txt")).unwrap();
    a.sync();
    let (old, new) = (sha256_bytes(b"version one"), sha256_bytes(b"version two"));
    let twin = sha256_bytes(b"same content");
    assert_eq!(cloud.blob_count(), 3);

    cloud.advance(ORPHAN_GRACE - HOUR);
    let r = a.sync();
    assert_eq!(r.cleaned.blobs, 0);
    assert!(cloud.blob(&old).is_some());

    cloud.advance(2 * HOUR);
    let r = a.sync();
    assert!(r.failures.is_empty());
    assert_eq!(r.cleaned.blobs, 1);
    assert_eq!(cloud.blob(&old), None);
    // The current version stays, and so does the content one of the twins
    // still has.
    assert!(cloud.blob(&new).is_some() && cloud.blob(&twin).is_some());
}

#[test]
fn a_blob_the_mirror_missed_a_doc_for_is_kept() {
    let (cloud, mut a, mut b) = synced_pair("orphan-race", 3);
    // a's mirror is read; then b adds a file a's mirror knows nothing of.
    let server_time = a.fetch();
    b.write("late.txt", b"written after a looked");
    b.poll();
    let late = sha256_bytes(b"written after a looked");
    // Old enough to go, as far as its age is concerned.
    cloud.advance(ORPHAN_GRACE + HOUR);

    let r = a.pass(Some(server_time + ORPHAN_GRACE + HOUR));
    assert_eq!(r.cleaned.blobs, 0);
    assert!(cloud.blob(&late).is_some());
}

#[test]
fn a_cloud_that_does_not_allow_deletes_yet_is_not_an_error() {
    let cloud = cloud("alice");
    let mut a = Machine::new("norule", "a", &cloud);
    a.write("a.txt", b"version one");
    a.sync();
    a.write("a.txt", b"version two");
    a.sync();
    cloud.advance(ORPHAN_GRACE + HOUR);
    cloud.deletes_forbidden.store(true, SeqCst);

    let r = a.sync();
    assert!(r.failures.is_empty() && r.stuck.is_empty() && !r.quota);
    assert_eq!(r.cleaned.blobs, 0);
    assert_eq!(cloud.blob_count(), 2);

    // The rule is published: the next clean-up does it.
    cloud.deletes_forbidden.store(false, SeqCst);
    assert_eq!(a.sync().cleaned.blobs, 1);
}

#[test]
fn a_blob_that_went_missing_is_stored_again_by_whoever_has_the_file() {
    let (cloud, mut a, mut b) = synced_pair("restore", 3);
    let sha = sha256_bytes(b"content 1");
    cloud.lose_blob(&sha);
    // b has the file too, with exactly that content.
    let r = b.sync();
    assert!(r.failures.is_empty());
    assert_eq!(r.cleaned.restored, 1);
    assert_eq!(cloud.blob(&sha).unwrap(), b"content 1");

    // A listing that shows nothing at all is not believed: nothing is
    // re-sent on its word, and nothing is deleted either.
    cloud.listing_broken.store(true, SeqCst);
    let uploads = cloud.uploads();
    let r = a.sync();
    assert_eq!(r.cleaned, Default::default());
    assert_eq!(cloud.uploads(), uploads);
    assert_eq!(cloud.blob_count(), 3);
}

#[test]
fn a_folder_deleted_there_does_not_leave_its_skeleton_here() {
    let (_cloud, mut a, mut b) = synced_pair("skeleton", 20);
    a.write("project/src/main.rs", b"fn main() {}");
    a.write("project/src/deep/er/mod.rs", b"mod x;");
    a.write("project/readme.md", b"read me");
    a.sync();
    b.sync();
    assert!(b.path("project/src/deep/er").is_dir());

    std::fs::remove_dir_all(a.path("project")).unwrap();
    let r = a.sync();
    assert!(r.held.is_none());
    let r = b.sync();
    assert!(r.failures.is_empty());
    // The files went to the Trash, and the folders that held nothing else
    // went with them.
    assert_eq!(b.trashed(), 3);
    assert!(!b.path("project").exists());
    // Folders that still hold files, and the root, are untouched.
    assert_eq!(b.files().len(), 20);
    assert!(b.places.root.is_dir());
}

#[test]
fn a_renamed_folder_moves_instead_of_doubling() {
    let (_cloud, mut a, mut b) = synced_pair("rename", 5);
    // (A second synced folder: a library whose every synced file vanishes
    // at once is asked about, see deletions.rs.)
    a.write("other/keep.txt", b"keep");
    a.sync();
    b.sync();
    std::fs::rename(a.path("docs"), a.path("papers")).unwrap();
    let r = a.sync();
    assert!(r.held.is_none() && r.failures.is_empty());
    let r = b.sync();
    assert!(r.failures.is_empty());
    assert_eq!(b.files(), a.files());
    assert!(b.path("papers").is_dir());
    assert!(!b.path("docs").exists());
}

#[test]
fn a_folder_with_something_else_in_it_stays() {
    let (_cloud, mut a, mut b) = synced_pair("keepdir", 20);
    a.write("shared/synced.txt", b"s");
    a.sync();
    b.sync();
    // On b the folder also holds a file sync leaves alone, and a subfolder.
    b.write("shared/notes.txt~", b"editor backup");
    std::fs::create_dir_all(b.path("shared/mine")).unwrap();

    std::fs::remove_file(a.path("shared/synced.txt")).unwrap();
    a.sync();
    b.sync();
    assert!(!b.path("shared/synced.txt").exists());
    assert!(b.path("shared/notes.txt~").exists());
    assert!(b.path("shared/mine").is_dir());
}

#[test]
fn a_folder_replaced_by_a_file_arrives_in_one_pass() {
    let (_cloud, mut a, mut b) = synced_pair("dir-to-file", 20);
    a.write("notes/one.txt", b"1");
    a.write("notes/two.txt", b"2");
    a.sync();
    b.sync();

    std::fs::remove_dir_all(a.path("notes")).unwrap();
    a.write("notes", b"now a single file");
    a.sync();
    let r = b.sync();
    assert!(r.failures.is_empty() && r.stuck.is_empty());
    assert_eq!(b.read("notes").unwrap(), b"now a single file");

    // And back: a file replaced by a folder of that name.
    std::fs::remove_file(a.path("notes")).unwrap();
    a.write("notes/three.txt", b"3");
    a.sync();
    let r = b.sync();
    assert!(r.failures.is_empty());
    assert_eq!(b.read("notes/three.txt").unwrap(), b"3");
}

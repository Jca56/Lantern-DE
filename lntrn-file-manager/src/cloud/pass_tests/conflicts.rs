// Writes the other machine made after this one read the mirror. A pass
// decides from a snapshot that can be minutes old by the time it acts; the
// cloud must refuse what the snapshot no longer justifies.

use std::sync::atomic::Ordering::SeqCst;

use super::super::failures::StuckKind;
use super::super::hash::sha256_bytes;
use super::{cloud, synced_pair, Machine};

#[test]
fn an_upload_does_not_replace_an_edit_made_since_the_mirror_was_read() {
    let (cloud, mut a, mut b) = synced_pair("stale-upload", 3);
    a.write("docs/f00.txt", b"edited on a");
    b.write("docs/f00.txt", b"edited on b");
    // a reads the cloud, then b's edit lands, then a acts on what it read.
    a.fetch();
    b.sync();
    let theirs = cloud.doc("docs/f00.txt");

    let r = a.stale_pass();
    assert!(r.failures.is_empty());
    assert_eq!(r.deferred, 1);
    // b's version is still what the cloud holds, and a's file is untouched.
    assert_eq!(cloud.doc("docs/f00.txt"), theirs);
    assert_eq!(a.read("docs/f00.txt").unwrap(), b"edited on a");
    // The refusal brought the doc itself up to date: no fetch needed for
    // the next pass to see two diverged versions and keep both.
    let r = a.stale_pass();
    assert!(r.failures.is_empty());
    assert_eq!(a.read("docs/f00.txt").unwrap(), b"edited on b");
    let copy = a
        .files()
        .into_iter()
        .find(|f| f.contains("conflict from"))
        .expect("a conflict copy");
    assert_eq!(a.read(&copy).unwrap(), b"edited on a");

    // And b ends up with both as well.
    b.sync();
    assert_eq!(b.read("docs/f00.txt").unwrap(), b"edited on b");
    assert_eq!(b.read(&copy).unwrap(), b"edited on a");
}

#[test]
fn a_deletion_never_beats_a_newer_edit() {
    let (cloud, mut a, mut b) = synced_pair("stale-delete", 20);
    std::fs::remove_file(a.path("docs/f03.txt")).unwrap();
    a.fetch();
    b.write("docs/f03.txt", b"edited on b after a looked");
    b.sync();

    let r = a.stale_pass();
    assert!(r.failures.is_empty() && r.held.is_none());
    assert_eq!(r.deferred, 1);
    assert_eq!(cloud.tombstones(), 0);

    // Deleted here, changed there: the edit comes back instead.
    a.poll();
    assert_eq!(a.read("docs/f03.txt").unwrap(), b"edited on b after a looked");
    b.sync();
    assert_eq!(b.read("docs/f03.txt").unwrap(), b"edited on b after a looked");
    assert_eq!((a.trashed(), b.trashed()), (0, 0));
}

#[test]
fn a_new_file_does_not_replace_one_the_other_machine_just_made() {
    let cloud = cloud("alice");
    let mut a = Machine::new("stale-new", "a", &cloud);
    let mut b = Machine::new("stale-new", "b", &cloud);
    a.write("notes.txt", b"a's notes");
    b.write("notes.txt", b"b's notes");
    a.fetch();
    b.sync();

    let r = a.stale_pass();
    assert_eq!(r.deferred, 1);
    assert_eq!(cloud.doc("notes.txt").sha256, sha256_bytes(b"b's notes"));
    assert_eq!(a.read("notes.txt").unwrap(), b"a's notes");
    a.stale_pass();
    assert_eq!(a.read("notes.txt").unwrap(), b"b's notes");
    assert_eq!(a.files().len(), 2);
}

#[test]
fn a_file_revived_over_a_purged_tombstone_is_uploaded() {
    // The mirror still shows a tombstone the other machine has since
    // cleaned away: the write is refused once, then goes through as new.
    let (cloud, mut a, mut b) = synced_pair("purged", 20);
    std::fs::remove_file(a.path("docs/f01.txt")).unwrap();
    a.sync();
    b.sync();
    assert_eq!(cloud.tombstones(), 1);
    cloud.docs.lock().unwrap().remove("docs/f01.txt");

    b.write("docs/f01.txt", b"back again");
    let r = b.stale_pass();
    assert_eq!(r.deferred, 1);
    let r = b.stale_pass();
    assert!(r.failures.is_empty());
    assert_eq!(r.deferred, 0);
    assert!(!cloud.doc("docs/f01.txt").deleted);
}

#[test]
fn a_write_refused_for_no_reason_is_a_failure_not_a_loop() {
    let cloud = cloud("alice");
    let mut a = Machine::new("refused", "a", &cloud);
    a.write("a.txt", b"one");
    cloud.writes_refused.store(true, SeqCst);
    let r = a.sync();
    // Refused, yet the doc is exactly as the mirror had it (absent): not a
    // change by the other machine. Shown, and backed off.
    assert_eq!(r.failures.len(), 1);
    assert_eq!(r.deferred, 0);
    assert_eq!(r.stuck[0].kind, StuckKind::Failing);
    let tries = cloud.upload_tries.load(SeqCst);
    let r = a.sync();
    assert!(r.failures.is_empty());
    assert_eq!(r.stuck.len(), 1);
    assert_eq!(cloud.upload_tries.load(SeqCst), tries);

    cloud.writes_refused.store(false, SeqCst);
    a.retry_failed();
    let r = a.sync();
    assert!(r.failures.is_empty() && r.stuck.is_empty());
    assert_eq!(cloud.live(), ["a.txt"]);
}

#[test]
fn every_write_carries_the_servers_stamp_not_this_machines() {
    let cloud = cloud("alice");
    let mut a = Machine::new("stamps", "a", &cloud);
    a.write("a.txt", b"one");
    let before = cloud.now();
    a.sync();
    let doc = cloud.doc("a.txt");
    let stamp = doc.updated_at.expect("a server stamp");
    assert!(stamp > before && stamp <= cloud.now());
    assert!(doc.version.is_some());
    // The mirror holds the doc exactly as the server does: the next pull
    // re-delivers it and changes nothing.
    assert_eq!(a.index.docs["a.txt"], doc);
    a.pull();
    assert_eq!(a.index.docs["a.txt"], doc);
}

#[test]
fn a_doc_of_unknown_version_is_looked_up_before_it_is_replaced() {
    // A write whose reply named no version leaves the mirror without one.
    // There is then nothing to make the next write conditional on, and an
    // unconditional write is not an option: the doc is fetched first.
    let (cloud, mut a, _b) = synced_pair("noversion", 3);
    a.fetch();
    a.index.docs.get_mut("docs/f00.txt").unwrap().version = None;
    a.write("docs/f00.txt", b"edited on a");

    let reads = cloud.reads();
    let r = a.stale_pass();
    assert!(r.failures.is_empty());
    assert_eq!(r.deferred, 1);
    assert_eq!(cloud.reads() - reads, 1);
    assert_eq!(cloud.doc("docs/f00.txt").sha256, sha256_bytes(b"content 0"));

    let r = a.stale_pass();
    assert!(r.failures.is_empty() && r.deferred == 0);
    assert_eq!(cloud.doc("docs/f00.txt").sha256, sha256_bytes(b"edited on a"));
}

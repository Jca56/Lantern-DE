// Blobs are named by the hash of their content. These are the ways the bytes
// and the name could come apart, and the files that cannot be synced at all.

use std::sync::atomic::Ordering::SeqCst;

use super::super::failures::{Failures, StuckKind};
use super::super::hash::sha256_bytes;
use super::super::storage::MAX_BLOB_BYTES;
use super::{cloud, synced_pair, Machine};

#[test]
fn a_file_that_changes_under_the_upload_is_not_stored_under_the_old_hash() {
    let cloud = cloud("alice");
    let mut a = Machine::new("midupload", "a", &cloud);
    a.write("report.txt", b"first version");
    let scanned = sha256_bytes(b"first version");

    // The scan has hashed the file and the pass has checked it once more;
    // the save lands as the upload begins.
    let target = a.path("report.txt");
    cloud.on_upload(Some(Box::new(move || {
        std::fs::write(&target, b"other version").unwrap();
    })));
    let r = a.sync();
    cloud.on_upload(None);
    assert!(r.failures.is_empty());
    assert_eq!(r.deferred, 1);
    // Nothing was stored: no blob under the old name holding the new
    // bytes, and no doc.
    assert_eq!(cloud.blob(&scanned), None);
    assert_eq!(cloud.blob_count(), 0);
    assert!(cloud.live().is_empty());

    // The next pass uploads what is there now, under its own name.
    let r = a.sync();
    assert!(r.failures.is_empty() && r.deferred == 0);
    let sha = sha256_bytes(b"other version");
    assert_eq!(cloud.doc("report.txt").sha256, sha);
    assert_eq!(cloud.blob(&sha).unwrap(), b"other version");
}

#[test]
fn a_download_that_is_not_what_its_hash_says_replaces_nothing() {
    let (cloud, mut a, mut b) = synced_pair("baddownload", 3);
    a.write("docs/f01.txt", b"the new version");
    a.sync();
    let sha = sha256_bytes(b"the new version");
    cloud.corrupt_blob(&sha);

    let r = b.sync();
    assert_eq!(r.failures.len(), 1);
    // The old version is still there, untouched, and no temp file is left.
    assert_eq!(b.read("docs/f01.txt").unwrap(), b"content 1");
    assert!(!b.files().iter().any(|f| f.ends_with(".fox-tmp")));
    assert_eq!(r.stuck.len(), 1);
    assert_eq!(r.stuck[0].path, "docs/f01.txt");

    // It is not pulled again on every pass...
    let downloads = cloud.downloads();
    for _ in 0..3 {
        let r = b.sync();
        assert!(r.failures.is_empty());
        assert_eq!((r.stuck.len(), r.stuck[0].attempts), (1, 1));
    }
    assert_eq!(cloud.downloads(), downloads);
    // ...and b does not push its old version over the new one either.
    assert_eq!(cloud.doc("docs/f01.txt").sha256, sha);

    // Once the blob is whole again (the uploader restores it, see
    // cleanup.rs), a retry gets it.
    cloud.repair_blob(&sha);
    b.retry_failed();
    let r = b.sync();
    assert!(r.failures.is_empty() && r.stuck.is_empty());
    assert_eq!(b.read("docs/f01.txt").unwrap(), b"the new version");
}

#[test]
fn a_file_the_cloud_refuses_is_not_sent_again_on_every_pass() {
    let cloud = cloud("alice");
    let mut a = Machine::new("refusedupload", "a", &cloud);
    a.write("a.bin", b"refused");
    cloud.uploads_refused.store(true, SeqCst);
    let r = a.sync();
    assert_eq!(r.failures.len(), 1);
    assert_eq!(cloud.upload_tries.load(SeqCst), 1);
    assert_eq!(r.stuck[0].kind, StuckKind::Failing);
    assert_eq!(r.stuck[0].detail, "status code 403 (Permission denied.)");

    // Thirty-second polls for the next half minute, hour, week: no hashing,
    // no reading, no sending. It is reported, not retried.
    for _ in 0..5 {
        let r = a.poll();
        assert!(r.failures.is_empty());
        assert_eq!(r.stuck.len(), 1);
    }
    assert_eq!(cloud.upload_tries.load(SeqCst), 1);
    // The record is on disk: a restart does not start over.
    assert_eq!(Failures::load_from(&a.places.failures, "alice").len(), 1);

    // Saving the file again makes it a new problem: tried at once.
    a.write("a.bin", b"refused, second version");
    let r = a.poll();
    assert_eq!(r.failures.len(), 1);
    assert_eq!(cloud.upload_tries.load(SeqCst), 2);
    assert_eq!(r.stuck[0].attempts, 1);

    // Other files are not held up by it.
    cloud.uploads_refused.store(false, SeqCst);
    a.write("b.txt", b"fine");
    let r = a.poll();
    assert!(r.failures.is_empty());
    assert_eq!(cloud.live(), ["b.txt"]);
    assert_eq!(r.stuck.len(), 1);

    // When the file is deleted, so is its record.
    std::fs::remove_file(a.path("a.bin")).unwrap();
    let r = a.poll();
    assert!(r.stuck.is_empty());
    assert_eq!(Failures::load_from(&a.places.failures, "alice").len(), 0);
}

#[test]
fn a_file_too_big_for_the_cloud_is_reported_and_left_alone() {
    let (cloud, mut a, mut b) = synced_pair("toobig", 3);
    // A new file of 1 GiB (sparse: no blocks on disk)...
    let iso = std::fs::File::create(a.path("big.iso")).unwrap();
    iso.set_len(MAX_BLOB_BYTES).unwrap();
    // ...and a synced file that grew past the limit.
    let grown = std::fs::OpenOptions::new()
        .write(true)
        .open(a.path("docs/f02.txt"))
        .unwrap();
    grown.set_len(MAX_BLOB_BYTES + 5).unwrap();

    let started = std::time::Instant::now();
    let r = a.sync();
    // Neither was hashed, read or sent.
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    assert!(r.failures.is_empty() && r.held.is_none());
    assert_eq!(cloud.upload_tries.load(SeqCst), 3);
    let stuck: Vec<(&str, StuckKind)> = r.stuck.iter().map(|s| (s.path.as_str(), s.kind)).collect();
    assert_eq!(
        stuck,
        [("big.iso", StuckKind::TooBig), ("docs/f02.txt", StuckKind::TooBig)]
    );
    // The version that did sync is still what the cloud and the other
    // machine have: growing too big is not a deletion.
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!(cloud.live().len(), 3);
    b.sync();
    assert_eq!(b.read("docs/f02.txt").unwrap(), b"content 2");
    assert_eq!(b.trashed(), 0);

    // Shrunk again: synced like any change.
    std::fs::write(a.path("docs/f02.txt"), b"small again").unwrap();
    std::fs::remove_file(a.path("big.iso")).unwrap();
    let r = a.sync();
    assert!(r.stuck.is_empty());
    b.sync();
    assert_eq!(b.read("docs/f02.txt").unwrap(), b"small again");
}

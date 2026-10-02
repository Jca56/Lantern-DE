// More of the sync threads end to end (see engine.rs for the harness): what
// the engine does about files that cannot sync, files still being written,
// writes the cloud refuses, and a cloud that will not answer the delta query.

use std::time::Instant;

use super::super::store::{Store, Timing};
use super::super::sync::{SyncProblem, SyncStatus};
use super::cloud;
use super::engine::{fast, machine, start, synced, wait_for, write, MS, NEVER};

#[test]
fn files_that_cannot_sync_are_reported_not_resent_and_retried_on_request() {
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("stuck", &cloud, 2);
    cloud.uploads_refused.store(true, SeqCst);
    let h = start(&cloud, &places, fast());
    // First the failures themselves, then (they are on record and waiting)
    // the standing report: two files cannot be synced.
    wait_for("the standing report", || {
        h.problem() == Some(SyncProblem::CannotSync) && h.stuck_files().0 == 2
    });
    assert_eq!(h.status(), SyncStatus::Error);
    let (_, sample) = h.stuck_files();
    assert_eq!(sample[0].path, "docs/f00.txt");
    assert_eq!(cloud.upload_tries.load(SeqCst), 2);

    // Poll after poll: reported, not sent again.
    std::thread::sleep(500 * MS);
    assert_eq!(cloud.upload_tries.load(SeqCst), 2);
    assert_eq!(cloud.uploads(), 0);

    // "Try again", asked from another window.
    cloud.uploads_refused.store(false, SeqCst);
    let other_window = start(&cloud, &places, fast());
    wait_for("the other window to follow", || other_window.stuck_files().0 == 2);
    assert!(!other_window.is_owner());
    assert!(other_window.retry_failed());
    wait_for("the retry to go through", || {
        cloud.live().len() == 2 && synced(&h) && h.stuck_files().0 == 0
    });
    assert_eq!(h.problem(), None);
}

#[test]
fn a_file_still_being_written_waits_until_it_has_settled() {
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("busy", &cloud, 0);
    // No polling: after the first pass only the watcher, or the engine's
    // own "look again", can start one.
    let timing = Timing {
        poll_every: NEVER,
        busy_window: 400 * MS,
        ..fast()
    };
    let h = start(&cloud, &places, timing);
    wait_for("the first pass", || synced(&h));

    // A copy in progress: written to again and again for a while.
    let path = places.root.join("docs/copying.bin");
    let started = Instant::now();
    let mut content = Vec::new();
    while started.elapsed() < 600 * MS {
        content.extend_from_slice(b"another chunk ");
        std::fs::write(&path, &content).unwrap();
        std::thread::sleep(20 * MS);
    }
    // Nothing of it was uploaded while it grew: not one partial version.
    assert_eq!(cloud.upload_tries.load(SeqCst), 0);
    assert_ne!(h.status(), SyncStatus::Idle);

    wait_for("the finished file", || cloud.live().len() == 1 && synced(&h));
    assert_eq!(cloud.upload_tries.load(SeqCst), 1);
    let sha = super::super::hash::sha256_bytes(&content);
    assert_eq!(cloud.blob(&sha).unwrap(), content);
}

#[test]
fn a_full_list_is_followed_by_deltas_on_the_servers_clock() {
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("cursor", &cloud, 3);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 3 && synced(&h));
    // Let the overlap pass (server time), then watch a few quiet polls.
    cloud.advance(super::super::remote_index::OVERLAP + super::super::stamp::SECOND);
    let polls = cloud.deltas.load(SeqCst);
    wait_for("a poll past the overlap", || cloud.deltas.load(SeqCst) > polls + 1);
    let (polls, reads) = (cloud.deltas.load(SeqCst), cloud.reads());
    wait_for("five more polls", || cloud.deltas.load(SeqCst) >= polls + 5);
    let polled = cloud.deltas.load(SeqCst) - polls;
    // One read per quiet poll (a poll may be in flight: allow it its read).
    assert!(cloud.reads() - reads <= polled + 1, "{} reads for {polled} polls", cloud.reads() - reads);
    assert_eq!(cloud.lists.load(SeqCst), 1);
}

#[test]
fn an_edit_made_by_the_other_machine_mid_pass_ends_as_a_conflict_copy_not_a_lost_edit() {
    use super::super::firestore::Expect;
    use super::super::hash::sha256_bytes;
    let cloud = cloud("alice");
    let (_dir, places) = machine("moved", &cloud, 2);
    // No polling: only the engine's own "look again" can run the pass that
    // settles a refused write.
    let timing = Timing {
        poll_every: NEVER,
        ..fast()
    };
    let h = start(&cloud, &places, timing);
    wait_for("the first pass", || cloud.live().len() == 2 && synced(&h));

    // The other machine saves f00 just as this one starts uploading its own
    // edit of it: after this machine last read the cloud, before its write.
    let theirs = sha256_bytes(b"edited on the laptop");
    let other = cloud.clone();
    let their_sha = theirs.clone();
    cloud.on_upload(Some(Box::new(move || {
        let mut doc = other.doc("docs/f00.txt");
        if doc.sha256 == their_sha {
            return;
        }
        let seen = Expect::Version(doc.version.clone().unwrap());
        doc.sha256 = their_sha.clone();
        doc.device = "laptop".to_string();
        other.put_blob(b"edited on the laptop");
        other.put_doc(&doc, &seen).unwrap();
    })));
    write(&places, "docs/f00.txt", b"edited on the pc");

    wait_for("both versions to be kept", || {
        synced(&h)
            && std::fs::read(places.root.join("docs/f00.txt")).is_ok_and(|b| b == b"edited on the laptop")
    });
    let copy = std::fs::read_dir(places.root.join("docs"))
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().contains("conflict from"))
        .expect("a conflict copy");
    assert_eq!(std::fs::read(copy.path()).unwrap(), b"edited on the pc");
    assert_eq!(cloud.doc("docs/f00.txt").sha256, theirs);
    assert_eq!(cloud.live().len(), 3);
}

#[test]
fn a_refused_delta_query_degrades_to_occasional_full_lists_not_to_no_sync() {
    use super::super::firestore::Expect;
    use super::super::hash::sha256_bytes;
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("nodelta", &cloud, 2);
    cloud.deltas_refused.store(true, SeqCst);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 2 && synced(&h));

    // Local changes still go up...
    write(&places, "docs/local.txt", b"local");
    wait_for("the local change", || cloud.live().len() == 3);
    // ...and the other machine's still come down.
    let mut theirs = cloud.doc("docs/f00.txt");
    theirs.path = "docs/theirs.txt".to_string();
    theirs.sha256 = sha256_bytes(b"from the laptop");
    cloud.put_blob(b"from the laptop");
    cloud.put_doc(&theirs, &Expect::Absent).unwrap();
    wait_for("the remote change", || {
        std::fs::read(places.root.join("docs/theirs.txt")).is_ok_and(|b| b == b"from the laptop")
    });
    wait_for("idle again", || synced(&h));
    assert_eq!(h.problem(), None);

    // Not by listing on every poll: at most one list per retry period
    // (0.5 s here), however many polls (every 0.12 s) went by.
    let lists = cloud.lists.load(SeqCst);
    std::thread::sleep(700 * MS);
    assert!(cloud.lists.load(SeqCst) - lists <= 2);

    // The query works again: back to deltas, no more lists.
    cloud.deltas_refused.store(false, SeqCst);
    wait_for("deltas to resume", || {
        let lists = cloud.lists.load(SeqCst);
        std::thread::sleep(600 * MS);
        cloud.lists.load(SeqCst) == lists
    });
}

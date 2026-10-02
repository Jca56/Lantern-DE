// What a poll costs. Firestore bills a read per document returned (at least
// one per query) and the free tier has 50 000 a day; a poll runs every 30
// seconds on every machine.

use super::super::remote_index::OVERLAP;
use super::super::stamp::{MINUTE, SECOND};
use super::{cloud, Machine};

#[test]
fn a_quiet_poll_costs_one_read_however_big_the_last_burst_was() {
    let cloud = cloud("alice");
    let mut a = Machine::new("quiet", "a", &cloud);
    let mut b = Machine::new("quiet", "b", &cloud);
    for i in 0..40 {
        a.write(&format!("burst/f{i:02}.txt"), format!("{i}").as_bytes());
    }
    a.sync();
    b.sync();
    assert_eq!(b.files().len(), 40);

    // Polls inside the overlap see the burst again (and change nothing)...
    cloud.advance(30 * SECOND);
    let reads = cloud.reads();
    let r = b.poll();
    assert!(r.failures.is_empty());
    assert_eq!(cloud.reads() - reads, 40);
    assert_eq!(cloud.downloads(), 40);

    // ...and once it has passed, every poll is one read: for good, not
    // until the next write somewhere happens to move a cursor.
    cloud.advance(OVERLAP);
    b.poll();
    for _ in 0..10 {
        cloud.advance(30 * SECOND);
        let reads = cloud.reads();
        b.poll();
        assert_eq!(cloud.reads() - reads, 1);
    }
    // The same for the machine that wrote the burst.
    a.poll();
    let reads = cloud.reads();
    a.poll();
    assert_eq!(cloud.reads() - reads, 1);
}

#[test]
fn a_change_is_picked_up_by_the_next_poll_long_after_the_last_one() {
    let cloud = cloud("alice");
    let mut a = Machine::new("later", "a", &cloud);
    let mut b = Machine::new("later", "b", &cloud);
    a.write("a.txt", b"one");
    a.sync();
    b.sync();

    // b is closed for a week; a keeps working.
    cloud.advance(3 * 24 * 60 * MINUTE);
    a.write("a.txt", b"two");
    a.write("new.txt", b"new");
    a.poll();
    cloud.advance(4 * 24 * 60 * MINUTE);

    let reads = cloud.reads();
    b.poll();
    assert_eq!(b.read("a.txt").unwrap(), b"two");
    assert_eq!(b.read("new.txt").unwrap(), b"new");
    // Two changed docs, two reads: not the collection.
    assert_eq!(cloud.reads() - reads, 2);
}

#[test]
fn own_writes_do_not_wait_for_a_poll_and_are_not_undone_by_one() {
    let cloud = cloud("alice");
    let mut a = Machine::new("own-poll", "a", &cloud);
    a.write("a.txt", b"one");
    a.sync();
    a.write("a.txt", b"two");
    a.poll();
    assert_eq!(cloud.uploads(), 2);
    // Poll after poll: the file stays as saved, nothing travels.
    for _ in 0..3 {
        cloud.advance(20 * SECOND);
        let r = a.poll();
        assert!(r.failures.is_empty() && r.deferred == 0);
    }
    assert_eq!(a.read("a.txt").unwrap(), b"two");
    assert_eq!((cloud.uploads(), cloud.downloads()), (2, 0));
}

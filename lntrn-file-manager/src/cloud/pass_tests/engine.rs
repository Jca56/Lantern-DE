// The sync threads end to end: `SyncHandle` with its owner/follower thread and
// the engine loop, on a fast clock, against the in-memory cloud. Everything
// waits for a condition (never for a fixed time to have been enough), so a
// busy machine makes these slower, not flaky.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::owner::OwnerLock;
use super::super::store::{Places, Store, Timing};
use super::super::sync::{SyncHandle, SyncProblem, SyncStatus};
use super::super::{Session, TestDir};
use super::fake::{FakeCloud, Outage};
use super::{cloud, places_in};

pub(super) const MS: Duration = Duration::from_millis(1);
pub(super) const NEVER: Duration = Duration::from_secs(3600);

pub(super) fn fast() -> Timing {
    Timing {
        tick: 10 * MS,
        poll_every: 120 * MS,
        full_every_ms: 6 * 3600 * 1000,
        full_retry: 500 * MS,
        debounce: 40 * MS,
        settle_max: 400 * MS,
        busy_window: Duration::ZERO,
        err_backoff_start: 150 * MS,
        err_backoff_cap: 300 * MS,
        quota_backoff_start: 200 * MS,
        quota_backoff_cap: 400 * MS,
        approval_ttl: Duration::from_secs(60),
        follow_tick: 20 * MS,
        lock_retry: 60 * MS,
        crash_pause: Duration::from_secs(1),
    }
}

pub(super) fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(10 * MS);
    }
}

fn sign_in(places: &Places, uid: &str) {
    let session = Session {
        uid: uid.to_string(),
        email: "someone@example.com".to_string(),
        id_token: "id".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: u64::MAX / 2,
    };
    std::fs::create_dir_all(places.session.parent().unwrap()).unwrap();
    std::fs::write(&places.session, serde_json::to_vec(&session).unwrap()).unwrap();
}

/// A signed-in machine with `n` files in its folder. Keep the `TestDir`
/// alive (and declared before the handles) for the length of the test.
pub(super) fn machine(test: &str, cloud: &Arc<FakeCloud>, n: usize) -> (TestDir, Places) {
    let dir = TestDir::new(&format!("engine-{test}"));
    let places = places_in(&dir);
    std::fs::create_dir_all(places.root.join("docs")).unwrap();
    for i in 0..n {
        write(&places, &format!("docs/f{i:02}.txt"), format!("content {i}").as_bytes());
    }
    sign_in(&places, &cloud.uid());
    (dir, places)
}

pub(super) fn write(places: &Places, rel: &str, bytes: &[u8]) {
    std::fs::write(places.root.join(rel), bytes).unwrap();
}

/// A running Fox. Stopped, and waited for, when it goes out of scope, so no
/// sync thread is still writing into the scratch directory as it is removed.
pub(super) struct Fox(Option<SyncHandle>);

impl Fox {
    fn quit(mut self) {
        self.stop_and_wait();
    }

    fn stop_and_wait(&mut self) {
        if let Some(h) = self.0.take() {
            h.shutdown(Duration::from_secs(30));
        }
    }
}

impl std::ops::Deref for Fox {
    type Target = SyncHandle;
    fn deref(&self) -> &SyncHandle {
        self.0.as_ref().expect("still running")
    }
}

impl Drop for Fox {
    fn drop(&mut self) {
        self.stop_and_wait();
    }
}

pub(super) fn start(cloud: &Arc<FakeCloud>, places: &Places, timing: Timing) -> Fox {
    Fox(Some(SyncHandle::spawn_with(cloud.clone(), places.clone(), timing)))
}

pub(super) fn synced(h: &SyncHandle) -> bool {
    h.status() == SyncStatus::Idle
}

fn lock_is_free(lock: &Path) -> bool {
    matches!(OwnerLock::try_acquire(lock), Ok(Some(_)))
}

#[test]
fn syncs_changes_and_releases_ownership_when_stopped() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("basic", &cloud, 3);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 3 && synced(&h));
    assert!(h.is_owner());
    assert!(!lock_is_free(&places.lock));

    write(&places, "docs/new.txt", b"new");
    wait_for("the new file", || cloud.live().len() == 4);

    h.quit();
    assert!(lock_is_free(&places.lock));
}

#[test]
fn a_restart_does_not_list_the_collection_again() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("restart", &cloud, 3);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 3 && synced(&h));
    h.quit();
    assert_eq!(cloud.lists.load(std::sync::atomic::Ordering::SeqCst), 1);

    let h = start(&cloud, &places, fast());
    wait_for("the pass after the restart", || {
        synced(&h) && cloud.deltas.load(std::sync::atomic::Ordering::SeqCst) > 0
    });
    h.quit();
    assert_eq!(cloud.lists.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(cloud.uploads(), 3);
}

#[test]
fn the_next_fox_takes_over_and_only_one_ever_syncs() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("takeover", &cloud, 2);
    let first = start(&cloud, &places, fast());
    wait_for("the first owner", || first.is_owner() && synced(&first));
    let second = start(&cloud, &places, fast());
    // The follower shows the owner's status and does not sync itself.
    wait_for("the follower to follow", || synced(&second));
    assert!(!second.is_owner());
    assert_eq!(cloud.uploads(), 2);

    first.quit();
    wait_for("the takeover", || second.is_owner());
    write(&places, "docs/later.txt", b"later");
    wait_for("the new owner to sync", || cloud.live().len() == 3);
    // Taking over is a delta, not another full list.
    assert_eq!(cloud.lists.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn signing_out_stops_owner_and_follower_and_stays_signed_out() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("signout", &cloud, 2);
    let owner = start(&cloud, &places, fast());
    wait_for("the owner", || owner.is_owner() && synced(&owner));
    let follower = start(&cloud, &places, fast());
    wait_for("the follower", || synced(&follower));

    // What `Session::forget` / --cloud-logout does.
    std::fs::remove_file(&places.session).unwrap();
    wait_for("both to stop", || owner.needs_sign_in() && follower.needs_sign_in());
    assert_eq!(owner.problem(), Some(SyncProblem::SignedOut));
    assert_eq!(owner.status(), SyncStatus::Error);
    wait_for("the lock to be released", || lock_is_free(&places.lock));

    // Nothing syncs any more, and nothing brings the session back.
    write(&places, "docs/after.txt", b"after sign-out");
    std::thread::sleep(300 * MS);
    assert_eq!(cloud.live().len(), 2);
    assert!(!places.session.exists());
}

#[test]
fn a_rejected_sign_in_stops_the_engine_instead_of_retrying() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("revoked", &cloud, 2);
    cloud.set_outage(Some(Outage::Revoked));
    let h = start(&cloud, &places, fast());
    wait_for("the engine to give up", || h.needs_sign_in());
    assert_eq!(h.problem(), Some(SyncProblem::SignInRequired));
    wait_for("the lock to be released", || lock_is_free(&places.lock));
    let asked = cloud.lists.load(std::sync::atomic::Ordering::SeqCst);
    std::thread::sleep(300 * MS);
    assert_eq!(cloud.lists.load(std::sync::atomic::Ordering::SeqCst), asked);
}

#[test]
fn held_deletions_wait_for_a_yes_given_in_any_window() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("yes", &cloud, 12);
    let owner = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 12 && synced(&owner));
    let other_window = start(&cloud, &places, fast());

    std::fs::remove_dir_all(places.root.join("docs")).unwrap();
    wait_for("the hold to show in the other window", || {
        other_window.held_deletions().is_some()
    });
    let held = other_window.held_deletions().unwrap();
    assert_eq!((held.held, held.total), (12, 12));
    assert_eq!(owner.problem(), Some(SyncProblem::DeletionsHeld));
    assert_eq!(owner.status(), SyncStatus::Error);

    // Poll after poll goes by: nothing is deleted without the answer.
    std::thread::sleep(400 * MS);
    assert_eq!(cloud.tombstones(), 0);

    assert!(other_window.confirm_deletions());
    wait_for("the deletions to go out", || cloud.tombstones() == 12);
    wait_for("the question to go away", || {
        owner.held_deletions().is_none() && other_window.held_deletions().is_none() && synced(&owner)
    });
    assert!(!other_window.confirm_deletions());
}

#[test]
fn held_deletions_come_back_on_a_no() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("no", &cloud, 12);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 12 && synced(&h));

    std::fs::remove_dir_all(places.root.join("docs")).unwrap();
    wait_for("the hold", || h.held_deletions().is_some());
    assert!(h.decline_deletions());
    wait_for("the files to return", || {
        (0..12).all(|i| places.root.join(format!("docs/f{i:02}.txt")).exists()) && synced(&h)
    });
    assert_eq!(cloud.tombstones(), 0);
    assert!(h.held_deletions().is_none());
    assert_eq!(
        std::fs::read(places.root.join("docs/f07.txt")).unwrap(),
        b"content 7"
    );
}

#[test]
fn a_missing_folder_pauses_sync_and_costs_no_remote_reads() {
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("nofolder", &cloud, 12);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 12 && synced(&h));

    let away = places.root.with_extension("away");
    std::fs::rename(&places.root, &away).unwrap();
    wait_for("the pause", || h.problem() == Some(SyncProblem::FolderMissing));
    assert_eq!(h.status(), SyncStatus::Error);
    let reads = cloud.deltas.load(SeqCst) + cloud.lists.load(SeqCst);
    std::thread::sleep(400 * MS);
    assert_eq!(cloud.deltas.load(SeqCst) + cloud.lists.load(SeqCst), reads);
    assert!(!places.root.exists());
    assert_eq!(cloud.tombstones(), 0);
    assert!(h.held_deletions().is_none());

    std::fs::rename(&away, &places.root).unwrap();
    wait_for("sync to resume", || synced(&h) && h.problem().is_none());
    assert_eq!(cloud.tombstones(), 0);
    // The watcher follows the folder that is there now.
    std::fs::write(places.root.join("docs/back.txt"), b"back").unwrap();
    wait_for("a change in the returned folder", || cloud.live().len() == 13);
}

#[test]
fn files_sync_ignores_do_not_wake_the_engine() {
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("wake", &cloud, 2);
    // This test is about the watcher. On a machine that is out of inotify
    // instances the engine falls back to polling, and there is nothing to
    // test.
    {
        use notify::Watcher;
        let probe = notify::recommended_watcher(|_| {})
            .and_then(|mut w| w.watch(&places.root, notify::RecursiveMode::Recursive));
        if probe.is_err() {
            return;
        }
    }
    // No polling at all: only the file watcher can start a pass.
    let timing = Timing {
        poll_every: NEVER,
        ..fast()
    };
    let h = start(&cloud, &places, timing);
    wait_for("the first pass", || cloud.live().len() == 2 && synced(&h));
    let passes = cloud.deltas.load(SeqCst) + cloud.lists.load(SeqCst);

    // What a failing download leaves behind, over and over.
    for i in 0..20 {
        write(&places, &format!("docs/f00.txt.{i}.fox-tmp"), b"partial");
        std::thread::sleep(10 * MS);
    }
    std::thread::sleep(300 * MS);
    assert_eq!(cloud.deltas.load(SeqCst) + cloud.lists.load(SeqCst), passes);

    // A real change does.
    write(&places, "docs/real.txt", b"real");
    wait_for("the real change", || cloud.live().len() == 3);
    assert!(!cloud.live().iter().any(|p| p.ends_with(".fox-tmp")));
}

#[test]
fn a_failed_pass_is_not_retried_before_its_time() {
    use std::sync::atomic::Ordering::SeqCst;
    let cloud = cloud("alice");
    let (_dir, places) = machine("backoff", &cloud, 2);
    // A failed pass may not run again for the rest of this test, whatever
    // the watcher or the poll timer say.
    let timing = Timing {
        err_backoff_start: NEVER,
        err_backoff_cap: NEVER,
        ..fast()
    };
    let h = start(&cloud, &places, timing);
    wait_for("the first pass", || cloud.live().len() == 2 && synced(&h));

    cloud.set_outage(Some(Outage::Network));
    wait_for("the failure to show", || h.problem() == Some(SyncProblem::Failed));
    let attempts = cloud.deltas.load(SeqCst);
    for i in 0..10 {
        write(&places, &format!("docs/more{i}.txt"), b"more");
        std::thread::sleep(20 * MS);
    }
    std::thread::sleep(300 * MS);
    assert_eq!(cloud.deltas.load(SeqCst), attempts);
    assert_eq!(h.status(), SyncStatus::Error);
}

#[test]
fn outages_end_and_sync_catches_up() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("recover", &cloud, 2);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 2 && synced(&h));

    cloud.set_outage(Some(Outage::Network));
    write(&places, "docs/while-offline.txt", b"offline");
    wait_for("the failure to show", || h.problem() == Some(SyncProblem::Failed));
    cloud.set_outage(Some(Outage::Quota));
    wait_for("the quota pause to show", || h.status() == SyncStatus::RateLimited);
    assert!(!h.needs_sign_in());

    cloud.set_outage(None);
    wait_for("sync to catch up", || cloud.live().len() == 3 && synced(&h));
}

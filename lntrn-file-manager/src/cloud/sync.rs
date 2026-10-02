// `SyncHandle`: what a Fox process holds for cloud sync, and what the UI asks.
//
// Spawning a handle starts one background thread. That thread is either
//   - the OWNER: it holds the machine-wide lock (owner.rs) and runs the sync
//     engine (engine.rs), or
//   - a FOLLOWER: another Fox process owns sync. The thread only mirrors the
//     owner's report from the status file and tries the lock every few
//     seconds, so sync carries on in this process when the owner exits.
// Either way the handle answers the same questions, so the UI does not care
// which one it got.
//
// The thread ends by itself when the account is signed out (the session file
// is removed or replaced) or the saved sign-in is rejected for good; the
// handle then says `needs_sign_in()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::auth::AuthStop;
use super::failures::StuckFile;
use super::guard::HeldDeletions;
use super::http::Authed;
use super::owner::{OwnerLock, ReportReader};
use super::session::SessionWatch;
use super::store::{Places, Store, Timing};
use super::{CloudConfig, Session};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncStatus {
    Idle,
    Syncing,
    Error,
    /// Firestore quota exhausted (HTTP 429) — sync is intentionally paused
    /// and will retry with backoff. Not an error; quotas reset daily.
    RateLimited,
}

/// What is behind `SyncStatus::Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncProblem {
    /// ~/Cloud does not exist (or is not a folder). Nothing syncs until it
    /// is back; nothing is deleted anywhere because of it.
    FolderMissing,
    /// ~/Cloud is there but cannot be listed.
    FolderUnreadable,
    /// Deletions are waiting for the user: see `SyncHandle::held_deletions`.
    DeletionsHeld,
    /// Some files or folders inside ~/Cloud could not be read. They are left
    /// alone on both sides; everything else syncs.
    Unreadable,
    /// Some files failed to sync (network, server). Retried automatically.
    Failed,
    /// Everything that can sync is in sync, but some files cannot: they are
    /// too big for the cloud, or keep failing and are waiting for their
    /// next try. See `SyncHandle::stuck_files` (which lists the unreadable
    /// ones of `Unreadable` too).
    CannotSync,
    /// The account was signed out, here or from another Fox.
    SignedOut,
    /// The saved sign-in is no longer accepted (password changed, account
    /// disabled). The user has to sign in again.
    SignInRequired,
}

/// Everything a window needs to show about sync. The owner publishes it;
/// followers read the same thing from the status file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub status: SyncStatus,
    /// Set whenever `status` is `Error`.
    pub problem: Option<SyncProblem>,
    /// Deletions held back, waiting for confirm or decline. Can be set
    /// together with any `problem`.
    pub held: Option<HeldDeletions>,
    /// Files and folders in ~/Cloud the last pass could not read.
    pub unreadable: usize,
    /// Files that failed to sync in the last pass (details in the log).
    pub failed: usize,
    /// Files that are not in sync and will not be after the next pass
    /// either: "N files cannot be synced". Exact count.
    #[serde(default)]
    pub stuck: usize,
    /// The first of them, sorted by path, with the reason for each.
    #[serde(default)]
    pub stuck_sample: Vec<StuckFile>,
    /// The account the report is about.
    pub uid: String,
}

impl SyncReport {
    pub(super) fn new(uid: &str) -> Self {
        Self {
            status: SyncStatus::Idle,
            problem: None,
            held: None,
            unreadable: 0,
            failed: 0,
            stuck: 0,
            stuck_sample: Vec::new(),
            uid: uid.to_string(),
        }
    }
}

/// State shared between the handle and its thread.
pub(super) struct Shared {
    pub(super) places: Places,
    pub(super) timing: Timing,
    report: Mutex<SyncReport>,
    stop: AtomicBool,
    owner: AtomicBool,
    ended: Mutex<Option<AuthStop>>,
}

/// A panic on the sync thread must not take the UI thread down with a
/// poisoned lock: the data behind these mutexes is plain values.
fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    pub(super) fn new(uid: &str, places: Places, timing: Timing) -> Self {
        // "Synced" is a claim; until the owner's first pass (or the owner's
        // report, for a follower) nothing backs it.
        let mut report = SyncReport::new(uid);
        report.status = SyncStatus::Syncing;
        Self {
            places,
            timing,
            report: Mutex::new(report),
            stop: AtomicBool::new(false),
            owner: AtomicBool::new(false),
            ended: Mutex::new(None),
        }
    }

    pub(super) fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub(super) fn report(&self) -> SyncReport {
        locked(&self.report).clone()
    }

    /// Owner: make `report` the current one, for this process and (through
    /// the status file) for the others. A report equal to the current one
    /// costs nothing.
    pub(super) fn publish(&self, report: SyncReport) {
        {
            let mut cur = locked(&self.report);
            if *cur == report {
                return;
            }
            *cur = report.clone();
        }
        super::owner::write_report(&self.places.status, &report);
    }

    /// Owner, on taking over: start from `report` and overwrite whatever a
    /// previous owner left in the status file.
    pub(super) fn replace(&self, report: SyncReport) {
        *locked(&self.report) = report.clone();
        super::owner::write_report(&self.places.status, &report);
    }

    /// Owner: change only the coarse status.
    pub(super) fn publish_status(&self, status: SyncStatus) {
        let mut report = self.report();
        report.status = status;
        self.publish(report);
    }

    fn end(&self, uid: &str, why: AuthStop, as_owner: bool) {
        *locked(&self.ended) = Some(why);
        let mut report = SyncReport::new(uid);
        report.status = SyncStatus::Error;
        report.problem = Some(match why {
            AuthStop::Revoked => SyncProblem::SignInRequired,
            AuthStop::SignedOut => SyncProblem::SignedOut,
        });
        if as_owner {
            self.publish(report);
        } else {
            *locked(&self.report) = report;
        }
    }
}

pub struct SyncHandle {
    shared: Arc<Shared>,
    /// Joined by `shutdown`.
    thread: Option<thread::JoinHandle<()>>,
}

impl SyncHandle {
    /// Start cloud sync for this process. Returns immediately. The process
    /// becomes the sync owner if no other Fox is; otherwise it follows the
    /// owner and takes over when that one exits.
    ///
    /// Not for short-lived processes (file pickers): they would grab
    /// ownership and die mid-pass.
    pub fn spawn(cfg: Arc<CloudConfig>, session: Arc<Mutex<Session>>) -> Self {
        let authed = Authed { cfg, session };
        Self::spawn_with(Arc::new(authed), Places::real(), Timing::REAL)
    }

    pub(super) fn spawn_with(store: Arc<dyn Store>, places: Places, timing: Timing) -> Self {
        let shared = Arc::new(Shared::new(&store.uid(), places, timing));
        let shared_thr = shared.clone();
        let thread = thread::Builder::new()
            .name("fox-cloud-sync".into())
            .spawn(move || thread_main(&*store, &shared_thr))
            .map_err(|e| {
                super::log_line(&format!("cannot start the sync thread: {e}"));
                let mut report = shared.report();
                report.status = SyncStatus::Error;
                report.problem = Some(SyncProblem::Failed);
                *locked(&shared.report) = report;
            })
            .ok();
        Self { shared, thread }
    }

    /// Cheap snapshot of the current sync status. Copies under the mutex.
    /// (The windows show the whole `report()`, of which this is one field.)
    #[cfg(test)]
    pub fn status(&self) -> SyncStatus {
        locked(&self.shared.report).status
    }

    /// Ask the thread to stop. It finishes the file it is on, saves its
    /// progress and releases ownership. Dropping the handle does the same.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

// What the UI needs to show the held-deletions question, the sign-in state
// and the details behind "Sync error" (src/cloud_ui.rs is the caller).
impl SyncHandle {
    /// The whole picture: status, what is wrong, what is held.
    pub fn report(&self) -> SyncReport {
        self.shared.report()
    }

    /// Why the status is `Error`, if it is.
    pub fn problem(&self) -> Option<SyncProblem> {
        locked(&self.shared.report).problem
    }

    /// Deletions sync wanted to send to the cloud and did not, because they
    /// look like an accident (see guard.rs). `None`: nothing is waiting.
    pub fn held_deletions(&self) -> Option<HeldDeletions> {
        locked(&self.shared.report).held.clone()
    }

    /// Every path of the held set `id`, for a window to list before it
    /// offers to delete them everywhere. `None` when the complete list for
    /// exactly that set cannot be had.
    pub fn held_paths(&self, id: &str) -> Option<Vec<String>> {
        let uid = locked(&self.shared.report).uid.clone();
        super::guard::read_held_list(&self.shared.places.held_list, &uid, id)
    }

    /// "Yes, delete them": the held files are deleted from the cloud (and so
    /// from the other machine) on the next pass. Applies to exactly the set
    /// `held_deletions()` returns right now. False if nothing is held.
    /// (The dialog answers through `answer_held`, which names the set.)
    #[cfg(test)]
    pub fn confirm_deletions(&self) -> bool {
        self.answer(None, true)
    }

    /// "No, bring them back": nothing is deleted; the held files are
    /// downloaded from the cloud again on the next pass. False if nothing
    /// is held.
    #[cfg(test)]
    pub fn decline_deletions(&self) -> bool {
        self.answer(None, false)
    }

    /// The answer of a dialog that showed the held set `id`: confirm
    /// (delete them from the cloud) or decline (bring them back), exactly
    /// as `confirm_deletions` / `decline_deletions`. Recorded only while
    /// that set is still the one being held; false when it is not (a pass
    /// has computed another one since, which the user has not seen), when
    /// nothing is held, or when the answer could not be written.
    pub fn answer_held(&self, id: &str, confirm: bool) -> bool {
        self.answer(Some(id), confirm)
    }

    /// The files that cannot be synced right now, for "N files cannot be
    /// synced" and the list behind it: how many, and the first of them
    /// (sorted by path) with why: too big for the cloud (1 GiB or larger),
    /// unreadable, or failing and waiting for the next try. (The windows
    /// read the same two fields from `report()`.)
    #[cfg(test)]
    pub fn stuck_files(&self) -> (usize, Vec<StuckFile>) {
        let report = locked(&self.shared.report);
        (report.stuck, report.stuck_sample.clone())
    }

    /// "Try again now": the files that keep failing are tried on the next
    /// pass instead of when their wait is over. (A file that is too big
    /// stays too big.) False if the request could not be left for the sync
    /// owner.
    pub fn retry_failed(&self) -> bool {
        match super::failures::request_retry(&self.shared.places.retry) {
            Ok(()) => true,
            Err(e) => {
                super::log_line(&format!("cannot ask for a retry of the failing files: {e}"));
                false
            }
        }
    }

    fn answer(&self, only: Option<&str>, confirm: bool) -> bool {
        let Some(held) = self.held_deletions() else {
            return false;
        };
        if only.is_some_and(|id| id != held.id) {
            return false;
        }
        match super::guard::write_answer(&self.shared.places.answer, &held.id, confirm) {
            Ok(()) => true,
            Err(e) => {
                super::log_line(&format!("cannot record the answer for held deletions: {e}"));
                false
            }
        }
    }

    /// True once sync has stopped because the account this handle was
    /// started for is no longer signed in on this machine: it was signed out
    /// (here or by another Fox), another account signed in, or its saved
    /// sign-in was rejected. The handle is finished: drop it together with
    /// the `CloudState`, then try to load the cached session again — if
    /// there is none, the Cloud button offers the sign-in dialog; if there
    /// is one (the other account), sync starts for that.
    pub fn needs_sign_in(&self) -> bool {
        locked(&self.shared.ended).is_some()
    }

    /// Is this process the one running the sync engine right now?
    pub fn is_owner(&self) -> bool {
        self.shared.owner.load(Ordering::Relaxed)
    }

    /// `stop()`, then wait up to `wait` for the thread to wind down. For a
    /// clean exit: a pass cut off by the process ending loses nothing, but
    /// repeats its last few seconds of work next time.
    pub fn shutdown(mut self, wait: Duration) {
        self.stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now() + wait;
        while !thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

impl Drop for SyncHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn thread_main(store: &dyn Store, shared: &Shared) {
    let uid = store.uid();
    let places = &shared.places;
    let timing = shared.timing;
    let mut session = SessionWatch::new(places.session.clone(), &uid);
    let mut reader = ReportReader::new(places.status.clone());
    let mut next_lock_try = Instant::now();
    let mut lock_error_logged = false;

    while !shared.stopping() {
        if !session.still_signed_in() {
            // Signed out, or did the owner find the sign-in rejected? It
            // removes the session file first and says why a moment later,
            // once its pass has unwound: look at its report now and once
            // more before settling for "signed out" (which a window shows
            // as a quiet state, not as "sync stopped: sign in again").
            let revoked = |reader: &mut ReportReader| {
                if let Some(report) = reader.poll().filter(|r| r.uid == uid) {
                    *locked(&shared.report) = report;
                }
                locked(&shared.report).problem == Some(SyncProblem::SignInRequired)
            };
            let why = if revoked(&mut reader) || {
                thread::sleep(timing.follow_tick);
                revoked(&mut reader)
            } {
                AuthStop::Revoked
            } else {
                AuthStop::SignedOut
            };
            shared.end(&uid, why, false);
            return;
        }

        if Instant::now() >= next_lock_try {
            next_lock_try = Instant::now() + timing.lock_retry;
            match OwnerLock::try_acquire(&places.lock) {
                Ok(Some(lock)) => {
                    match own(store, &uid, shared) {
                        Some(super::engine::Exit::Stopped) => return,
                        Some(super::engine::Exit::Auth(why)) => {
                            shared.end(&uid, why, true);
                            return;
                        }
                        None => next_lock_try = Instant::now() + timing.crash_pause,
                    }
                    // Only now may another process take over.
                    drop(lock);
                }
                Ok(None) => {}
                Err(e) => {
                    // Without the lock there is no telling whether another
                    // engine runs; two engines are worse than none.
                    if !lock_error_logged {
                        super::log_line(&format!("cannot take the sync lock, not syncing: {e}"));
                        lock_error_logged = true;
                        let mut report = SyncReport::new(&uid);
                        report.status = SyncStatus::Error;
                        report.problem = Some(SyncProblem::Failed);
                        *locked(&shared.report) = report;
                    }
                }
            }
        }

        if let Some(report) = reader.poll() {
            // A report about another account is not ours to show.
            if report.uid == uid {
                *locked(&shared.report) = report;
            }
        }
        thread::sleep(timing.follow_tick);
    }
}

/// Run the engine as the owner. `None`: it panicked. That must not leave
/// this handle claiming ownership and a healthy status forever, so the
/// thread goes back to following, and the lock is released for another Fox.
fn own(store: &dyn Store, uid: &str, shared: &Shared) -> Option<super::engine::Exit> {
    store.adopt_session(&shared.places.session);
    shared.owner.store(true, Ordering::Relaxed);
    let exit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::engine::run(store, shared)
    }));
    shared.owner.store(false, Ordering::Relaxed);
    match exit {
        Ok(exit) => Some(exit),
        Err(_) => {
            super::log_line("the sync engine crashed; this process stops syncing for now");
            let mut report = SyncReport::new(uid);
            report.status = SyncStatus::Error;
            report.problem = Some(SyncProblem::Failed);
            shared.replace(report);
            None
        }
    }
}

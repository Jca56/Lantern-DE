// The sync owner's loop. Runs on the fox-cloud-sync thread of the one process
// that holds the owner lock (owner.rs), until it is told to stop or the
// account is signed out.
//
// A pass (refresh the remote mirror, then reconcile) runs when
//   - files under ~/Cloud changed and have been quiet for the debounce time,
//   - the remote poll is due, or
//   - the user answered a held-deletions question.
// The periods are in `Timing` (store.rs).
//
// QUOTA: Firestore bills one read per document returned. A full list of the
// collection is therefore rare: when the mirror has never been seeded for
// this account, and then once per full-list period (wall clock, persisted
// with the mirror, so restarting Fox does not list again). Every other pass
// is a delta query that returns only what changed since the last one, on the
// server's clock (remote_index.rs): a quiet poll is one read.
//
// The pass after a full list also cleans up: tombstones past their keep
// time and blobs nothing refers to (gc.rs).
//
// A pass that failed is not retried before a not-before time, and it cannot
// wake itself up: the watcher ignores events on paths sync ignores (its own
// download temp files among them).

use std::collections::HashSet;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use super::auth::AuthStop;
use super::failures;
use super::guard::HeldSet;
use super::http::status_of;
use super::reconcile::{self, PassError, PassInput};
use super::remote_index::RemoteIndex;
use super::scan::{self, RootError};
use super::session::SessionWatch;
use super::stamp::Micros;
use super::store::{Places, Store, Timing};
use super::sync::{Shared, SyncProblem, SyncReport, SyncStatus};
use super::watch::Watch;
use super::{firestore, guard, log_line};

mod requests;

pub(super) enum Exit {
    /// The handle asked the thread to stop.
    Stopped,
    /// Nobody is signed in any more.
    Auth(AuthStop),
}

// ── Loop state ─────────────────────────────────────────────────────────────

enum Outcome {
    Ok,
    /// ~/Cloud is missing or unreadable: nothing was attempted.
    Blocked,
    Err,
    Quota,
    Auth(AuthStop),
    Cancelled,
}

fn classify(e: &anyhow::Error) -> Outcome {
    if let Some(why) = super::auth::stop_reason(e) {
        return Outcome::Auth(why);
    }
    log_line(&format!("sync pass failed: {e}"));
    // Matched on ureq's exact wording for an HTTP status error: a bare
    // "429" also turns up inside URLs, sha256 hex and file names.
    if reconcile::is_quota_error(&format!("{e:#}")) {
        Outcome::Quota
    } else {
        Outcome::Err
    }
}

struct Engine<'a> {
    store: &'a dyn Store,
    shared: &'a Shared,
    places: &'a Places,
    timing: Timing,
    uid: String,
    index: RemoteIndex,
    watch: Watch,
    session: SessionWatch,
    /// Deletions the last pass held back, in full.
    held: Option<HeldSet>,
    /// Deletions the user confirmed, and until when that counts.
    approved: Option<(HashSet<String>, Instant)>,
    /// (first, latest) unsynced local change.
    dirty: Option<(Instant, Instant)>,
    next_poll: Instant,
    /// No pass before this (set by a failed pass or a quota pause).
    not_before: Option<Instant>,
    /// `not_before` is a quota pause: not even an answer overrides it.
    quota_paused: bool,
    err_backoff: Duration,
    quota_backoff: Duration,
    full_retry_at: Option<Instant>,
    /// Set while the server refuses the delta query: no remote refresh
    /// before this time, then a full list in place of the delta.
    list_instead_at: Option<Instant>,
    /// Scan notes of the previous pass: a note is logged when it is new.
    last_notes: HashSet<String>,
    /// How soon to look again at what the last pass had to leave for the
    /// next one (files still being written, docs that had changed in the
    /// cloud). Doubles while that keeps happening, up to the poll period: a
    /// file that is written to without end must not become a pass (and a
    /// Firestore read) every second.
    busy_wait: Duration,
    /// Set by a pass that left such things: poll again after this long.
    look_again: Option<Duration>,
    /// Wall-clock start of the last pass: an answer given while it ran is
    /// read when it ends (guard::take_answer).
    pass_started_ms: u64,
}

pub(super) fn run(store: &dyn Store, shared: &Shared) -> Exit {
    let uid = store.uid();
    let places = &shared.places;
    let timing = shared.timing;
    let mut engine = Engine {
        store,
        shared,
        places,
        timing,
        index: RemoteIndex::load_from(&places.index, &uid),
        watch: Watch::new(places.root.clone()),
        session: SessionWatch::new(places.session.clone(), &uid),
        uid,
        held: None,
        approved: None,
        dirty: None,
        next_poll: Instant::now(),
        not_before: None,
        quota_paused: false,
        err_backoff: timing.err_backoff_start,
        quota_backoff: timing.quota_backoff_start,
        full_retry_at: None,
        list_instead_at: None,
        last_notes: HashSet::new(),
        busy_wait: timing.debounce,
        look_again: None,
        pass_started_ms: u64::MAX,
    };
    log_line("this process now runs cloud sync");
    // Whatever a previous owner left in the status file is history, and
    // "synced" is not known until the first pass has looked.
    let mut report = SyncReport::new(&engine.uid);
    report.status = SyncStatus::Syncing;
    shared.replace(report);
    let exit = engine.run();
    log_line("cloud sync stopped in this process");
    exit
}

impl Engine<'_> {
    fn run(&mut self) -> Exit {
        loop {
            if self.shared.stopping() {
                return Exit::Stopped;
            }
            if !self.session.still_signed_in() {
                return Exit::Auth(AuthStop::SignedOut);
            }

            // Wait a tick, or less if a file event arrives.
            match self.watch.rx.recv_timeout(self.timing.tick) {
                Ok(first) => {
                    let mut matters = self.watch.matters(&first);
                    while let Ok(ev) = self.watch.rx.try_recv() {
                        matters |= self.watch.matters(&ev);
                    }
                    if matters {
                        self.mark_dirty();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(self.timing.tick),
            }

            let answered = self.take_answer() | self.take_retry();
            let now = Instant::now();
            if let Some(until) = self.not_before {
                if now < until && (self.quota_paused || !answered) {
                    continue;
                }
            }
            let settled = self.dirty.is_some_and(|(first, latest)| {
                now.duration_since(latest) >= self.timing.debounce
                    || now.duration_since(first) >= self.timing.settle_max
            });
            if !(answered || settled || now >= self.next_poll) {
                continue;
            }

            self.dirty = None;
            self.pass_started_ms = firestore::now_ms();
            let outcome = self.pass();
            let now = Instant::now();
            let wait = self.look_again.take().unwrap_or(self.timing.poll_every);
            self.next_poll = now + wait.min(self.timing.poll_every);
            self.not_before = None;
            self.quota_paused = false;
            match outcome {
                Outcome::Ok => {
                    self.err_backoff = self.timing.err_backoff_start;
                    self.quota_backoff = self.timing.quota_backoff_start;
                }
                // Checking for the folder again costs one stat: the normal
                // poll is slow enough.
                Outcome::Blocked | Outcome::Cancelled => {}
                Outcome::Err => {
                    self.not_before = Some(now + self.err_backoff);
                    self.err_backoff = (self.err_backoff * 2).min(self.timing.err_backoff_cap);
                }
                Outcome::Quota => {
                    log_line(&format!(
                        "quota exhausted (429/402) — sync paused {}s",
                        self.quota_backoff.as_secs()
                    ));
                    self.not_before = Some(now + self.quota_backoff);
                    self.quota_paused = true;
                    self.quota_backoff =
                        (self.quota_backoff * 2).min(self.timing.quota_backoff_cap);
                }
                Outcome::Auth(why) => return Exit::Auth(why),
            }
        }
    }

    fn mark_dirty(&mut self) {
        let now = Instant::now();
        self.dirty = Some((self.dirty.map_or(now, |(first, _)| first), now));
    }

    /// Refresh the remote mirror: a full list when one is due, else a delta
    /// query. On `Ok` the mirror is seeded and may be reconciled against;
    /// `Some(server time)` when it was listed in full just now, which is
    /// when the pass may clean up.
    fn fetch(&mut self) -> anyhow::Result<Option<Micros>> {
        let now_ms = firestore::now_ms();
        let must_list = !self.index.is_seeded();
        let full = must_list
            || (self.index.needs_full(now_ms, self.timing.full_every_ms)
                && self.full_retry_at.is_none_or(|t| Instant::now() >= t));
        if !full {
            if self.list_instead_at.is_some_and(|t| Instant::now() < t) {
                return Ok(None);
            }
            match self.store.changed_since(self.index.delta_since()) {
                Ok(pull) => {
                    self.index.apply_delta(pull.docs, pull.read_at);
                    self.list_instead_at = None;
                    return Ok(None);
                }
                // The server will not take the question as it is asked, and
                // asking again changes nothing. Sync must not stop over
                // that: a full list stands in for the delta, at most once
                // per `full_retry`, and the passes in between run on the
                // mirror as it is. That is safe (every write they make is
                // conditional on what the cloud really holds), just slower
                // to notice the other machine.
                Err(e) if status_of(&e).is_some_and(|st| st.code == 400) => {
                    if self.list_instead_at.is_none() {
                        log_line(&format!(
                            "the delta query was refused ({e}); listing the whole collection every {} s instead",
                            self.timing.full_retry.as_secs()
                        ));
                    }
                    self.list_instead_at = Some(Instant::now() + self.timing.full_retry);
                }
                Err(e) => return Err(e),
            }
        }
        // A list takes a while and is worth showing.
        self.shared.publish_status(SyncStatus::Syncing);
        // The server's clock, read before the list: the mirror's cursor
        // starts there, so whatever is written while the list is being read
        // comes again with the next delta.
        let listed = self
            .store
            .server_time()
            .and_then(|at| Ok((at, self.store.list_all()?)));
        match listed {
            Ok((server_time, docs)) => {
                self.index.seed_full(docs, server_time, now_ms);
                self.full_retry_at = None;
                Ok(server_time)
            }
            Err(e) => {
                // A list bills every page it got through. Do not start it
                // over on the next turn; delta passes go on if there is a
                // mirror to run them against.
                if !must_list {
                    self.full_retry_at = Some(Instant::now() + self.timing.full_retry);
                }
                Err(e)
            }
        }
    }

    /// The report for a pass that could not do its work at all.
    fn publish_failure(&self, outcome: &Outcome, problem: SyncProblem) {
        let mut report = self.shared.report();
        match outcome {
            Outcome::Quota => {
                report.status = SyncStatus::RateLimited;
                report.problem = None;
            }
            _ => {
                report.status = SyncStatus::Error;
                report.problem = Some(problem);
            }
        }
        self.shared.publish(report);
    }

    fn pass(&mut self) -> Outcome {
        // No folder, no pass — and no remote reads spent on one.
        if let Err(e) = scan::check_root(&self.places.root) {
            return self.blocked(e);
        }
        // Attached before the scan below: what changed earlier the scan
        // sees, what changes later the watcher reports.
        self.watch.ensure();

        let maintain = match self.fetch() {
            Ok(maintain) => maintain,
            Err(e) => {
                let outcome = classify(&e);
                if !matches!(outcome, Outcome::Auth(_)) {
                    self.publish_failure(&outcome, SyncProblem::Failed);
                }
                return outcome;
            }
        };
        if let Err(e) = self.index.save_to(&self.places.index) {
            log_line(&format!("could not save the remote index: {e}"));
        }

        let approved = match self.approved.take() {
            Some((set, until)) if Instant::now() < until => set,
            _ => HashSet::new(),
        };
        let shared = self.shared;
        let session = &mut self.session;
        let result = reconcile::run_pass(PassInput {
            store: self.store,
            places: self.places,
            index: &mut self.index,
            approved: &approved,
            cancel: &mut || shared.stopping() || !session.still_signed_in(),
            on_work: &mut || shared.publish_status(SyncStatus::Syncing),
            busy_window: self.timing.busy_window,
            maintain,
        });
        let pass = match result {
            Ok(pass) => pass,
            Err(PassError::Root(e)) => return self.blocked(e),
            Err(PassError::Other(e)) => {
                let outcome = classify(&e);
                self.publish_failure(&outcome, SyncProblem::Failed);
                return outcome;
            }
        };

        // Each distinct scan problem is logged when it shows up, not on
        // every pass for as long as it lasts.
        let notes: HashSet<String> = pass.notes.iter().cloned().collect();
        for note in notes.difference(&self.last_notes) {
            log_line(note);
        }
        self.last_notes = notes;

        if let Some(why) = pass.auth_stop {
            return Outcome::Auth(why);
        }
        if pass.held.as_ref().map(|h| &h.id) != self.held.as_ref().map(|h| &h.id) {
            if let Some(h) = &pass.held {
                // The whole set, where the windows can read it, before the
                // report that makes them ask about it.
                if let Err(e) = guard::write_held_list(&self.places.held_list, h) {
                    log_line(&format!(
                        "could not write the list of held deletions ({e}): they can be restored, not confirmed"
                    ));
                }
                log_line(&format!(
                    "{} of {} synced file(s) are gone from {} ({:?}): deletions held until confirmed; the full list is in {}",
                    h.paths.len(),
                    h.total,
                    self.places.root.display(),
                    h.reason,
                    self.places.held_list.display()
                ));
            }
        }
        self.held = pass.held;
        if self.held.is_none() {
            guard::clear_held_list(&self.places.held_list);
            // Nothing is being asked: an answer still lying around is stale.
            let _ = guard::take_answer(&self.places.answer, None, u64::MAX);
        }

        if !pass.failures.is_empty() {
            log_line(&format!(
                "{} file(s) failed to sync: {}",
                pass.failures.len(),
                // Each failure is already logged on its own line; the
                // summary stays short however many files failed.
                pass.failures
                    .iter()
                    .take(3)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }

        let cleaned = pass.cleaned;
        if cleaned.tombstones + cleaned.blobs + cleaned.restored > 0 {
            log_line(&format!(
                "clean-up: {} old tombstone(s) and {} unused blob(s) removed, {} missing blob(s) restored",
                cleaned.tombstones, cleaned.blobs, cleaned.restored
            ));
        }

        // A file that was still being written is looked at again soon: its
        // last write may have been the one just before this pass, with no
        // further event to come. So is a path whose doc in the cloud turned
        // out to have changed: the mirror has the new doc, nothing else
        // will announce it.
        if pass.busy > 0 || pass.moved > 0 {
            self.look_again = Some(self.busy_wait);
            self.busy_wait = (self.busy_wait * 2).min(self.timing.poll_every);
        } else {
            self.busy_wait = self.timing.debounce;
        }

        let mut report = SyncReport::new(&self.uid);
        report.held = self.held.as_ref().map(HeldSet::summary);
        report.unreadable = pass.unreadable;
        report.failed = pass.failures.len();
        report.stuck = pass.stuck.len();
        report.stuck_sample = pass.stuck.iter().take(failures::SAMPLE_MAX).cloned().collect();
        report.problem = if report.held.is_some() {
            Some(SyncProblem::DeletionsHeld)
        } else if !pass.failures.is_empty() && !pass.quota {
            Some(SyncProblem::Failed)
        } else if pass.unreadable > 0 {
            Some(SyncProblem::Unreadable)
        } else if report.stuck > 0 {
            Some(SyncProblem::CannotSync)
        } else {
            None
        };
        report.status = if pass.quota {
            SyncStatus::RateLimited
        } else if report.problem.is_some() {
            SyncStatus::Error
        } else if pass.deferred > 0 || pass.busy > 0 || pass.cancelled {
            // Files changed under the pass (or it was cut short): not in
            // sync yet, the next pass finishes the job.
            SyncStatus::Syncing
        } else {
            SyncStatus::Idle
        };
        self.shared.publish(report);

        if pass.cancelled {
            Outcome::Cancelled
        } else if pass.quota {
            Outcome::Quota
        } else if !pass.failures.is_empty() {
            Outcome::Err
        } else {
            // Held deletions and unreadable files are not failures to retry
            // harder: nothing changes until the user or the disk does.
            Outcome::Ok
        }
    }

    fn blocked(&mut self, e: RootError) -> Outcome {
        let (problem, why) = match &e {
            RootError::Missing => (SyncProblem::FolderMissing, "is missing".to_string()),
            RootError::Unreadable(msg) => (
                SyncProblem::FolderUnreadable,
                format!("cannot be read: {msg}"),
            ),
        };
        if self.shared.report().problem != Some(problem) {
            log_line(&format!(
                "{} {why} — sync is paused, nothing is deleted",
                self.places.root.display()
            ));
        }
        // What was held was computed from a folder that is no longer there
        // to look at; it is recomputed when the folder is back, and asked
        // about again.
        self.held = None;
        self.approved = None;
        guard::clear_held_list(&self.places.held_list);
        let _ = guard::take_answer(&self.places.answer, None, u64::MAX);
        let mut report = SyncReport::new(&self.uid);
        report.status = SyncStatus::Error;
        report.problem = Some(problem);
        self.shared.publish(report);
        Outcome::Blocked
    }
}

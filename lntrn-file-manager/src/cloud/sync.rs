// Reconciliation loop. Spawned once per signed-in session.
//
// The thread does:
//   1. on start: pull-then-reconcile (Firestore list_all + walk ~/Cloud)
//   2. main loop: wait for either
//        - an inotify event (debounced ~2s) → reconcile (local changes sync
//          within seconds)
//        - the periodic poll tick (hourly) → re-pull Firestore for changes
//          made on the other machine
//
// QUOTA: every reconcile is a full Firestore list, billed one READ PER
// DOCUMENT — with ~1.4k files in ~/Cloud that's ~1.4k reads per pass. The
// original 10s poll burned the free tier's 50k daily reads in minutes and
// then 429'd until midnight PT. Hence the hourly poll + the 429 backoff
// below. The proper fix (TODO) is an incremental pull via a `runQuery` on
// `updated_at > last_pull` — ~1 read per poll — which would let the poll
// tighten back up.
//
// This is intentionally simple — no delta queue, no per-file mutex. The
// reconciler is idempotent (sha256 keyed), so re-running it is safe.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::http::Authed;
use super::{cloud_root, CloudConfig, Session};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Syncing,
    Error,
    Offline,
    /// Firestore quota exhausted (HTTP 429) — sync is intentionally paused
    /// and will retry with backoff. Not an error; quotas reset daily.
    RateLimited,
}

pub struct SyncHandle {
    pub status: Arc<Mutex<SyncStatus>>,
    _stop: Arc<std::sync::atomic::AtomicBool>,
}

impl SyncHandle {
    /// Cheap snapshot of the current sync status. Copies under the mutex.
    pub fn status(&self) -> SyncStatus {
        *self.status.lock().unwrap()
    }
}

impl SyncHandle {
    /// Spawn the background sync thread. Returns immediately.
    pub fn spawn(cfg: Arc<CloudConfig>, session: Arc<Mutex<Session>>) -> Self {
        let status = Arc::new(Mutex::new(SyncStatus::Idle));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let authed = Authed { cfg, session };
        let status_thr = status.clone();
        let stop_thr = stop.clone();

        thread::Builder::new()
            .name("fox-cloud-sync".into())
            .spawn(move || run_loop(authed, status_thr, stop_thr))
            .expect("spawn fox-cloud-sync thread");

        Self {
            status,
            _stop: stop,
        }
    }
}

fn run_loop(
    authed: Authed,
    status: Arc<Mutex<SyncStatus>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    // Ensure ~/Cloud exists.
    if let Err(e) = std::fs::create_dir_all(cloud_root()) {
        super::log_line(&format!("cannot create cloud root: {e}"));
        *status.lock().unwrap() = SyncStatus::Error;
        return;
    }

    // ── Filesystem watcher ─────────────────────────────────────────────
    // notify gives us recursive inotify events. We don't act per-event — instead,
    // any event sets `dirty=true` and the main loop runs a full reconcile on the
    // next debounce window. Reconcile is idempotent so this is safe.
    let (fs_tx, fs_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(move |res| {
        let _ = fs_tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            super::log_line(&format!("watcher init failed: {e}"));
            *status.lock().unwrap() = SyncStatus::Error;
            return;
        }
    };
    use notify::Watcher;
    if let Err(e) = watcher.watch(&cloud_root(), notify::RecursiveMode::Recursive) {
        super::log_line(&format!("watch ~/Cloud failed: {e}"));
    }

    // Remote poll cadence. A delta poll costs ~1 Firestore read (see
    // firestore::query_changed_since), so 30s is safely inside the free
    // tier — near-instant pickup of the other machine's edits.
    const POLL_EVERY: Duration = Duration::from_secs(30);
    // Drift-healing full list: catches docs written by pre-delta builds
    // (no updated_at) and any index corruption. One collection-sized read
    // burst, a few times a day.
    const FULL_EVERY: Duration = Duration::from_secs(6 * 3600);
    const DEBOUNCE: Duration = Duration::from_millis(1500);
    // 429 backoff: first pause 10 min, doubling to a 2 h cap.
    const BACKOFF_START: Duration = Duration::from_secs(600);
    const BACKOFF_CAP: Duration = Duration::from_secs(7200);

    let mut last_poll = std::time::Instant::now();
    let mut last_full = std::time::Instant::now();
    let mut backoff_until: Option<std::time::Instant> = None;
    let mut backoff_len = BACKOFF_START;
    // Local changes that arrived while backed off — reconciled as soon as
    // the pause lifts instead of waiting for the next poll.
    let mut pending_dirty = false;

    let mut index = super::remote_index::RemoteIndex::load();

    // Startup: full pull-then-reconcile so a freshly-signed-in machine (and
    // the index mirror) catch up with the cloud before the user looks at it.
    if let Outcome::RateLimited = sync_pass(&authed, &status, &mut index, true) {
        super::log_line(&format!(
            "quota exhausted (429) — sync paused {}s",
            backoff_len.as_secs()
        ));
        backoff_until = Some(std::time::Instant::now() + backoff_len);
        backoff_len = (backoff_len * 2).min(BACKOFF_CAP);
    }

    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Drain any fs events; if we got at least one, run reconcile after the
        // debounce window. Bare recv_timeout = sleeping with quick wake on event.
        match fs_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(_ev) => {
                pending_dirty = true;
                // Drain bursts.
                while let Ok(_) = fs_rx.try_recv() {}
                // Debounce: wait for events to settle.
                thread::sleep(DEBOUNCE);
                while let Ok(_) = fs_rx.try_recv() {}
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // While backed off, do nothing — pending_dirty holds any local
        // changes for the moment the pause lifts.
        if let Some(until) = backoff_until {
            if std::time::Instant::now() < until {
                continue;
            }
            backoff_until = None;
        }

        let full_due = last_full.elapsed() >= FULL_EVERY;
        let poll_due = last_poll.elapsed() >= POLL_EVERY;
        if pending_dirty || poll_due || full_due {
            match sync_pass(&authed, &status, &mut index, full_due) {
                Outcome::Ok => {
                    pending_dirty = false;
                    backoff_len = BACKOFF_START;
                    if full_due {
                        last_full = std::time::Instant::now();
                    }
                }
                Outcome::RateLimited => {
                    super::log_line(&format!(
                        "quota exhausted (429) — sync paused {}s",
                        backoff_len.as_secs()
                    ));
                    backoff_until = Some(std::time::Instant::now() + backoff_len);
                    backoff_len = (backoff_len * 2).min(BACKOFF_CAP);
                    // keep pending_dirty — retry after the pause
                }
                Outcome::Err => {
                    // Non-quota failure (network, auth, partial file errors):
                    // drop the dirty flag so we don't hot-loop; the next poll
                    // (or fs event) retries naturally.
                    pending_dirty = false;
                }
            }
            last_poll = std::time::Instant::now();
        }
    }
}

enum Outcome {
    Ok,
    Err,
    RateLimited,
}

/// One sync pass: refresh the remote mirror (full list or ~1-read delta
/// query), persist it, then three-way reconcile against it.
fn sync_pass(
    authed: &Authed,
    status: &Arc<Mutex<SyncStatus>>,
    index: &mut super::remote_index::RemoteIndex,
    full: bool,
) -> Outcome {
    *status.lock().unwrap() = SyncStatus::Syncing;

    let fetched = if full {
        super::firestore::list_all(authed).map(|docs| {
            index.seed_full(docs);
        })
    } else {
        let since = index
            .cursor_ms
            .saturating_sub(super::remote_index::OVERLAP_MS);
        super::firestore::query_changed_since(authed, since).map(|docs| {
            index.apply_delta(docs);
        })
    };
    if let Err(e) = fetched {
        return classify_failure(status, &e);
    }
    let _ = index.save();

    match super::reconcile::reconcile_with(authed, &index.docs) {
        Ok(()) => {
            *status.lock().unwrap() = SyncStatus::Idle;
            Outcome::Ok
        }
        Err(e) => classify_failure(status, &e),
    }
}

fn classify_failure(status: &Arc<Mutex<SyncStatus>>, e: &anyhow::Error) -> Outcome {
    super::log_line(&format!("reconcile failed: {e}"));
    if format!("{e:#}").contains("429") {
        *status.lock().unwrap() = SyncStatus::RateLimited;
        Outcome::RateLimited
    } else {
        *status.lock().unwrap() = SyncStatus::Error;
        Outcome::Err
    }
}

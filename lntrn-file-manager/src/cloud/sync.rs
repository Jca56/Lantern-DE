// Reconciliation loop. Spawned once per signed-in session.
//
// The thread does:
//   1. on start: pull-then-reconcile (Firestore list_all + walk ~/Cloud)
//   2. main loop: wait for either
//        - an inotify event (debounced ~2s) → reconcile changed paths
//        - the periodic poll tick (every 10s) → re-pull Firestore for remote changes
//
// This is intentionally simple — no delta queue, no per-file mutex. The reconciler
// is idempotent (sha256 keyed), so re-running it on the same path is cheap and safe.
//
// Wiring the actual notify::Watcher + reconciliation logic is the next milestone;
// for now this module exposes the public surface the rest of the app will call.

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

        Self { status, _stop: stop }
    }
}

fn run_loop(
    authed: Authed,
    status: Arc<Mutex<SyncStatus>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    // Ensure ~/Cloud exists.
    if let Err(e) = std::fs::create_dir_all(cloud_root()) {
        eprintln!("[fox-cloud] cannot create cloud root: {e}");
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
            eprintln!("[fox-cloud] watcher init failed: {e}");
            *status.lock().unwrap() = SyncStatus::Error;
            return;
        }
    };
    use notify::Watcher;
    if let Err(e) = watcher.watch(&cloud_root(), notify::RecursiveMode::Recursive) {
        eprintln!("[fox-cloud] watch ~/Cloud failed: {e}");
    }

    // Run an immediate pull-then-reconcile so a freshly-signed-in machine
    // catches up with the cloud before the user looks at it.
    do_reconcile(&authed, &status);

    const POLL_EVERY: Duration = Duration::from_secs(10);
    const DEBOUNCE: Duration = Duration::from_millis(1500);
    let mut last_poll = std::time::Instant::now();

    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Drain any fs events; if we got at least one, run reconcile after the
        // debounce window. Bare recv_timeout = sleeping with quick wake on event.
        let mut dirty = false;
        match fs_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(_ev) => {
                dirty = true;
                // Drain bursts.
                while let Ok(_) = fs_rx.try_recv() {}
                // Debounce: wait for events to settle.
                thread::sleep(DEBOUNCE);
                while let Ok(_) = fs_rx.try_recv() {}
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let poll_due = last_poll.elapsed() >= POLL_EVERY;
        if dirty || poll_due {
            do_reconcile(&authed, &status);
            last_poll = std::time::Instant::now();
        }
    }
}

fn do_reconcile(authed: &Authed, status: &Arc<Mutex<SyncStatus>>) {
    *status.lock().unwrap() = SyncStatus::Syncing;
    match super::reconcile::reconcile_all(authed) {
        Ok(()) => *status.lock().unwrap() = SyncStatus::Idle,
        Err(e) => {
            eprintln!("[fox-cloud] reconcile failed: {e}");
            *status.lock().unwrap() = SyncStatus::Error;
        }
    }
}

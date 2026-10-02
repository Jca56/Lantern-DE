// Cloud sync for ~/Cloud/.
//
// Architecture:
//   config.rs    — loads ~/.lantern/config/fox-cloud.json (api_key, project_id, bucket)
//   session.rs   — auth state shared across threads, refresh-on-demand; the
//                  session file is also the sign-out signal for every process
//   auth.rs      — Firebase Auth REST (signInWithPassword, securetoken refresh)
//   http.rs      — ureq wrapper that attaches the bearer token
//   storage.rs   — Firebase Storage REST: PUT/GET blobs by sha256
//   firestore.rs — Firestore REST: per-file metadata docs, stamped by the
//                  server and written conditionally
//   stamp.rs     — server times as numbers
//   transfer.rs  — streamed blob transfers, verified against their hash
//   manifest.rs  — local sha256 ledger at ~/.lantern/cloud/manifest.json
//   remote_index.rs — local mirror of the Firestore collection
//   hash.rs      — sha256 of a file
//   ignore.rs    — which paths sync leaves alone
//   scan.rs      — the local scan; a read failure is "unknown", never "deleted"
//   plan.rs      — the three-way decision table (pure)
//   guard.rs     — holds mass deletions back until the user confirms
//   store.rs     — the cloud operations and disk places a pass is handed
//   reconcile.rs — one pass: scan, plan, guard, then act path by path
//   actions.rs   — what is done to one path (upload, download, delete...)
//   failures.rs  — files that keep failing: remembered, retried ever later
//   gc.rs        — removes old tombstones and blobs nothing refers to
//   trash.rs     — where a file deleted by the other machine goes
//   owner.rs     — one sync engine per machine (flock) + the status file
//   watch.rs     — the file watcher, minus the events sync causes itself
//   engine.rs    — the owner's loop: polls, backoff (engine/requests.rs:
//                  what the windows ask of it)
//   sync.rs      — `SyncHandle`: what the UI holds and asks
//
// On disk:
//   ~/Cloud/                                   the synced folder
//   ~/.lantern/config/fox-cloud.json           Firebase project config
//   ~/.lantern/config/fox-cloud-session.json   the sign-in (+ .lock)
//   ~/.lantern/cloud/manifest.json             what was last agreed, per path
//   ~/.lantern/cloud/remote-index.json         mirror of the remote docs
//   ~/.lantern/cloud/sync.lock                 held by the one syncing process
//   ~/.lantern/cloud/status.json               its report, for the others
//   ~/.lantern/cloud/deletions-answer.json     confirm/decline, for the owner
//   ~/.lantern/cloud/failures.json             what keeps failing, and until when
//   ~/.lantern/cloud/retry-now                 "try those again", for the owner
//   ~/.lantern/cloud/*.corrupt, *.other-account   state files set aside
//
// The public surface is `CloudState`, owned by `App` behind an `Option`. None means
// "not signed in yet" — UI shows a login prompt. Some means a background sync thread
// is running.

mod actions;
pub mod auth;
pub mod config;
mod engine;
pub mod failures;
pub mod firestore;
mod gc;
pub mod guard;
pub mod hash;
pub mod http;
mod ignore;
pub mod manifest;
mod owner;
#[cfg(test)]
mod pass_tests;
mod plan;
pub mod reconcile;
pub mod remote_index;
mod scan;
pub mod session;
pub mod stamp;
pub mod storage;
mod store;
pub mod sync;
mod transfer;
mod trash;
mod watch;
#[cfg(test)]
mod wire_tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use config::CloudConfig;
pub use session::Session;

/// The local root for synced files.
pub fn cloud_root() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("Cloud")
    } else {
        PathBuf::from("/tmp/Cloud")
    }
}

/// Ensure ~/Cloud/ exists. Idempotent. For the UI (the sidebar entry, the
/// Cloud button). The sync engine never calls this: to it a missing root is
/// an error, and a root that was just re-created empty holds its deletions
/// back (see guard.rs).
pub fn ensure_cloud_dir() -> std::io::Result<PathBuf> {
    let p = cloud_root();
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// Where sync keeps its state: manifest, remote mirror, owner lock, status.
pub(crate) fn state_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".lantern/cloud")
}

/// Per-machine identifier used for conflict-rename suffixes. The kernel's
/// hostname first: Gentoo keeps its hostname in /etc/conf.d, so /etc/hostname
/// does not exist there and every doc was labelled "device".
pub fn device_name() -> String {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty() && s != "(none)")
        .unwrap_or_else(|| "device".to_string())
}

/// Write `bytes` to `path` so a crash or a second Fox process never leaves a
/// half-written file: temp file beside it (per-process name), fsync, rename.
/// `mode` applies from creation, so a secret is never briefly world-readable.
pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    write_replacing(path, bytes, mode, true)
}

/// `write_atomic`, with the fsync optional: a file that is only a cache of
/// what the running process knows (the status file) need not survive a power
/// cut, and need not cost a disk flush every time it changes.
pub(crate) fn write_replacing(
    path: &std::path::Path,
    bytes: &[u8],
    mode: u32,
    durable: bool,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(bytes)?;
        if durable {
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// fox-cloud.log is rotated to `.1` past this size.
const LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Append a line to ~/.lantern/log/fox-cloud.log and echo to stderr. Best-effort:
/// never panics, swallows its own I/O errors. This is the audit trail for sync —
/// per-file uploads/downloads and any failures land here so problems are visible.
pub fn log_line(msg: &str) {
    use std::io::Write;
    eprintln!("[fox-cloud] {msg}");
    // Unit tests exercise the failure paths on purpose; their lines do not
    // belong in the real log.
    if cfg!(test) {
        return;
    }
    let path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".lantern/log/fox-cloud.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Rotate by rename: another Fox process appending keeps a valid fd.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

/// Top-level state owned by App. None = not signed in. Some = sync thread running.
pub struct CloudState {
    pub config: Arc<CloudConfig>,
    pub session: Arc<Mutex<Session>>,
}

impl CloudState {
    /// Try to initialize from on-disk config + cached session. Returns None if the
    /// config file is missing or the user has never signed in successfully.
    pub fn try_load() -> Option<Self> {
        let config = Arc::new(CloudConfig::load().ok()?);
        let session = Session::load_cached(&config).ok()?;
        Some(Self {
            config,
            session: Arc::new(Mutex::new(session)),
        })
    }
}

/// A scratch directory for one test, removed again when the test is done.
#[cfg(test)]
pub(crate) struct TestDir(PathBuf);

#[cfg(test)]
impl TestDir {
    /// `name` has to be unique among the tests of the crate.
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lntrn-fm-cloud-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        Self(dir)
    }
}

#[cfg(test)]
impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

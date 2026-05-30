// Cloud sync for ~/Cloud/.
//
// Architecture:
//   config.rs    — loads ~/.lantern/config/fox-cloud.json (api_key, project_id, bucket)
//   session.rs   — auth state shared across threads, refresh-on-demand
//   auth.rs      — Firebase Auth REST (signInWithPassword, securetoken refresh)
//   http.rs      — ureq wrapper that attaches the bearer token
//   storage.rs   — Firebase Storage REST: PUT/GET blobs by sha256
//   firestore.rs — Firestore REST: per-file metadata docs
//   manifest.rs  — local sha256 ledger at ~/.lantern/cloud/manifest.json
//   sync.rs      — three-way merge + inotify watcher + periodic remote poll
//   hash.rs      — sha256 of a file
//
// The public surface is `CloudState`, owned by `App` behind an `Option`. None means
// "not signed in yet" — UI shows a login prompt. Some means a background sync thread
// is running.

pub mod auth;
pub mod config;
pub mod firestore;
pub mod hash;
pub mod http;
pub mod manifest;
pub mod reconcile;
pub mod session;
pub mod storage;
pub mod sync;

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

/// Ensure ~/Cloud/ exists. Idempotent.
pub fn ensure_cloud_dir() -> std::io::Result<PathBuf> {
    let p = cloud_root();
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// Per-machine identifier used for conflict-rename suffixes.
/// Reads /etc/hostname or falls back to "device".
pub fn device_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "device".to_string())
}

/// Append a line to ~/.lantern/log/fox-cloud.log and echo to stderr. Best-effort:
/// never panics, swallows its own I/O errors. This is the audit trail for sync —
/// per-file uploads/downloads and any failures land here so problems are visible.
pub fn log_line(msg: &str) {
    use std::io::Write;
    eprintln!("[fox-cloud] {msg}");
    let path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".lantern/log/fox-cloud.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
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

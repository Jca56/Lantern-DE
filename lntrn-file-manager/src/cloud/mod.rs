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
pub mod remote_index;
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
        f.sync_all()?;
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

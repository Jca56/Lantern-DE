// What sync talks to, and where it works.
//
// Sync needs a handful of things from the cloud (whose account, list the
// docs, store and fetch a blob, write a metadata doc) and a handful of places
// on disk. Both are handed to it instead of being reached for, so the logic
// that decides what gets overwritten and deleted — a reconcile pass, the
// engine loop, the owner hand-over — runs in tests against an in-memory cloud
// and scratch directories, with the real ~/Cloud, the real state files and
// the network nowhere in sight.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::firestore::{self, Expect, FileDoc, Pull, Put};
use super::http::Authed;
use super::stamp::Micros;
use super::storage::{self, BlobDelete};
use super::transfer::BlobSource;

pub(super) trait Store: Send + Sync {
    /// The signed-in account.
    fn uid(&self) -> String;
    /// Every doc of the account. Billed one read per doc.
    fn list_all(&self) -> anyhow::Result<Vec<FileDoc>>;
    /// The docs the server stamped after `since`, and its time of the read.
    /// Billed one read per doc, at least one.
    fn changed_since(&self, since: Micros) -> anyhow::Result<Pull>;
    /// The server's clock (one billed read). `None`: it did not say.
    fn server_time(&self) -> anyhow::Result<Option<Micros>>;
    /// The doc for one path as it is now (one billed read).
    fn get_doc(&self, path: &str) -> anyhow::Result<Option<FileDoc>>;
    /// Create or replace the metadata doc for `doc.path`, provided the cloud
    /// still holds what `expect` names.
    fn put_doc(&self, doc: &FileDoc, expect: &Expect) -> anyhow::Result<Put>;
    /// Remove the doc for `path` if it is still `version`. False: it is not.
    fn delete_doc(&self, path: &str, version: &str) -> anyhow::Result<bool>;
    /// Does any doc name this content right now? Asked of the server.
    fn sha_in_use(&self, sha256: &str) -> anyhow::Result<bool>;
    /// Store the file behind `src` as the blob named by its hash, reading
    /// it through `src.open()` (which refuses to complete a changed file).
    fn upload_blob(&self, src: &BlobSource, mime: &str) -> anyhow::Result<()>;
    /// Write the blob into `out`. Unverified: the caller checks the hash.
    fn download_blob(&self, sha256: &str, out: &mut dyn Write) -> anyhow::Result<()>;
    /// The names (sha256) of all stored blobs.
    fn list_blobs(&self) -> anyhow::Result<Vec<String>>;
    /// When a blob was last written, server clock. `None`: unknown or gone.
    fn blob_written_at(&self, sha256: &str) -> anyhow::Result<Option<Micros>>;
    fn delete_blob(&self, sha256: &str) -> anyhow::Result<BlobDelete>;
    /// Called when this process becomes the sync owner: another process may
    /// have refreshed the tokens in `session_file` since this one read them.
    fn adopt_session(&self, _session_file: &Path) {}
}

impl Store for Authed {
    fn uid(&self) -> String {
        self.user_id()
    }

    fn list_all(&self) -> anyhow::Result<Vec<FileDoc>> {
        firestore::list_all(self)
    }

    fn changed_since(&self, since: Micros) -> anyhow::Result<Pull> {
        firestore::query_changed_since(self, since)
    }

    fn server_time(&self) -> anyhow::Result<Option<Micros>> {
        firestore::server_time(self)
    }

    fn get_doc(&self, path: &str) -> anyhow::Result<Option<FileDoc>> {
        firestore::get(self, path)
    }

    fn put_doc(&self, doc: &FileDoc, expect: &Expect) -> anyhow::Result<Put> {
        firestore::put(self, doc, expect)
    }

    fn delete_doc(&self, path: &str, version: &str) -> anyhow::Result<bool> {
        firestore::delete(self, path, version)
    }

    fn sha_in_use(&self, sha256: &str) -> anyhow::Result<bool> {
        firestore::sha_in_use(self, sha256)
    }

    fn upload_blob(&self, src: &BlobSource, mime: &str) -> anyhow::Result<()> {
        storage::upload_blob(self, src, mime)
    }

    fn download_blob(&self, sha256: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        storage::download_blob(self, sha256, out)
    }

    fn list_blobs(&self) -> anyhow::Result<Vec<String>> {
        storage::list_blobs(self)
    }

    fn blob_written_at(&self, sha256: &str) -> anyhow::Result<Option<Micros>> {
        storage::blob_written_at(self, sha256)
    }

    fn delete_blob(&self, sha256: &str) -> anyhow::Result<BlobDelete> {
        storage::delete_blob(self, sha256)
    }

    fn adopt_session(&self, session_file: &Path) {
        let on_disk = std::fs::read(session_file)
            .ok()
            .and_then(|raw| serde_json::from_slice::<super::Session>(&raw).ok());
        if let Some(on_disk) = on_disk {
            let mut mine = self.session.lock().unwrap_or_else(|e| e.into_inner());
            if on_disk.uid == mine.uid {
                *mine = on_disk;
            }
        }
    }
}

/// The directories and files of one sync setup.
#[derive(Debug, Clone)]
pub(super) struct Places {
    /// The synced folder (~/Cloud). Never created by sync.
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub index: PathBuf,
    /// The freedesktop Trash that locally deleted files are moved into.
    pub trash: PathBuf,
    /// The cached sign-in. Its disappearance is the sign-out signal.
    pub session: PathBuf,
    /// Whoever holds a flock on this file is the sync owner.
    pub lock: PathBuf,
    /// The owner's report, for the other processes.
    pub status: PathBuf,
    /// The complete list of the deletions being held (guard.rs).
    pub held_list: PathBuf,
    /// The user's answer to held deletions, for the owner.
    pub answer: PathBuf,
    /// Which paths keep failing, and when each may be tried again.
    pub failures: PathBuf,
    /// "Try the failing files again now", for the owner.
    pub retry: PathBuf,
}

impl Places {
    pub fn real() -> Self {
        let state = super::state_dir();
        Self {
            root: super::cloud_root(),
            manifest: super::manifest::Manifest::path(),
            index: super::remote_index::RemoteIndex::path(),
            // The one Fox's Trash view lists and restores from.
            trash: crate::trash::home_trash().dir,
            session: super::Session::path(),
            lock: state.join("sync.lock"),
            status: state.join("status.json"),
            held_list: state.join("held-deletions.json"),
            answer: state.join("deletions-answer.json"),
            failures: state.join("failures.json"),
            retry: state.join("retry-now"),
        }
    }
}

/// Every wait and period of the sync threads. One real set; tests run the
/// same loops on a faster clock.
#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    /// One engine loop turn: how long a stop request, a sign-out or an
    /// answer waits at most before the loop notices.
    pub tick: Duration,
    /// Remote poll cadence. A quiet delta poll costs one Firestore read.
    pub poll_every: Duration,
    /// Drift-healing full list: catches anything a delta pull missed and
    /// any mirror corruption, and is when old tombstones and unreferenced
    /// blobs are cleaned up (gc.rs). This machine's wall clock, persisted
    /// with the mirror, so a restart does not list again.
    pub full_every_ms: u64,
    /// A periodic full list that failed for a non-quota reason is tried
    /// again after this long; delta passes carry on meanwhile.
    pub full_retry: Duration,
    /// Local changes are synced once the tree has been quiet this long...
    pub debounce: Duration,
    /// ...or, when it never goes quiet, this long after the first change.
    pub settle_max: Duration,
    /// A file written to less than this long ago is taken to be still in
    /// the middle of a copy or a download and is not hashed yet (scan.rs).
    /// Shorter than `debounce`, or a plain save would be put off.
    pub busy_window: Duration,
    /// Not-before time after a failed pass: doubling from start to cap.
    pub err_backoff_start: Duration,
    pub err_backoff_cap: Duration,
    /// Quota (429/402) pause: doubling from start to cap.
    pub quota_backoff_start: Duration,
    pub quota_backoff_cap: Duration,
    /// A confirmation that could not be used within this long is void.
    pub approval_ttl: Duration,
    /// How often a follower looks at the owner's report and at the session.
    pub follow_tick: Duration,
    /// How often a follower tries to become the owner.
    pub lock_retry: Duration,
    /// After the engine crashed in this process, how long before it may try
    /// again (another Fox is free to take over at once).
    pub crash_pause: Duration,
}

impl Timing {
    pub const REAL: Timing = Timing {
        tick: Duration::from_millis(250),
        poll_every: Duration::from_secs(30),
        full_every_ms: 6 * 3600 * 1000,
        full_retry: Duration::from_secs(900),
        debounce: Duration::from_millis(1500),
        settle_max: Duration::from_secs(20),
        busy_window: Duration::from_secs(1),
        err_backoff_start: Duration::from_secs(30),
        err_backoff_cap: Duration::from_secs(300),
        quota_backoff_start: Duration::from_secs(600),
        quota_backoff_cap: Duration::from_secs(7200),
        approval_ttl: Duration::from_secs(600),
        follow_tick: Duration::from_millis(500),
        lock_retry: Duration::from_secs(3),
        crash_pause: Duration::from_secs(120),
    };
}

pub(super) fn mime_for(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" | "rs" | "toml" | "json" | "yaml" | "yml" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

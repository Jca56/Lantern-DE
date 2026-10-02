// Files that cannot be synced, and when to try them again.
//
// A path whose upload or download fails used to be tried again on every
// pass: hashed, read and sent in full every 30 seconds for as long as the
// cause lasted (one file failed 3523 times in three weeks). Now a failure is
// remembered per path, at ~/.lantern/cloud/failures.json, with a wait that
// doubles each time. Until the wait is over the path is skipped, without any
// I/O, unless something about it changed:
//   - the local file (size, mtime, inode),
//   - the remote doc (content, version), or
//   - what sync wants to do with it.
// Any of those makes it a different problem, and it is tried at once.
//
// The record is persisted so a restart does not re-send everything, and it
// is read at the start of every pass, so removing the file (or
// `SyncHandle::retry_failed`) makes the next pass try everything again.
//
// A file too big for the cloud (storage::MAX_BLOB_BYTES) never gets here:
// the scan sets it aside without reading it, as it does with what it cannot
// read. All three kinds are reported through `SyncReport::stuck` so the UI
// can say "N files cannot be synced".

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::firestore::FileDoc;
use super::http::status_of;
use super::plan::Action;
use super::scan::LocalFile;

/// The wait after the first failure; doubled by each further one.
const FIRST_WAIT_MS: u64 = 60 * 1000;
/// Longest wait when the trouble may pass by itself (network, disk, server).
const MAX_WAIT_MS: u64 = 60 * 60 * 1000;
/// Longest wait when the server refused the request outright (4xx): trying
/// again changes nothing until the file or the rules do, and every try may
/// send the whole file.
const MAX_WAIT_REFUSED_MS: u64 = 6 * 60 * 60 * 1000;
/// Paths listed in a report; the count is exact.
pub(super) const SAMPLE_MAX: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StuckKind {
    /// 1 GiB or larger: the cloud does not take it. Not tried at all.
    TooBig,
    /// A file or folder in ~/Cloud that cannot be read (permissions, a
    /// broken disk). Left alone on both sides until it can.
    Unreadable,
    /// Failed, and waiting to be tried again.
    Failing,
}

/// A file that is not in sync and will not be on the next pass either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StuckFile {
    /// Relative to ~/Cloud.
    pub path: String,
    pub kind: StuckKind,
    /// The last error, shortened; for the details view.
    pub detail: String,
    /// How often it has failed in a row (0 unless `Failing`).
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    /// What the path looked like when it failed (`signature`).
    sig: String,
    count: u32,
    /// This machine's wall clock, unix ms.
    retry_at_ms: u64,
    refused: bool,
    error: String,
}

impl Record {
    fn max_wait(&self) -> u64 {
        if self.refused {
            MAX_WAIT_REFUSED_MS
        } else {
            MAX_WAIT_MS
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Failures {
    #[serde(default)]
    uid: String,
    #[serde(default)]
    entries: HashMap<String, Record>,
    #[serde(skip)]
    dirty: bool,
}

fn wait_ms(count: u32, max: u64) -> u64 {
    FIRST_WAIT_MS
        .saturating_mul(1u64 << count.saturating_sub(1).min(32))
        .min(max)
}

/// An error as one short line: the server's status when there is one (the
/// request URL adds nothing a user can act on), else the message, cut.
pub(super) fn brief(e: &anyhow::Error) -> String {
    let text = match status_of(e) {
        Some(st) if st.message.is_empty() => format!("status code {}", st.code),
        Some(st) => format!("status code {} ({})", st.code, st.message),
        None => e.to_string(),
    };
    let mut out: String = text.chars().take(160).collect();
    if out.len() < text.len() {
        out.push('\u{2026}');
    }
    out
}

/// Everything about a path that, when it changes, makes an earlier failure
/// history: the action, the local file, the remote doc.
pub(super) fn signature(action: Action, local: Option<&LocalFile>, remote: Option<&FileDoc>) -> String {
    let local = match local {
        Some(l) => format!("{}:{}:{}", l.size, l.mtime_ns, l.ino),
        None => "-".to_string(),
    };
    let remote = match remote {
        Some(r) => format!(
            "{}:{}:{}",
            r.sha256,
            r.deleted,
            r.version.as_deref().unwrap_or("")
        ),
        None => "-".to_string(),
    };
    format!("{action:?}|{local}|{remote}")
}

impl Failures {
    /// The record for `uid` at `path`. Missing, unreadable or another
    /// account's: an empty one (there is nothing here worth setting aside;
    /// the worst an empty record does is one retry too many).
    pub fn load_from(path: &Path, uid: &str) -> Self {
        let loaded = std::fs::read(path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Failures>(&raw).ok())
            .filter(|f| f.uid == uid);
        match loaded {
            Some(f) => f,
            None => Failures {
                uid: uid.to_string(),
                entries: HashMap::new(),
                // Whatever is on disk is not this account's record (or not
                // one at all): replaced by the next save.
                dirty: path.exists(),
            },
        }
    }

    pub fn save_to(&mut self, path: &Path) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        super::write_atomic(path, serde_json::to_string_pretty(self)?.as_bytes(), 0o644)?;
        self.dirty = false;
        Ok(())
    }

    /// Is `path` to be left alone this pass? `Some` when it failed looking
    /// exactly like `sig` and its wait is not over.
    pub fn waiting(&self, path: &str, sig: &str, now_ms: u64) -> Option<StuckFile> {
        let r = self.entries.get(path)?;
        if r.sig != sig || now_ms >= r.retry_at_ms {
            return None;
        }
        // Further away than any wait can be: the clock was set back since.
        // The wait is over as far as anyone can tell.
        if r.retry_at_ms - now_ms > r.max_wait() {
            return None;
        }
        Some(StuckFile {
            path: path.to_string(),
            kind: StuckKind::Failing,
            detail: r.error.clone(),
            attempts: r.count,
        })
    }

    /// `path` failed just now. Returns what to report for it.
    pub fn record(&mut self, path: &str, sig: String, error: &anyhow::Error, now_ms: u64) -> StuckFile {
        let refused = status_of(error).is_some_and(|st| (400..500).contains(&st.code));
        let count = match self.entries.get(path) {
            Some(prev) if prev.sig == sig => prev.count.saturating_add(1),
            _ => 1,
        };
        let mut record = Record {
            sig,
            count,
            retry_at_ms: 0,
            refused,
            error: brief(error),
        };
        record.retry_at_ms = now_ms.saturating_add(wait_ms(count, record.max_wait()));
        let stuck = StuckFile {
            path: path.to_string(),
            kind: StuckKind::Failing,
            detail: record.error.clone(),
            attempts: count,
        };
        self.entries.insert(path.to_string(), record);
        self.dirty = true;
        stuck
    }

    /// `path` went through (or is no longer what failed).
    pub fn clear(&mut self, path: &str) {
        if self.entries.remove(path).is_some() {
            self.dirty = true;
        }
    }

    /// Drop every record whose path is not in `keep`: the paths a complete
    /// pass neither skipped nor failed on have nothing wrong with them any
    /// more (synced by other means, deleted, ignored).
    pub fn retain(&mut self, keep: &HashSet<&str>) {
        let before = self.entries.len();
        self.entries.retain(|path, _| keep.contains(path.as_str()));
        self.dirty |= self.entries.len() != before;
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ── "Try again now" ────────────────────────────────────────────────────────
//
// Like the answer to held deletions, the request travels through a file: the
// window with the button is not necessarily the process that owns sync.

/// Ask the sync owner to forget every wait and try the failing files again.
pub(super) fn request_retry(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    super::write_atomic(path, b"retry\n", 0o600)
}

/// Owner side: was a retry asked for? Consumes the request.
pub(super) fn take_retry(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::super::http::HttpStatus;
    use super::super::TestDir;
    use super::*;

    const MIN: u64 = 60 * 1000;

    fn refusal() -> anyhow::Error {
        anyhow::Error::new(HttpStatus {
            method: "POST",
            url: "https://storage/o?name=users%2Fu%2Fblobs%2Fabc".to_string(),
            code: 403,
            status: String::new(),
            message: "Permission denied.".to_string(),
        })
    }

    #[test]
    fn waits_double_up_to_a_cap() {
        assert_eq!(wait_ms(1, MAX_WAIT_MS), MIN);
        assert_eq!(wait_ms(2, MAX_WAIT_MS), 2 * MIN);
        assert_eq!(wait_ms(5, MAX_WAIT_MS), 16 * MIN);
        assert_eq!(wait_ms(7, MAX_WAIT_MS), 60 * MIN);
        assert_eq!(wait_ms(200, MAX_WAIT_MS), 60 * MIN);
        assert_eq!(wait_ms(9, MAX_WAIT_REFUSED_MS), 256 * MIN);
        assert_eq!(wait_ms(u32::MAX, MAX_WAIT_REFUSED_MS), 360 * MIN);
        // A count of zero is not a thing, but must not underflow.
        assert_eq!(wait_ms(0, MAX_WAIT_MS), MIN);
    }

    #[test]
    fn a_failed_path_waits_then_is_tried_again() {
        let mut f = Failures::default();
        let now = 1_000_000_000;
        let err = anyhow::anyhow!("POST https://x: Connection reset by peer");
        let stuck = f.record("a.bin", "sig".into(), &err, now);
        assert_eq!((stuck.kind, stuck.attempts), (StuckKind::Failing, 1));

        assert!(f.waiting("a.bin", "sig", now + MIN - 1).is_some());
        assert!(f.waiting("a.bin", "sig", now + MIN).is_none());
        assert!(f.waiting("other.bin", "sig", now).is_none());

        // Failing again doubles the wait, counted from the new failure.
        let later = now + MIN;
        assert_eq!(f.record("a.bin", "sig".into(), &err, later).attempts, 2);
        assert!(f.waiting("a.bin", "sig", later + 2 * MIN - 1).is_some());
        assert!(f.waiting("a.bin", "sig", later + 2 * MIN).is_none());

        f.clear("a.bin");
        assert!(f.waiting("a.bin", "sig", later).is_none());
        assert_eq!(f.len(), 0);
    }

    #[test]
    fn a_change_to_the_file_ends_the_wait_and_the_count() {
        let mut f = Failures::default();
        let now = 5_000_000;
        for i in 0..6 {
            f.record("a.bin", "old".into(), &refusal(), now + i);
        }
        assert_eq!(f.waiting("a.bin", "old", now + 10).unwrap().attempts, 6);
        // Saved again (other size, other mtime): tried at once...
        assert!(f.waiting("a.bin", "new", now + 10).is_none());
        // ...and if that fails too, it is a first failure.
        assert_eq!(f.record("a.bin", "new".into(), &refusal(), now + 10).attempts, 1);
    }

    #[test]
    fn a_refusal_waits_longer_than_a_hiccup() {
        let mut f = Failures::default();
        let now = 9_000_000_000;
        let hiccup = anyhow::anyhow!("timed out");
        for _ in 0..20 {
            f.record("net.bin", "s".into(), &hiccup, now);
            f.record("no.bin", "s".into(), &refusal(), now);
        }
        assert!(f.waiting("net.bin", "s", now + 60 * MIN - 1).is_some());
        assert!(f.waiting("net.bin", "s", now + 60 * MIN).is_none());
        assert!(f.waiting("no.bin", "s", now + 360 * MIN - 1).is_some());
        assert!(f.waiting("no.bin", "s", now + 360 * MIN).is_none());
    }

    #[test]
    fn a_clock_set_back_does_not_park_a_path_for_years() {
        let mut f = Failures::default();
        let now = 1_790_000_000_000;
        f.record("a.bin", "s".into(), &anyhow::anyhow!("x"), now);
        // The clock now says a year earlier: the retry time is a year away.
        let year = 365 * 24 * 60 * MIN;
        assert!(f.waiting("a.bin", "s", now - year).is_none());
    }

    #[test]
    fn the_record_survives_a_restart_but_not_an_account_switch() {
        let dir = TestDir::new("failures-persist");
        let path = dir.join("failures.json");
        let mut f = Failures::load_from(&path, "alice");
        // Nothing to say: no file is written.
        f.save_to(&path).unwrap();
        assert!(!path.exists());

        let now = 7_000_000;
        f.record("big.iso", "s".into(), &refusal(), now);
        f.record("gone.txt", "s".into(), &refusal(), now);
        f.save_to(&path).unwrap();

        let mut again = Failures::load_from(&path, "alice");
        let stuck = again.waiting("big.iso", "s", now + 1).unwrap();
        assert_eq!(stuck.detail, "status code 403 (Permission denied.)");

        // A complete pass that no longer met "gone.txt" drops its record.
        again.retain(&HashSet::from(["big.iso"]));
        assert_eq!(again.len(), 1);
        again.save_to(&path).unwrap();
        assert_eq!(Failures::load_from(&path, "alice").len(), 1);

        let mut bob = Failures::load_from(&path, "bob");
        assert_eq!(bob.len(), 0);
        bob.save_to(&path).unwrap();
        assert_eq!(Failures::load_from(&path, "alice").len(), 0);
    }

    #[test]
    fn errors_are_reported_short_and_without_the_url() {
        assert_eq!(brief(&refusal()), "status code 403 (Permission denied.)");
        let long = anyhow::anyhow!("{}", "x".repeat(500));
        let short = brief(&long);
        assert_eq!(short.chars().count(), 161);
        assert!(short.ends_with('\u{2026}'));
        assert_eq!(brief(&anyhow::anyhow!("disk full")), "disk full");
    }

    #[test]
    fn a_retry_request_is_taken_once() {
        let dir = TestDir::new("failures-retry");
        let path = dir.join("state/retry-now");
        assert!(!take_retry(&path));
        request_retry(&path).unwrap();
        assert!(take_retry(&path));
        assert!(!take_retry(&path));
    }
}

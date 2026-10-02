// One sync engine per machine.
//
// Every Fox process used to run its own engine against the same ~/Cloud,
// manifest and mirror: each window, each file-picker dialog, the desktop
// daemon. They raced each other (lost manifest updates, both resolving the
// same conflict, each billing its own Firestore reads) and a picker was
// killed mid-pass when its dialog closed.
//
// Now the engine runs only in the process that holds an exclusive flock on
// ~/.lantern/cloud/sync.lock (`Places::lock`). The kernel drops the lock when that process
// exits, however it exits, so there is no stale-lock case to clean up. The
// other processes keep trying (sync.rs) and take over when the owner is gone.
//
// So that every window can still show what sync is doing, the owner writes
// its `SyncReport` to ~/.lantern/cloud/status.json whenever it changes, and
// the others read it.

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use super::sync::SyncReport;

/// Holding this is what makes a process the sync owner. Dropping it (or
/// exiting) releases the lock.
pub(super) struct OwnerLock {
    _file: std::fs::File,
}

impl OwnerLock {
    /// `Ok(None)`: another process owns sync right now.
    pub fn try_acquire(path: &Path) -> std::io::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // std opens with O_CLOEXEC, so helpers Fox launches never inherit
        // the descriptor (and with it the lock).
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EINTR => Ok(None),
                _ => Err(e),
            };
        }
        // For a human looking at the directory; nothing reads it back.
        let _ = file.set_len(0);
        let _ = writeln!(file, "{}", std::process::id());
        Ok(Some(Self { _file: file }))
    }
}

/// Owner side: publish the report for the other processes.
pub(super) fn write_report(path: &Path, report: &SyncReport) {
    let Ok(raw) = serde_json::to_vec(report) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Not fsynced: after a crash the next owner rewrites it anyway.
    if let Err(e) = super::write_replacing(path, &raw, 0o644, false) {
        super::log_line(&format!("cannot write sync status: {e}"));
    }
}

/// Follower side: the owner's report, handed out once per change.
pub(super) struct ReportReader {
    path: PathBuf,
    /// The bytes last returned. Compared by content, not by mtime: two
    /// writes inside one timestamp tick would look like one.
    seen: Vec<u8>,
}

impl ReportReader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            seen: Vec::new(),
        }
    }

    /// `Some` when there is a report this reader has not returned yet.
    pub fn poll(&mut self) -> Option<SyncReport> {
        let raw = std::fs::read(&self.path).ok()?;
        if raw == self.seen {
            return None;
        }
        let report = serde_json::from_slice(&raw).ok()?;
        self.seen = raw;
        Some(report)
    }
}

#[cfg(test)]
mod tests {
    use super::super::sync::{SyncProblem, SyncStatus};
    use super::super::TestDir;
    use super::*;

    fn scratch(name: &str) -> TestDir {
        TestDir::new(&format!("owner-{name}"))
    }

    #[test]
    fn only_one_owner_at_a_time() {
        let dir = scratch("lock");
        let path = dir.join("sync.lock");
        let first = OwnerLock::try_acquire(&path).unwrap();
        assert!(first.is_some());
        // A second open of the same file is a second "process" as far as
        // flock is concerned.
        assert!(OwnerLock::try_acquire(&path).unwrap().is_none());
        assert!(OwnerLock::try_acquire(&path).unwrap().is_none());
        drop(first);
        // The owner is gone: the next one in line takes over.
        assert!(OwnerLock::try_acquire(&path).unwrap().is_some());
    }

    #[test]
    fn followers_see_what_the_owner_reports() {
        let dir = scratch("status");
        let path = dir.join("status.json");
        let mut reader = ReportReader::new(path.clone());
        assert!(reader.poll().is_none());

        let mut report = SyncReport::new("alice");
        report.status = SyncStatus::Error;
        report.problem = Some(SyncProblem::FolderMissing);
        write_report(&path, &report);
        assert_eq!(reader.poll(), Some(report.clone()));
        // Unchanged file: nothing new.
        assert!(reader.poll().is_none());

        report.status = SyncStatus::Idle;
        report.problem = None;
        write_report(&path, &report);
        assert_eq!(reader.poll(), Some(report));
    }
}

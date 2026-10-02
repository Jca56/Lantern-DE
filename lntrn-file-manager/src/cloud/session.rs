// Cached Firebase Auth session: id_token + refresh_token + uid + expiry.
// Stored at ~/.lantern/config/fox-cloud-session.json. The id_token is a short-lived
// JWT (~1h); refresh_token is long-lived. We refresh on demand when within
// REFRESH_SKEW of expiry.
//
// The file is also the sign-out signal for every running Fox: removing it
// (`Session::forget`, `--cloud-logout`) is what "signed out" means. So
//   - a sync thread stops as soon as the file is gone or names another
//     account (`SessionWatch`), and
//   - a token refresh only writes the file back while it still holds the
//     session being refreshed (`save_refreshed`). A refresh that finishes
//     after a sign-out must not sign the user in again.
// Writers and the remover take a short flock, so "check, then write" cannot
// interleave with the removal.

use serde::{Deserialize, Serialize};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::CloudConfig;

/// Refresh if the id_token would expire within this many seconds.
const REFRESH_SKEW: u64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub uid: String,
    pub email: String,
    pub id_token: String,
    pub refresh_token: String,
    /// Unix seconds at which id_token expires.
    pub expires_at: u64,
}

/// Held while the session file is checked and then written or removed.
/// Dropping it closes the file, which releases the lock. `None` inside means
/// the lock file could not be opened: the operation still runs, just
/// unserialized.
struct FileLock {
    _file: Option<std::fs::File>,
}

impl FileLock {
    fn take() -> Self {
        let mut name = Session::path().into_os_string();
        name.push(".lock");
        let path = PathBuf::from(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .ok();
        if let Some(f) = &file {
            // Blocking: the holder only reads or writes one small file.
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
        }
        FileLock { _file: file }
    }
}

impl Session {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/config/fox-cloud-session.json")
    }

    pub fn load_cached(_cfg: &CloudConfig) -> anyhow::Result<Self> {
        let path = Self::path();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let s: Session = serde_json::from_str(&raw)?;
        Ok(s)
    }

    fn write(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        // This file holds a refresh token: private from the moment it exists.
        super::write_atomic(&path, raw.as_bytes(), 0o600)?;
        Ok(())
    }

    /// Store a session the user just signed in with.
    pub fn save(&self) -> anyhow::Result<()> {
        let _lock = FileLock::take();
        self.write()
    }

    /// Store refreshed tokens, but only over the session they were refreshed
    /// from: `Ok(false)` (and nothing written) when the file is gone or holds
    /// another account, i.e. the user signed out or switched meanwhile.
    pub fn save_refreshed(&self) -> anyhow::Result<bool> {
        let _lock = FileLock::take();
        if !self.is_current() {
            return Ok(false);
        }
        self.write()?;
        Ok(true)
    }

    /// Sign out: every running Fox stops syncing when this file disappears.
    pub fn forget() {
        let _lock = FileLock::take();
        let _ = std::fs::remove_file(Self::path());
    }

    /// Remove the cached session because `refresh_token` was rejected for
    /// good. Leaves the file alone when it holds a different token: the user
    /// has signed in again since, and that session is not the dead one.
    pub(super) fn forget_revoked(refresh_token: &str) {
        let _lock = FileLock::take();
        let on_disk = std::fs::read(Self::path())
            .ok()
            .and_then(|raw| serde_json::from_slice::<Session>(&raw).ok());
        if on_disk.is_some_and(|s| s.refresh_token == refresh_token) {
            let _ = std::fs::remove_file(Self::path());
        }
    }

    /// Is this account still the one signed in on this machine? False when
    /// the session file is gone or holds something else; a file that merely
    /// cannot be read right now signs nobody out.
    pub(super) fn is_current(&self) -> bool {
        holds_account(&Self::path(), &self.uid)
    }

    pub fn is_fresh(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.expires_at > now + REFRESH_SKEW
    }
}

/// Does the session file at `path` (still) belong to `uid`?
fn holds_account(path: &std::path::Path, uid: &str) -> bool {
    match std::fs::read(path) {
        Ok(raw) => serde_json::from_slice::<Session>(&raw).is_ok_and(|s| s.uid == uid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        // Cannot tell (I/O hiccup): do not sign anyone out on a guess.
        Err(_) => true,
    }
}

/// "Is this account still signed in on this machine?" for the sync threads,
/// asked several times a second: one small read per call, a parse only when
/// the bytes changed.
pub(super) struct SessionWatch {
    uid: String,
    path: PathBuf,
    /// The file's bytes when it was last parsed and found to hold `uid`.
    /// Compared by content, not by mtime: a sign-out and sign-in inside one
    /// timestamp tick would look like nothing happened.
    seen: Vec<u8>,
}

impl SessionWatch {
    pub fn new(path: PathBuf, uid: &str) -> Self {
        Self {
            uid: uid.to_string(),
            path,
            seen: Vec::new(),
        }
    }

    /// False once the session file is gone, is not a session, or holds
    /// another account.
    pub fn still_signed_in(&mut self) -> bool {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            // Cannot tell (I/O hiccup): do not sign anyone out on a guess.
            Err(_) => return true,
        };
        if !self.seen.is_empty() && raw == self.seen {
            return true;
        }
        let same_account =
            serde_json::from_slice::<Session>(&raw).is_ok_and(|s| s.uid == self.uid);
        if same_account {
            self.seen = raw;
        }
        same_account
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_json(uid: &str) -> String {
        serde_json::to_string(&Session {
            uid: uid.to_string(),
            email: "a@b.c".to_string(),
            id_token: "id".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: 0,
        })
        .unwrap()
    }

    #[test]
    fn a_removed_or_replaced_session_file_ends_the_session() {
        let dir = super::super::TestDir::new("session");
        let path = dir.join("session.json");

        let mut watch = SessionWatch::new(path.clone(), "alice");
        assert!(!watch.still_signed_in());

        std::fs::write(&path, session_json("alice")).unwrap();
        assert!(watch.still_signed_in());
        assert!(watch.still_signed_in());

        // Another account signed in on this machine.
        std::fs::write(dir.join("next"), session_json("bob")).unwrap();
        std::fs::rename(dir.join("next"), &path).unwrap();
        assert!(!watch.still_signed_in());

        // A token refresh of the same account rewrites the file: still in.
        std::fs::write(dir.join("next"), session_json("alice")).unwrap();
        std::fs::rename(dir.join("next"), &path).unwrap();
        assert!(watch.still_signed_in());

        std::fs::remove_file(&path).unwrap();
        assert!(!watch.still_signed_in());
    }
}

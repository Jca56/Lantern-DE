// Local sync ledger at ~/.lantern/cloud/manifest.json.
//
// For each synced file we remember the last-known-synced sha256 — that's the
// pivot for three-way merge:
//   local_sha == manifest && remote_sha == manifest  → in sync, no-op
//   local_sha == manifest && remote_sha != manifest  → pull (remote changed alone)
//   local_sha != manifest && remote_sha == manifest  → push (local changed alone)
//   local_sha != manifest && remote_sha != manifest  → conflict → keep both

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Stat snapshot paired with a manifest sha: "a file at this path with exactly
/// this (size, mtime) hashed to `entries[path]`". Lets the local scan skip
/// re-hashing unchanged files — with a large ~/Cloud, hashing every file on
/// every reconcile was constant CPU+disk churn.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct FileMeta {
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// The account these pivots were agreed with. Another account's pivots
    /// applied to this account's remote would overwrite and delete local
    /// files without a conflict copy, so a manifest is only ever used for
    /// the uid it names.
    #[serde(default)]
    pub uid: String,
    /// Relative path (the file names joined with '/') → last-synced sha256.
    pub entries: HashMap<String, String>,
    /// Stat cache keyed like `entries`.
    #[serde(default)]
    pub meta: HashMap<String, FileMeta>,
    /// Changed since load or the last save. A quiet poll changes nothing, and
    /// rewriting the file every 30 seconds is pointless disk churn.
    #[serde(skip)]
    dirty: bool,
}

/// A file whose mtime is this close to the moment it was stat'ed can be
/// rewritten again inside the same mtime second with the same size, and the
/// stat cache could not tell. Such a file is simply hashed again on the next
/// scan.
const RACY_WINDOW_SECS: u64 = 2;

/// Where a loaded manifest came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Read from disk, for this account.
    Loaded,
    /// No manifest yet: first sync on this machine.
    Fresh,
    /// The file was damaged; kept as `manifest.json.corrupt`.
    Corrupt,
    /// The file was another account's; kept as `manifest.json.other-account`.
    OtherAccount,
}

/// Move a state file out of the way as `<name>.<tag>`, replacing an older
/// one with that tag. If even that fails the file is left alone: the next
/// save replaces it atomically.
pub(super) fn set_aside(path: &std::path::Path, tag: &str) {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{tag}"));
    let aside = path.with_file_name(name);
    if let Err(e) = std::fs::rename(path, &aside) {
        super::log_line(&format!("could not set {} aside: {e}", path.display()));
    }
}

impl Manifest {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/cloud/manifest.json")
    }

    /// Changed since load or the last save.
    #[cfg(test)]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The manifest for `uid`, from `path` (`Manifest::path()` outside
    /// tests). One that cannot be parsed, or that belongs to another account,
    /// is moved aside (never deleted, never reused) and an empty one takes
    /// its place; `Origin` tells the caller, because a pass that starts
    /// without its pivots must not delete anything.
    pub(super) fn load_from(path: &std::path::Path, uid: &str) -> (Self, Origin) {
        let fresh = |origin| {
            let m = Manifest {
                uid: uid.to_string(),
                // Written out at the end of the pass even if it stays empty,
                // so the reset is reported once, not on every pass.
                dirty: origin != Origin::Fresh,
                ..Manifest::default()
            };
            (m, origin)
        };
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fresh(Origin::Fresh),
            Err(e) => {
                // Unreadable (I/O error, or not UTF-8): same as unparsable.
                super::log_line(&format!("manifest unreadable ({e}), set aside"));
                set_aside(path, "corrupt");
                return fresh(Origin::Corrupt);
            }
        };
        match serde_json::from_str::<Manifest>(&raw) {
            Ok(m) if m.uid == uid => (m, Origin::Loaded),
            Ok(m) => {
                super::log_line(&format!(
                    "manifest belongs to another account ({} entries), set aside",
                    m.entries.len()
                ));
                set_aside(path, "other-account");
                fresh(Origin::OtherAccount)
            }
            Err(e) => {
                super::log_line(&format!("manifest does not parse ({e}), set aside"));
                set_aside(path, "corrupt");
                fresh(Origin::Corrupt)
            }
        }
    }

    /// Write the manifest if anything changed since the last save.
    pub(super) fn save_to(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
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

    pub fn get(&self, rel: &str) -> Option<&str> {
        self.entries.get(rel).map(|s| s.as_str())
    }

    pub fn set(&mut self, rel: String, sha: String) {
        if self.entries.get(&rel) != Some(&sha) {
            self.entries.insert(rel, sha);
            self.dirty = true;
        }
    }

    /// Record the stat snapshot that goes with `entries[rel]`. `as_of` is
    /// when that snapshot was taken (unix seconds) — the scan time, not the
    /// time of this call, which can be a whole upload later. Two-sided on
    /// purpose: a file dated in the future is not "fresh", and treating it
    /// as such would re-hash it on every pass.
    pub fn set_meta(&mut self, rel: String, size: u64, mtime: u64, as_of: u64) {
        if as_of.abs_diff(mtime) <= RACY_WINDOW_SECS {
            // Too fresh to trust (see RACY_WINDOW_SECS): no cache entry.
            if self.meta.remove(&rel).is_some() {
                self.dirty = true;
            }
            return;
        }
        let unchanged = self
            .meta
            .get(&rel)
            .is_some_and(|m| m.size == size && m.mtime == mtime);
        if !unchanged {
            self.meta.insert(rel, FileMeta { size, mtime });
            self.dirty = true;
        }
    }

    /// Stat-cache lookup: the last-synced sha for `rel`, valid only if the
    /// file's current (size, mtime) still matches the recorded snapshot.
    pub fn cached_sha(&self, rel: &str, size: u64, mtime: u64) -> Option<&str> {
        let m = self.meta.get(rel)?;
        if m.size == size && m.mtime == mtime {
            self.entries.get(rel).map(|s| s.as_str())
        } else {
            None
        }
    }

    /// Stop trusting the stat cache for `rel`: the file turned out not to be
    /// what its (size, mtime) promised, so the next scan must hash it.
    pub fn distrust(&mut self, rel: &str) {
        if self.meta.remove(rel).is_some() {
            self.dirty = true;
        }
    }

    pub fn remove(&mut self, rel: &str) {
        let had_entry = self.entries.remove(rel).is_some();
        let had_meta = self.meta.remove(rel).is_some();
        self.dirty |= had_entry || had_meta;
    }
}

#[cfg(test)]
mod tests {
    use super::super::TestDir;
    use super::*;

    fn scratch(name: &str) -> TestDir {
        TestDir::new(&format!("manifest-{name}"))
    }

    #[test]
    fn a_missing_manifest_is_a_first_sync() {
        let dir = scratch("fresh");
        let path = dir.join("manifest.json");
        let (m, origin) = Manifest::load_from(&path, "alice");
        assert_eq!(origin, Origin::Fresh);
        assert_eq!(m.uid, "alice");
        assert!(m.entries.is_empty() && !m.is_dirty());
    }

    #[test]
    fn a_corrupt_manifest_is_kept_aside_not_replaced() {
        let dir = scratch("corrupt");
        let path = dir.join("manifest.json");
        std::fs::write(&path, b"{\"uid\":\"alice\",\"entries\":{\"a.txt\":\"ab").unwrap();
        let (m, origin) = Manifest::load_from(&path, "alice");
        assert_eq!(origin, Origin::Corrupt);
        assert!(m.entries.is_empty());
        assert!(m.is_dirty());
        assert!(!path.exists());
        let kept = std::fs::read(dir.join("manifest.json.corrupt")).unwrap();
        assert!(kept.starts_with(b"{\"uid\""));
    }

    #[test]
    fn another_accounts_manifest_is_not_reused() {
        let dir = scratch("account");
        let path = dir.join("manifest.json");
        std::fs::write(
            &path,
            br#"{"uid":"alice","entries":{"a.txt":"aa"},"meta":{}}"#,
        )
        .unwrap();
        let (m, origin) = Manifest::load_from(&path, "alice");
        assert_eq!(origin, Origin::Loaded);
        assert_eq!(m.get("a.txt"), Some("aa"));

        let (m, origin) = Manifest::load_from(&path, "bob");
        assert_eq!(origin, Origin::OtherAccount);
        assert_eq!(m.uid, "bob");
        assert!(m.entries.is_empty());
        assert!(dir.join("manifest.json.other-account").exists());
        // Bob's next load starts clean; alice's pivots do not come back.
        let (_, origin) = Manifest::load_from(&path, "bob");
        assert_eq!(origin, Origin::Fresh);
    }

    #[test]
    fn a_manifest_without_an_account_is_not_trusted() {
        let dir = scratch("nouid");
        let path = dir.join("manifest.json");
        std::fs::write(&path, br#"{"entries":{"a.txt":"aa"}}"#).unwrap();
        let (m, origin) = Manifest::load_from(&path, "alice");
        assert_eq!(origin, Origin::OtherAccount);
        assert!(m.entries.is_empty());
    }
}

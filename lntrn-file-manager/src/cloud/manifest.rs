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
    /// Relative path (forward-slashed) → last-synced sha256.
    pub entries: HashMap<String, String>,
    /// Stat cache keyed like `entries`. `#[serde(default)]` keeps old
    /// manifest.json files (and older builds re-saving) compatible.
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

impl Manifest {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/cloud/manifest.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Manifest::default(),
        }
    }

    /// Write the manifest if anything changed since the last save.
    pub fn save(&mut self) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        super::write_atomic(&path, serde_json::to_string_pretty(self)?.as_bytes(), 0o644)?;
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

    pub fn remove(&mut self, rel: &str) {
        let had_entry = self.entries.remove(rel).is_some();
        let had_meta = self.meta.remove(rel).is_some();
        self.dirty |= had_entry || had_meta;
    }
}

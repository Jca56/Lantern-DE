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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// Relative path (forward-slashed) → last-synced sha256.
    pub entries: HashMap<String, String>,
}

impl Manifest {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/cloud/manifest.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Manifest::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, rel: &str) -> Option<&str> {
        self.entries.get(rel).map(|s| s.as_str())
    }

    pub fn set(&mut self, rel: String, sha: String) {
        self.entries.insert(rel, sha);
    }

    pub fn remove(&mut self, rel: &str) {
        self.entries.remove(rel);
    }
}

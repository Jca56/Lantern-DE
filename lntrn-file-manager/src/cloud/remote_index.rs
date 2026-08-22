// Local mirror of the Firestore file collection, persisted at
// ~/.lantern/cloud/remote-index.json.
//
// This is the quota fix's second half: reconcile passes 3-way against THIS
// map instead of re-listing Firestore. The index is seeded by one full list
// (startup + a periodic drift-heal) and kept fresh by `query_changed_since`
// delta pulls keyed on `cursor_ms` — so a steady-state sync pass costs ~1
// Firestore read, regardless of how many files ~/Cloud holds.
//
// `cursor_ms` is the largest `updated_at` ever seen. Delta pulls query from
// `cursor_ms - OVERLAP_MS` so modest clock skew between the two machines
// (both write client-side stamps) can't hide a doc; the reconciler is
// idempotent, so re-seeing a doc is free.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::firestore::FileDoc;

/// Clock-skew allowance subtracted from the cursor on every delta pull.
pub const OVERLAP_MS: u64 = 300_000; // 5 minutes

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteIndex {
    /// Highest `updated_at` observed across all docs (full or delta).
    pub cursor_ms: u64,
    /// Relative path → last-known remote doc (tombstones included, exactly
    /// like a live listing).
    pub docs: HashMap<String, FileDoc>,
}

impl RemoteIndex {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/cloud/remote-index.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string(self)?)?;
        Ok(())
    }

    fn bump_cursor(&mut self, doc: &FileDoc) {
        if let Some(ts) = doc.updated_at {
            self.cursor_ms = self.cursor_ms.max(ts);
        }
    }

    /// Replace the whole mirror with a fresh full listing.
    pub fn seed_full(&mut self, docs: Vec<FileDoc>) {
        self.docs.clear();
        for d in docs {
            self.bump_cursor(&d);
            self.docs.insert(d.path.clone(), d);
        }
    }

    /// Upsert delta-pull results. Returns how many docs changed the mirror
    /// (0 = quiet poll, nothing to reconcile against).
    pub fn apply_delta(&mut self, docs: Vec<FileDoc>) -> usize {
        let mut changed = 0;
        for d in docs {
            self.bump_cursor(&d);
            let differs = self.docs.get(&d.path).map_or(true, |old| {
                old.sha256 != d.sha256 || old.deleted != d.deleted
            });
            if differs {
                changed += 1;
            }
            self.docs.insert(d.path.clone(), d);
        }
        changed
    }
}

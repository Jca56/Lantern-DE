// Local mirror of the Firestore file collection, persisted at
// ~/.lantern/cloud/remote-index.json.
//
// Reconcile passes merge 3-way against THIS map instead of re-listing
// Firestore. The mirror is seeded by one full list and kept fresh by delta
// pulls, so a steady-state sync pass costs one Firestore read, regardless of
// how many files ~/Cloud holds.
//
// A full list bills one read per document, so it is NOT done on every start:
// the mirror records when it was last seeded (`last_full_ms`, this machine's
// wall clock) and a start only lists again when that is older than the
// drift-heal period, or when there is no usable mirror (missing, unparsable,
// another account's, another format).
//
// This machine's own writes are put into the mirror the moment Firestore
// accepts them (`record_own_write`), with the stamp and version the server
// gave them. Decisions never depend on reading an own write back.
//
// THE CURSOR is server time: the read time Firestore reported for the last
// pull. Every write is stamped by the server too (firestore.rs), so "what
// changed since my last pull" is asked and answered on one clock, and the
// machines' own clocks cannot make a write invisible. A delta pull asks for
// every doc stamped after `cursor - OVERLAP`:
//   - the cursor moves with every pull, changed docs or not, so a quiet poll
//     matches nothing (one billed read) and a doc is re-read only by the
//     polls of the OVERLAP after it was written, not forever;
//   - the overlap is for a write that was stamped just before a pull but
//     committed just after it: the pull does not see it, and without the
//     overlap the next one would ask for later stamps only.
// Re-seeing a doc costs its read and nothing else: the reconciler is
// idempotent.
//
// The cursor alone is not worth a rewrite of the file (that would be one
// every 30 seconds): it is saved when a doc changes. After a restart the
// first pull then starts from the cursor of the last change, and whatever it
// returns in addition has not been seen yet anyway.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::firestore::FileDoc;
use super::stamp::{Micros, MINUTE};

/// How far behind the cursor a delta pull reaches (see above). Firestore
/// stamps a write when it processes the request and commits it moments
/// later; two minutes is far more than that gap ever is.
pub const OVERLAP: Micros = 2 * MINUTE;

/// Bumped whenever a field changes meaning. A mirror written in another
/// format is not read: the next pass lists the collection afresh.
const FORMAT: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteIndex {
    #[serde(default)]
    format: u32,
    /// The account this mirror was listed from. Never used for another one.
    #[serde(default)]
    pub uid: String,
    /// Server time of the last pull (full or delta) this mirror reflects.
    #[serde(default)]
    pub cursor: Micros,
    /// This machine's wall clock (unix ms) at the last full list that went
    /// through. 0 = never seeded: nothing may be reconciled against this
    /// mirror yet. Only used to schedule the next full list.
    #[serde(default)]
    pub last_full_ms: u64,
    /// Relative path → last-known remote doc (tombstones included, exactly
    /// like a live listing).
    pub docs: HashMap<String, FileDoc>,
    /// Changed since load or the last save (a quiet delta poll changes nothing).
    #[serde(skip)]
    dirty: bool,
}

impl RemoteIndex {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/cloud/remote-index.json")
    }

    /// The mirror for `uid` from `path` (`RemoteIndex::path()` outside
    /// tests), or an unseeded one when the file is missing, does not parse,
    /// is in another format, or was listed from another account (that one
    /// is set aside). An unseeded mirror forces a full list.
    pub(super) fn load_from(path: &std::path::Path, uid: &str) -> Self {
        let empty = || RemoteIndex {
            format: FORMAT,
            uid: uid.to_string(),
            ..RemoteIndex::default()
        };
        let Ok(raw) = std::fs::read_to_string(path) else {
            return empty();
        };
        match serde_json::from_str::<RemoteIndex>(&raw) {
            Ok(idx) if idx.uid == uid && idx.format == FORMAT => idx,
            Ok(idx) if idx.uid == uid => {
                super::log_line("remote index is in an older format, re-listing");
                empty()
            }
            Ok(_) => {
                super::log_line("remote index belongs to another account, set aside");
                super::manifest::set_aside(path, "other-account");
                empty()
            }
            Err(e) => {
                super::log_line(&format!("remote index does not parse ({e}), re-listing"));
                empty()
            }
        }
    }

    /// Has a full list ever filled this mirror?
    pub fn is_seeded(&self) -> bool {
        self.last_full_ms != 0
    }

    /// Is a full list due? Never seeded, older than `period_ms`, or stamped
    /// in the future (the clock was set back: the age is unknowable).
    pub fn needs_full(&self, now_ms: u64, period_ms: u64) -> bool {
        !self.is_seeded()
            || now_ms < self.last_full_ms
            || now_ms - self.last_full_ms >= period_ms
    }

    /// The lower bound (exclusive) for the next delta pull. See the top of
    /// this file.
    pub fn delta_since(&self) -> Micros {
        self.cursor.saturating_sub(OVERLAP)
    }

    /// Write the mirror if anything changed since the last save.
    pub(super) fn save_to(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.format = FORMAT;
        super::write_atomic(path, serde_json::to_string(self)?.as_bytes(), 0o644)?;
        self.dirty = false;
        Ok(())
    }

    /// Replace the whole mirror with a fresh full listing. `server_time` is
    /// the server's clock from BEFORE the listing started: a doc written
    /// while the pages were being read is stamped after it and comes again
    /// with the next delta. `now_ms` is this machine's clock, for scheduling
    /// the next full list.
    pub fn seed_full(&mut self, docs: Vec<FileDoc>, server_time: Option<Micros>, now_ms: u64) {
        self.dirty = true;
        self.format = FORMAT;
        // max(1): 0 means "never seeded", whatever the clock says.
        self.last_full_ms = now_ms.max(1);
        // Without the server's time the newest stamp in the listing is the
        // latest moment the listing is known to cover.
        self.cursor = server_time
            .or_else(|| docs.iter().filter_map(FileDoc::stamp).max())
            .unwrap_or(0);
        self.docs = docs.into_iter().map(|d| (d.path.clone(), d)).collect();
    }

    /// Upsert the result of a delta pull read at `read_at` (server clock).
    /// Returns how many docs changed what the mirror says about a path's
    /// content (0 = nothing new to reconcile).
    pub fn apply_delta(&mut self, docs: Vec<FileDoc>, read_at: Option<Micros>) -> usize {
        let newest = docs.iter().filter_map(FileDoc::stamp).max();
        let mut changed = 0;
        for d in docs {
            let differs = self.docs.get(&d.path).map_or(true, |old| {
                old.sha256 != d.sha256 || old.deleted != d.deleted
            });
            if differs {
                changed += 1;
            }
            // The overlap re-delivers the same docs for a while; only a real
            // difference is worth a rewrite of the mirror.
            if self.docs.get(&d.path) != Some(&d) {
                self.dirty = true;
                self.docs.insert(d.path.clone(), d);
            }
        }
        // Not `dirty` by itself: see the top of this file. Never backwards:
        // server time does not run backwards, and a reply without a read
        // time must not undo one that had it.
        if let Some(seen) = read_at.or(newest) {
            self.cursor = self.cursor.max(seen);
        }
        changed
    }

    /// Record a doc this machine just wrote, as the server reported it.
    pub fn record_own_write(&mut self, doc: FileDoc) {
        self.dirty = true;
        self.docs.insert(doc.path.clone(), doc);
    }

    /// The doc for `path` was fetched on its own (after a refused write):
    /// make the mirror say what the cloud says now.
    pub fn refresh(&mut self, path: &str, doc: Option<FileDoc>) {
        match doc {
            Some(doc) => {
                if self.docs.get(path) != Some(&doc) {
                    self.dirty = true;
                    self.docs.insert(path.to_string(), doc);
                }
            }
            None => self.forget(path),
        }
    }

    /// The doc for `path` no longer exists in the cloud.
    pub fn forget(&mut self, path: &str) {
        if self.docs.remove(path).is_some() {
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::stamp::{HOUR, SECOND};
    use super::*;

    fn doc(path: &str, sha: &str, at: Micros) -> FileDoc {
        FileDoc {
            path: path.to_string(),
            sha256: sha.to_string(),
            size: 1,
            mtime: 1,
            mime: String::new(),
            device: "test".to_string(),
            deleted: false,
            updated_at: Some(at),
            version: Some(super::super::stamp::format(at)),
        }
    }

    const SIX_HOURS: u64 = 6 * 3600 * 1000;
    /// Some server time.
    const T0: Micros = 1_790_000_000 * SECOND;

    #[test]
    fn a_fresh_mirror_starts_with_a_delta_not_a_full_list() {
        let mut idx = RemoteIndex::default();
        assert!(!idx.is_seeded());
        assert!(idx.needs_full(1_000_000, SIX_HOURS));

        idx.seed_full(vec![doc("a.txt", "aa", T0 - HOUR)], Some(T0), 1_000_000);
        assert!(idx.is_seeded());
        assert_eq!(idx.cursor, T0);
        // A restart a minute later, or five hours later: delta.
        assert!(!idx.needs_full(1_060_000, SIX_HOURS));
        assert!(!idx.needs_full(1_000_000 + SIX_HOURS - 1, SIX_HOURS));
        // Past the period, or with the clock set back: full list.
        assert!(idx.needs_full(1_000_000 + SIX_HOURS, SIX_HOURS));
        assert!(idx.needs_full(999_999, SIX_HOURS));
    }

    #[test]
    fn the_cursor_follows_the_servers_clock_not_the_newest_doc() {
        let mut idx = RemoteIndex::default();
        // A burst of uploads, listed right after.
        let burst: Vec<FileDoc> = (0..50)
            .map(|i| doc(&format!("f{i}"), "x", T0 - i * SECOND))
            .collect();
        idx.seed_full(burst.clone(), Some(T0 + SECOND), 1);
        assert_eq!(idx.delta_since(), T0 + SECOND - OVERLAP);

        // A poll inside the overlap sees the burst again and changes nothing.
        assert_eq!(idx.apply_delta(burst, Some(T0 + 30 * SECOND)), 0);
        // Quiet polls move the cursor on all the same...
        assert_eq!(idx.apply_delta(Vec::new(), Some(T0 + 60 * SECOND)), 0);
        assert_eq!(idx.apply_delta(Vec::new(), Some(T0 + 5 * MINUTE)), 0);
        // ...so the bound is now past every doc of the burst: the next poll
        // matches nothing, and stays that way.
        assert!(idx.delta_since() > T0);
        assert_eq!(idx.delta_since(), T0 + 5 * MINUTE - OVERLAP);
    }

    #[test]
    fn a_write_committed_just_after_a_pull_is_inside_the_next_one() {
        let mut idx = RemoteIndex::default();
        idx.seed_full(Vec::new(), Some(T0), 1);
        // Stamped a second before the pull at T0 + 30 s, committed after it:
        // that pull does not return it.
        let late = doc("late.txt", "l", T0 + 29 * SECOND);
        idx.apply_delta(Vec::new(), Some(T0 + 30 * SECOND));
        assert!(late.updated_at.unwrap() > idx.delta_since());
        assert_eq!(idx.apply_delta(vec![late], Some(T0 + 60 * SECOND)), 1);
        assert!(idx.docs.contains_key("late.txt"));
    }

    #[test]
    fn the_cursor_never_runs_backwards_and_survives_a_reply_without_a_time() {
        let mut idx = RemoteIndex::default();
        idx.seed_full(vec![doc("a", "1", T0)], None, 1);
        // No server time with the listing: the newest stamp stands in.
        assert_eq!(idx.cursor, T0);
        idx.apply_delta(vec![doc("b", "2", T0 + SECOND)], None);
        assert_eq!(idx.cursor, T0 + SECOND);
        idx.apply_delta(Vec::new(), Some(T0 + MINUTE));
        idx.apply_delta(Vec::new(), None);
        idx.apply_delta(vec![doc("c", "3", T0)], Some(T0 + 2 * SECOND));
        assert_eq!(idx.cursor, T0 + MINUTE);
        // An empty collection listed without a time: everything is "after".
        let mut empty = RemoteIndex::default();
        empty.seed_full(Vec::new(), None, 1);
        assert_eq!(empty.delta_since(), 0);
    }

    #[test]
    fn own_writes_are_visible_without_reading_them_back() {
        let mut idx = RemoteIndex::default();
        idx.seed_full(vec![doc("a.txt", "old", T0)], Some(T0 + SECOND), 1_000);
        let written = doc("a.txt", "new", T0 + MINUTE);
        idx.record_own_write(written.clone());
        assert_eq!(idx.docs["a.txt"].sha256, "new");
        // An own write is not a pull: the cursor stays.
        assert_eq!(idx.cursor, T0 + SECOND);
        // A delta that re-delivers the same doc changes nothing.
        assert_eq!(idx.apply_delta(vec![written], Some(T0 + 2 * MINUTE)), 0);
    }

    #[test]
    fn a_refreshed_or_forgotten_doc_replaces_the_mirrors_view() {
        let mut idx = RemoteIndex::default();
        idx.seed_full(vec![doc("a.txt", "old", T0)], Some(T0), 1);
        idx.refresh("a.txt", Some(doc("a.txt", "theirs", T0 + SECOND)));
        assert_eq!(idx.docs["a.txt"].sha256, "theirs");
        idx.refresh("a.txt", None);
        assert!(idx.docs.is_empty());
        idx.forget("never-there");
    }

    #[test]
    fn another_accounts_or_another_formats_mirror_is_not_reused() {
        let dir = super::super::TestDir::new("index");
        let path = dir.join("remote-index.json");

        let mut idx = RemoteIndex::load_from(&path, "alice");
        assert!(!idx.is_seeded());
        idx.seed_full(vec![doc("a.txt", "aa", T0)], Some(T0), 77);
        idx.save_to(&path).unwrap();

        let same = RemoteIndex::load_from(&path, "alice");
        assert!(same.is_seeded());
        assert_eq!(same.docs.len(), 1);
        assert_eq!(same.cursor, T0);

        // What the build before this one wrote: millisecond client stamps
        // under other field names.
        let old = dir.join("old-index.json");
        std::fs::write(
            &old,
            br#"{"uid":"alice","cursor_ms":1790000000000,"last_full_ms":5,"docs":{}}"#,
        )
        .unwrap();
        let reset = RemoteIndex::load_from(&old, "alice");
        assert!(!reset.is_seeded());
        assert_eq!(reset.cursor, 0);

        let other = RemoteIndex::load_from(&path, "bob");
        assert_eq!(other.uid, "bob");
        assert!(!other.is_seeded());
        assert!(other.docs.is_empty());
        assert_eq!(other.cursor, 0);
        assert!(dir.join("remote-index.json.other-account").exists());
    }
}

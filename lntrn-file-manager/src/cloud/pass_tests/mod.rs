// Whole reconcile passes against an in-memory cloud and scratch directories:
// two "machines" share one cloud, each with its own ~/Cloud, state and Trash.
// These are the scenarios where sync used to lose files.
//
//   deletions.rs — what must never be read as "the user deleted this"
//   changes.rs   — merging, files that change under a pass, odd names

//   conflicts.rs — writes the other machine made since the mirror was read
//   transfers.rs — blobs that are not what their hash says, files that
//                  cannot be synced
//   cleanup.rs   — old tombstones, unused blobs, empty folders
//   polling.rs   — what a poll costs
//   engine.rs, engine_more.rs — the sync threads end to end
//   answers.rs   — a dialog's answer to held deletions names its set
//   fake.rs      — the in-memory cloud all of them run against

mod answers;
mod changes;
mod cleanup;
mod conflicts;
mod deletions;
mod engine;
mod engine_more;
mod fake;
mod polling;
mod transfers;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::firestore::now_ms;
use super::manifest::Manifest;
use super::reconcile::{run_pass, PassError, PassInput, PassReport};
use super::remote_index::RemoteIndex;
use super::stamp::Micros;
use super::store::{Places, Store};
use super::TestDir;

use fake::FakeCloud;

/// The places of one machine under `dir`.
fn places_in(dir: &Path) -> Places {
    Places {
        root: dir.join("Cloud"),
        manifest: dir.join("state/manifest.json"),
        index: dir.join("state/remote-index.json"),
        trash: dir.join("Trash"),
        session: dir.join("config/session.json"),
        lock: dir.join("state/sync.lock"),
        status: dir.join("state/status.json"),
        held_list: dir.join("state/held-deletions.json"),
        answer: dir.join("state/deletions-answer.json"),
        failures: dir.join("state/failures.json"),
        retry: dir.join("state/retry-now"),
    }
}

/// One machine: its own folder, state files and Trash, on a shared cloud.
struct Machine {
    cloud: Arc<FakeCloud>,
    _dir: TestDir,
    places: Places,
    index: RemoteIndex,
    approved: HashSet<String>,
}

impl Machine {
    fn new(test: &str, name: &str, cloud: &Arc<FakeCloud>) -> Self {
        let dir = TestDir::new(&format!("pass-{test}-{name}"));
        let places = places_in(&dir);
        std::fs::create_dir_all(&places.root).unwrap();
        Self {
            cloud: cloud.clone(),
            index: RemoteIndex::load_from(&places.index, &cloud.uid()),
            _dir: dir,
            places,
            approved: HashSet::new(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.places.root.join(rel)
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.path(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(rel)).ok()
    }

    fn files(&self) -> Vec<String> {
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), root, out);
                } else {
                    let rel = e.path().strip_prefix(root).unwrap().to_path_buf();
                    out.push(rel.to_string_lossy().into_owned());
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.places.root, &self.places.root, &mut out);
        out.sort();
        out
    }

    fn trashed(&self) -> usize {
        std::fs::read_dir(self.places.trash.join("files"))
            .map(|rd| rd.count())
            .unwrap_or(0)
    }

    /// What the engine does before a pass when a full list is due: the
    /// server's time, then the whole collection. Returns that time, which is
    /// what lets the pass clean up afterwards.
    fn fetch(&mut self) -> Micros {
        let server_time = self.cloud.server_time().unwrap();
        let docs = self.cloud.list_all().unwrap();
        self.index.seed_full(docs, server_time, now_ms());
        server_time.unwrap()
    }

    /// What the engine does before every other pass: a delta pull.
    fn pull(&mut self) {
        let pull = self.cloud.changed_since(self.index.delta_since()).unwrap();
        self.index.apply_delta(pull.docs, pull.read_at);
    }

    /// One pass against the mirror as it is. `maintain`: as after a full
    /// list (see `PassInput::maintain`).
    fn try_pass(
        &mut self,
        maintain: Option<Micros>,
        cancel: &mut dyn FnMut() -> bool,
    ) -> Result<PassReport, PassError> {
        let approved = std::mem::take(&mut self.approved);
        run_pass(PassInput {
            store: &*self.cloud,
            places: &self.places,
            index: &mut self.index,
            approved: &approved,
            cancel,
            on_work: &mut || {},
            busy_window: Duration::ZERO,
            maintain,
        })
    }

    fn try_pass_with(&mut self, cancel: &mut dyn FnMut() -> bool) -> Result<PassReport, PassError> {
        self.try_pass(None, cancel)
    }

    fn pass(&mut self, maintain: Option<Micros>) -> PassReport {
        match self.try_pass(maintain, &mut || false) {
            Ok(report) => report,
            Err(PassError::Root(e)) => panic!("pass blocked: {e:?}"),
            Err(PassError::Other(e)) => panic!("pass failed: {e}"),
        }
    }

    /// A full list, then one pass that must get as far as acting, clean-up
    /// included: what the engine does when a full list is due.
    fn sync(&mut self) -> PassReport {
        let server_time = self.fetch();
        self.pass(Some(server_time))
    }

    /// A delta pull, then one pass: what the engine does every 30 seconds.
    fn poll(&mut self) -> PassReport {
        self.pull();
        self.pass(None)
    }

    /// One pass against the mirror as it was last read: what a pass is when
    /// the other machine has written since.
    fn stale_pass(&mut self) -> PassReport {
        self.pass(None)
    }

    /// Forget every failure on record, as `SyncHandle::retry_failed` does.
    fn retry_failed(&self) {
        let _ = std::fs::remove_file(&self.places.failures);
    }

    /// What `SyncHandle::confirm_deletions` leads to.
    fn confirm(&mut self, report: &PassReport) {
        self.approved = report.held.as_ref().unwrap().paths.iter().cloned().collect();
    }

    /// What `SyncHandle::decline_deletions` leads to.
    fn decline(&mut self, report: &PassReport) {
        let uid = self.cloud.uid();
        let (mut m, _) = Manifest::load_from(&self.places.manifest, &uid);
        for p in &report.held.as_ref().unwrap().paths {
            m.remove(p);
        }
        m.save_to(&self.places.manifest).unwrap();
    }
}

fn cloud(uid: &str) -> Arc<FakeCloud> {
    let c = FakeCloud::default();
    *c.uid.lock().unwrap() = uid.to_string();
    Arc::new(c)
}

/// Two machines holding the same `n` synced files.
fn synced_pair(test: &str, n: usize) -> (Arc<FakeCloud>, Machine, Machine) {
    let cloud = cloud("alice");
    let mut a = Machine::new(test, "a", &cloud);
    let mut b = Machine::new(test, "b", &cloud);
    for i in 0..n {
        a.write(&format!("docs/f{i:02}.txt"), format!("content {i}").as_bytes());
    }
    let r = a.sync();
    assert!(r.failures.is_empty() && r.held.is_none());
    let r = b.sync();
    assert!(r.failures.is_empty() && r.held.is_none());
    assert_eq!(a.files(), b.files());
    assert_eq!(a.files().len(), n);
    (cloud, a, b)
}

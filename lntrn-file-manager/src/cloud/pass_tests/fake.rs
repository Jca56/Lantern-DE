// The cloud, in memory: Firestore's docs with server stamps, versions and
// preconditions, Storage's blobs with their write times, and one clock for
// both that the tests can move.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::super::auth::AuthStop;
use super::super::firestore::{Expect, FileDoc, Pull, Put};
use super::super::http::HttpStatus;
use super::super::stamp::{self, Micros, SECOND};
use super::super::storage::BlobDelete;
use super::super::store::Store;
use super::super::transfer::BlobSource;

/// How every call to the fake cloud fails, while one of these is set.
#[derive(Clone, Copy)]
pub(super) enum Outage {
    Network,
    Quota,
    Revoked,
}

pub(super) type Hook = Box<dyn FnMut() + Send>;

struct Blob {
    bytes: Vec<u8>,
    written: Micros,
}

pub(super) struct FakeCloud {
    pub uid: Mutex<String>,
    blobs: Mutex<HashMap<String, Blob>>,
    pub docs: Mutex<HashMap<String, FileDoc>>,
    /// The server's clock. Moves a millisecond with everything the server
    /// does, and wherever a test puts it.
    clock: AtomicU64,
    /// Blob uploads that went through, and those that were tried.
    uploads: AtomicUsize,
    pub upload_tries: AtomicUsize,
    downloads: AtomicUsize,
    /// Full lists and delta queries.
    pub lists: AtomicUsize,
    pub deltas: AtomicUsize,
    /// What Firestore bills: one per doc handed out, at least one per query.
    pub reads: AtomicUsize,
    outage: Mutex<Option<Outage>>,
    /// Runs in the middle of a blob download: "the user did something while
    /// the transfer was running".
    during_download: Mutex<Option<Hook>>,
    /// Runs when an upload starts, before the file is read.
    before_upload: Mutex<Option<Hook>>,
    /// The Storage rule that allows deletes has not been published.
    pub deletes_forbidden: AtomicBool,
    /// Storage refuses every upload (403).
    pub uploads_refused: AtomicBool,
    /// Firestore refuses every write as if its precondition had failed.
    pub writes_refused: AtomicBool,
    /// Blob listings come back empty, whatever is stored.
    pub listing_broken: AtomicBool,
    /// Firestore answers the delta query with 400, as it would a query it
    /// cannot parse.
    pub deltas_refused: AtomicBool,
}

impl Default for FakeCloud {
    fn default() -> Self {
        Self {
            uid: Mutex::default(),
            blobs: Mutex::default(),
            docs: Mutex::default(),
            clock: AtomicU64::new(1_790_000_000 * SECOND),
            uploads: AtomicUsize::new(0),
            upload_tries: AtomicUsize::new(0),
            downloads: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            deltas: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            outage: Mutex::default(),
            during_download: Mutex::default(),
            before_upload: Mutex::default(),
            deletes_forbidden: AtomicBool::new(false),
            uploads_refused: AtomicBool::new(false),
            writes_refused: AtomicBool::new(false),
            listing_broken: AtomicBool::new(false),
            deltas_refused: AtomicBool::new(false),
        }
    }
}

fn run(hook: &Mutex<Option<Hook>>) {
    // Taken out while it runs: a hook may well sync another machine against
    // this same cloud.
    let taken = hook.lock().unwrap().take();
    if let Some(mut f) = taken {
        f();
        let mut slot = hook.lock().unwrap();
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

impl FakeCloud {
    pub fn live(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .docs
            .lock()
            .unwrap()
            .values()
            .filter(|d| !d.deleted)
            .map(|d| d.path.clone())
            .collect();
        v.sort();
        v
    }

    pub fn tombstones(&self) -> usize {
        self.docs.lock().unwrap().values().filter(|d| d.deleted).count()
    }

    pub fn doc(&self, path: &str) -> FileDoc {
        self.docs.lock().unwrap()[path].clone()
    }

    pub fn uploads(&self) -> usize {
        self.uploads.load(Ordering::SeqCst)
    }

    pub fn downloads(&self) -> usize {
        self.downloads.load(Ordering::SeqCst)
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    pub fn on_download(&self, hook: Option<Hook>) {
        *self.during_download.lock().unwrap() = hook;
    }

    pub fn on_upload(&self, hook: Option<Hook>) {
        *self.before_upload.lock().unwrap() = hook;
    }

    pub fn set_outage(&self, outage: Option<Outage>) {
        *self.outage.lock().unwrap() = outage;
    }

    /// The server's time now.
    pub fn now(&self) -> Micros {
        self.clock.load(Ordering::SeqCst)
    }

    /// Let server time pass.
    pub fn advance(&self, by: Micros) {
        self.clock.fetch_add(by, Ordering::SeqCst);
    }

    fn tick(&self) -> Micros {
        self.clock.fetch_add(1000, Ordering::SeqCst) + 1000
    }

    pub fn blob(&self, sha256: &str) -> Option<Vec<u8>> {
        self.blobs.lock().unwrap().get(sha256).map(|b| b.bytes.clone())
    }

    /// Store a blob as another machine's upload would.
    pub fn put_blob(&self, bytes: &[u8]) {
        let written = self.tick();
        self.blobs.lock().unwrap().insert(
            super::super::hash::sha256_bytes(bytes),
            Blob {
                bytes: bytes.to_vec(),
                written,
            },
        );
    }

    pub fn blob_count(&self) -> usize {
        self.blobs.lock().unwrap().len()
    }

    /// Bit rot, or a blob stored under the wrong name by a build that did
    /// not check.
    pub fn corrupt_blob(&self, sha256: &str) {
        let mut blobs = self.blobs.lock().unwrap();
        let blob = blobs.get_mut(sha256).expect("blob to corrupt");
        blob.bytes[0] ^= 0xff;
    }

    pub fn repair_blob(&self, sha256: &str) {
        self.corrupt_blob(sha256);
    }

    pub fn lose_blob(&self, sha256: &str) {
        self.blobs.lock().unwrap().remove(sha256);
    }

    fn check(&self) -> anyhow::Result<()> {
        match *self.outage.lock().unwrap() {
            None => Ok(()),
            Some(Outage::Network) => Err(anyhow::anyhow!("GET https://x: Connection refused")),
            Some(Outage::Quota) => Err(anyhow::anyhow!("GET https://x: status code 429")),
            Some(Outage::Revoked) => Err(anyhow::Error::new(AuthStop::Revoked)),
        }
    }

    fn bill(&self, docs: usize) {
        self.reads.fetch_add(docs.max(1), Ordering::SeqCst);
    }
}

impl Store for FakeCloud {
    fn uid(&self) -> String {
        self.uid.lock().unwrap().clone()
    }

    fn list_all(&self) -> anyhow::Result<Vec<FileDoc>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        let docs: Vec<FileDoc> = self.docs.lock().unwrap().values().cloned().collect();
        self.bill(docs.len());
        Ok(docs)
    }

    fn changed_since(&self, since: Micros) -> anyhow::Result<Pull> {
        self.deltas.fetch_add(1, Ordering::SeqCst);
        self.check()?;
        if self.deltas_refused.load(Ordering::SeqCst) {
            return Err(anyhow::Error::new(HttpStatus {
                method: "POST",
                url: "https://firestore/documents:runQuery".to_string(),
                code: 400,
                status: "INVALID_ARGUMENT".to_string(),
                message: "bad query".to_string(),
            }));
        }
        let docs: Vec<FileDoc> = self
            .docs
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.updated_at.is_some_and(|at| at > since))
            .cloned()
            .collect();
        self.bill(docs.len());
        Ok(Pull {
            docs,
            read_at: Some(self.tick()),
        })
    }

    fn server_time(&self) -> anyhow::Result<Option<Micros>> {
        self.check()?;
        self.bill(1);
        Ok(Some(self.tick()))
    }

    fn get_doc(&self, path: &str) -> anyhow::Result<Option<FileDoc>> {
        self.check()?;
        self.bill(1);
        Ok(self.docs.lock().unwrap().get(path).cloned())
    }

    fn put_doc(&self, doc: &FileDoc, expect: &Expect) -> anyhow::Result<Put> {
        self.check()?;
        if self.writes_refused.load(Ordering::SeqCst) {
            return Ok(Put::Moved);
        }
        let mut docs = self.docs.lock().unwrap();
        let holds = match (expect, docs.get(&doc.path)) {
            (Expect::Absent, None) => true,
            (Expect::Version(v), Some(cur)) => cur.version.as_deref() == Some(v.as_str()),
            _ => false,
        };
        if !holds {
            return Ok(Put::Moved);
        }
        let now = self.tick();
        let stored = FileDoc {
            updated_at: Some(now),
            // Like the real thing: the update time is close to the stamp,
            // not equal to it.
            version: Some(stamp::format(now + 7)),
            ..doc.clone()
        };
        docs.insert(doc.path.clone(), stored.clone());
        Ok(Put::Written(stored))
    }

    fn delete_doc(&self, path: &str, version: &str) -> anyhow::Result<bool> {
        self.check()?;
        let mut docs = self.docs.lock().unwrap();
        if docs.get(path).and_then(|d| d.version.as_deref()) != Some(version) {
            return Ok(false);
        }
        docs.remove(path);
        self.tick();
        Ok(true)
    }

    fn sha_in_use(&self, sha256: &str) -> anyhow::Result<bool> {
        self.check()?;
        self.bill(1);
        let docs = self.docs.lock().unwrap();
        Ok(docs.values().any(|d| !d.deleted && d.sha256 == sha256))
    }

    fn upload_blob(&self, src: &BlobSource, _mime: &str) -> anyhow::Result<()> {
        self.check()?;
        self.upload_tries.fetch_add(1, Ordering::SeqCst);
        run(&self.before_upload);
        // The body is read off the wire piece by piece; an error on the way
        // means the request never completed and nothing is stored.
        let mut bytes = Vec::new();
        src.open()?.read_to_end(&mut bytes)?;
        if self.uploads_refused.load(Ordering::SeqCst) {
            return Err(anyhow::Error::new(HttpStatus {
                method: "POST",
                url: "https://storage/o".to_string(),
                code: 403,
                status: String::new(),
                message: "Permission denied.".to_string(),
            }));
        }
        self.uploads.fetch_add(1, Ordering::SeqCst);
        let written = self.tick();
        self.blobs
            .lock()
            .unwrap()
            .insert(src.sha256.to_string(), Blob { bytes, written });
        Ok(())
    }

    fn download_blob(&self, sha256: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        self.check()?;
        self.downloads.fetch_add(1, Ordering::SeqCst);
        run(&self.during_download);
        let bytes = self
            .blob(sha256)
            .ok_or_else(|| anyhow::anyhow!("no blob {sha256}"))?;
        out.write_all(&bytes)?;
        Ok(())
    }

    fn list_blobs(&self) -> anyhow::Result<Vec<String>> {
        self.check()?;
        if self.listing_broken.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        Ok(self.blobs.lock().unwrap().keys().cloned().collect())
    }

    fn blob_written_at(&self, sha256: &str) -> anyhow::Result<Option<Micros>> {
        self.check()?;
        Ok(self.blobs.lock().unwrap().get(sha256).map(|b| b.written))
    }

    fn delete_blob(&self, sha256: &str) -> anyhow::Result<BlobDelete> {
        self.check()?;
        if self.deletes_forbidden.load(Ordering::SeqCst) {
            return Ok(BlobDelete::Forbidden);
        }
        self.blobs.lock().unwrap().remove(sha256);
        Ok(BlobDelete::Gone)
    }
}

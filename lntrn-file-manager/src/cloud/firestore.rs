// Firestore REST. One doc per synced file at
//   /users/{uid}/files2/{sha256 of the path}
//
// "files2", not "files": builds before 2026-10 filed each doc under the
// escaped path, with a client clock in updated_at, in /users/{uid}/files.
// The two schemes cannot share a collection. An old build on the other
// machine would put a second doc for a path next to this build's and could
// then take the stale one for the truth; this build would ignore everything
// the old one wrote and clean up its blobs as unused. With a collection (and
// a blob folder, storage.rs) of its own, each scheme only ever sees its own
// data.
//
// Document fields (Firestore typed-value JSON):
//   path        : string    — relative path inside ~/Cloud/: the file names
//                             exactly as they are on disk, joined with '/'
//   sha256      : string    — hash of current contents
//   size        : integer   — bytes
//   mtime       : integer   — unix seconds (informational only; sha is the source of truth)
//   mime        : string
//   device      : string    — uploader's hostname (used for conflict-rename label)
//   deleted     : bool      — tombstone for deletions
//   updated_at  : timestamp — set BY THE SERVER on every write (a REQUEST_TIME
//                             field transform); the field delta pulls query on
//
// ONE CLOCK. `updated_at`, a document's `updateTime` and a query's `readTime`
// all come from Firestore's clock. Neither machine's wall clock is part of
// what gets written or asked for, so a machine whose clock is wrong cannot
// stamp a write into the other's blind spot.
//
// CONDITIONAL WRITES. Every write names the version of the document it was
// decided against (`Expect`): its `updateTime`, or "there is none". If the
// other machine wrote in between, Firestore refuses the write (`Put::Moved`)
// and the caller looks again instead of overwriting.
//
// The document id is the sha256 of the path: a path can be longer than an id
// may be, can hold characters an id may not, and an id built by escaping is
// decoded differently by the URL and by the JSON side of the API. A hash has
// none of that. The path itself is in the `path` field.

use serde::{Deserialize, Serialize};

use super::hash::sha256_bytes;
use super::http::{status_of, Authed};
use super::stamp::{self, Micros};

pub fn doc_id_from_path(rel: &str) -> String {
    sha256_bytes(rel.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDoc {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub mtime: u64,
    pub mime: String,
    pub device: String,
    pub deleted: bool,
    /// When the server took the last write (server clock). `None` only on a
    /// doc that has not been written yet, or whose stamp could not be read.
    #[serde(default)]
    pub updated_at: Option<Micros>,
    /// Firestore's `updateTime` for this doc, exactly as sent: the name of
    /// this version, and the precondition for replacing it.
    #[serde(default)]
    pub version: Option<String>,
}

impl FileDoc {
    /// When this version was written, server clock.
    pub fn stamp(&self) -> Option<Micros> {
        self.updated_at
            .or_else(|| self.version.as_deref().and_then(stamp::parse))
    }
}

/// What the cloud must still hold at a path for a write to go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// No doc at all.
    Absent,
    /// The doc with this `FileDoc::version`.
    Version(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Put {
    /// Accepted. The doc as the server now holds it, stamp and version
    /// included (as far as the reply carried them).
    Written(FileDoc),
    /// Refused: the cloud no longer holds what `Expect` named.
    Moved,
}

/// What a query returned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pull {
    pub docs: Vec<FileDoc>,
    /// The server's time of the read. `None`: the reply named none.
    pub read_at: Option<Micros>,
}

/// A remote doc's `path` is joined onto ~/Cloud and written to. It must be a
/// plain relative path: no leading '/', no empty, "." or ".." component, no
/// NUL. Anything else could write or delete outside the sync root.
pub fn is_safe_rel_path(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains('\0')
        && rel.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// This machine's wall clock, unix millis. For scheduling on this machine
/// only (when the next full list is due, how long a retry waits): never
/// written to the cloud, never compared with a server stamp.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `projects/{p}/databases/(default)/documents`: how documents are named
/// inside request and response bodies.
fn documents_root(authed: &Authed) -> String {
    format!(
        "projects/{}/databases/(default)/documents",
        authed.cfg.project_id
    )
}

fn base_url(authed: &Authed) -> String {
    format!("{}/v1/{}", authed.cfg.firestore_url, documents_root(authed))
}

/// The collection of this document scheme (see the top of the file).
const COLLECTION: &str = "files2";

fn collection_url(authed: &Authed) -> String {
    format!("{}/users/{}/{COLLECTION}", base_url(authed), authed.user_id())
}

fn doc_url(authed: &Authed, rel_path: &str) -> String {
    format!("{}/{}", collection_url(authed), doc_id_from_path(rel_path))
}

fn doc_name(authed: &Authed, rel_path: &str) -> String {
    format!(
        "{}/users/{}/{COLLECTION}/{}",
        documents_root(authed),
        authed.user_id(),
        doc_id_from_path(rel_path)
    )
}

// ── Firestore typed value JSON helpers ─────────────────────────────────────

/// The fields a write sends. `updated_at` is not among them: the server
/// sets it.
fn to_fields(doc: &FileDoc) -> serde_json::Value {
    serde_json::json!({
        "path":    { "stringValue":  doc.path },
        "sha256":  { "stringValue":  doc.sha256 },
        "size":    { "integerValue": doc.size.to_string() },
        "mtime":   { "integerValue": doc.mtime.to_string() },
        "mime":    { "stringValue":  doc.mime },
        "device":  { "stringValue":  doc.device },
        "deleted": { "booleanValue": doc.deleted },
    })
}

/// A document resource (`name`, `fields`, `updateTime`) as a `FileDoc`.
/// `None` for anything sync must not act on: fields missing, a path that
/// would leave the sync root, or a doc filed under another id than its path
/// hashes to (two docs could then claim one path, and writes to the path's
/// own id would never reach this one).
fn from_document(doc: &serde_json::Value) -> Option<FileDoc> {
    let fields = doc.get("fields")?;
    let s = |k: &str| {
        fields
            .get(k)?
            .get("stringValue")?
            .as_str()
            .map(|s| s.to_string())
    };
    let i = |k: &str| {
        fields
            .get(k)?
            .get("integerValue")?
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
    };
    let b = |k: &str| fields.get(k)?.get("booleanValue")?.as_bool();
    let path = s("path")?;
    if !is_safe_rel_path(&path) {
        super::log_line(&format!("ignoring remote doc with unsafe path {path:?}"));
        return None;
    }
    let name = doc.get("name").and_then(|n| n.as_str()).unwrap_or("");
    if name.rsplit('/').next() != Some(doc_id_from_path(&path).as_str()) {
        super::log_line(&format!(
            "ignoring remote doc {name:?}: not filed under the id of its path {path:?}"
        ));
        return None;
    }
    Some(FileDoc {
        path,
        sha256: s("sha256")?,
        size: i("size")?,
        mtime: i("mtime")?,
        mime: s("mime").unwrap_or_default(),
        device: s("device").unwrap_or_default(),
        deleted: b("deleted").unwrap_or(false),
        updated_at: fields
            .get("updated_at")
            .and_then(|v| v.get("timestampValue"))
            .and_then(|t| t.as_str())
            .and_then(stamp::parse),
        version: doc
            .get("updateTime")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string()),
    })
}

/// The entries of a runQuery reply: a JSON array whose elements carry a
/// "document" (a result) and/or bookkeeping ("readTime", "done"). An empty
/// result is one element with only the read time.
fn parse_query(reply: &serde_json::Value) -> Pull {
    let mut pull = Pull::default();
    for entry in reply.as_array().into_iter().flatten() {
        if let Some(doc) = entry.get("document").and_then(from_document) {
            pull.docs.push(doc);
        }
        let read_at = entry
            .get("readTime")
            .and_then(|t| t.as_str())
            .and_then(stamp::parse);
        // The earliest one: nothing in the reply was read before it.
        pull.read_at = match (pull.read_at, read_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    pull
}

// ── Writes ─────────────────────────────────────────────────────────────────

fn precondition(expect: &Expect) -> serde_json::Value {
    match expect {
        Expect::Absent => serde_json::json!({ "exists": false }),
        Expect::Version(v) => serde_json::json!({ "updateTime": v }),
    }
}

/// Does this refusal mean "the document is not what the precondition
/// named"? Firestore answers FAILED_PRECONDITION for a version that moved
/// on, ALREADY_EXISTS for a doc that should not be there, NOT_FOUND for one
/// that should, ABORTED when it lost a race with another write.
fn is_precondition_refusal(code: u16, status: &str) -> bool {
    matches!(
        status,
        "FAILED_PRECONDITION" | "ALREADY_EXISTS" | "NOT_FOUND" | "ABORTED"
    ) || matches!(code, 409 | 412)
}

/// Send one write. `Ok(None)`: refused by its precondition. `Ok(Some)`: the
/// write result (`updateTime`, `transformResults`).
fn commit(authed: &Authed, write: serde_json::Value) -> anyhow::Result<Option<serde_json::Value>> {
    let url = format!("{}:commit", base_url(authed));
    let body = serde_json::json!({ "writes": [write] });
    match authed.post_json(&url, body) {
        Ok(resp) => {
            let v: serde_json::Value = resp.into_json()?;
            Ok(Some(
                v.get("writeResults")
                    .and_then(|r| r.get(0))
                    .cloned()
                    .unwrap_or_default(),
            ))
        }
        Err(e) => match status_of(&e) {
            Some(st) if is_precondition_refusal(st.code, &st.status) => Ok(None),
            _ => Err(e),
        },
    }
}

/// What a write result says the stored doc now looks like.
fn stored(doc: &FileDoc, result: &serde_json::Value) -> FileDoc {
    FileDoc {
        updated_at: result
            .get("transformResults")
            .and_then(|t| t.get(0))
            .and_then(|v| v.get("timestampValue"))
            .and_then(|t| t.as_str())
            .and_then(stamp::parse),
        version: result
            .get("updateTime")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string()),
        ..doc.clone()
    }
}

/// Create or replace the doc for `doc.path`, if the cloud still holds what
/// `expect` names. The server stamps `updated_at` with its own clock.
pub fn put(authed: &Authed, doc: &FileDoc, expect: &Expect) -> anyhow::Result<Put> {
    let write = serde_json::json!({
        "update": {
            "name": doc_name(authed, &doc.path),
            "fields": to_fields(doc),
        },
        "updateTransforms": [
            { "fieldPath": "updated_at", "setToServerValue": "REQUEST_TIME" }
        ],
        "currentDocument": precondition(expect),
    });
    Ok(match commit(authed, write)? {
        Some(result) => Put::Written(stored(doc, &result)),
        None => Put::Moved,
    })
}

/// Remove the doc for `rel_path` if it is still the version named. `false`:
/// it is not (changed, or already gone).
pub fn delete(authed: &Authed, rel_path: &str, version: &str) -> anyhow::Result<bool> {
    let write = serde_json::json!({
        "delete": doc_name(authed, rel_path),
        "currentDocument": precondition(&Expect::Version(version.to_string())),
    });
    Ok(commit(authed, write)?.is_some())
}

// ── Reads ──────────────────────────────────────────────────────────────────

/// The doc for one path as it is right now. `None`: there is none (or it is
/// one sync must not act on). One billed read.
pub fn get(authed: &Authed, rel_path: &str) -> anyhow::Result<Option<FileDoc>> {
    match authed.get(&doc_url(authed, rel_path)) {
        Ok(resp) => {
            let v: serde_json::Value = resp.into_json()?;
            Ok(from_document(&v).filter(|d| d.path == rel_path))
        }
        Err(e) if status_of(&e).is_some_and(|st| st.code == 404) => Ok(None),
        Err(e) => Err(e),
    }
}

fn run_query_raw(authed: &Authed, query: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let url = format!("{}/users/{}:runQuery", base_url(authed), authed.user_id());
    let body = serde_json::json!({ "structuredQuery": query });
    let resp = authed.post_json(&url, body)?;
    Ok(resp.into_json()?)
}

fn run_query(authed: &Authed, query: serde_json::Value) -> anyhow::Result<Pull> {
    Ok(parse_query(&run_query_raw(authed, query)?))
}

/// Incremental pull: every doc the server stamped after `since`, and the
/// server's time of the read. Firestore bills one read per RETURNED doc
/// (minimum one per query), so a quiet poll costs one read.
pub fn query_changed_since(authed: &Authed, since: Micros) -> anyhow::Result<Pull> {
    run_query(
        authed,
        serde_json::json!({
            "from": [{ "collectionId": COLLECTION }],
            "where": {
                "fieldFilter": {
                    "field": { "fieldPath": "updated_at" },
                    "op": "GREATER_THAN",
                    "value": { "timestampValue": stamp::format(since) },
                }
            },
            "orderBy": [
                { "field": { "fieldPath": "updated_at" }, "direction": "ASCENDING" }
            ],
        }),
    )
}

/// The server's clock, for one billed read. `None`: it would not say.
pub fn server_time(authed: &Authed) -> anyhow::Result<Option<Micros>> {
    let asked = run_query(
        authed,
        serde_json::json!({ "from": [{ "collectionId": COLLECTION }], "limit": 1 }),
    );
    match asked {
        Ok(pull) => Ok(pull.read_at),
        // The time makes the first delta tighter and the clean-up possible;
        // the full list that follows this call does not depend on it. A
        // request the server will not take must not stand in the list's way.
        Err(e) if status_of(&e).is_some_and(|st| matches!(st.code, 400 | 403 | 404)) => {
            super::log_line(&format!("the cloud did not tell its time: {e}"));
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Does any doc name `sha256` as its content right now? Asked of the server,
/// not of the mirror: this is the last check before a blob is deleted.
pub fn sha_in_use(authed: &Authed, sha256: &str) -> anyhow::Result<bool> {
    let reply = run_query_raw(
        authed,
        serde_json::json!({
            "from": [{ "collectionId": COLLECTION }],
            "where": {
                "fieldFilter": {
                    "field": { "fieldPath": "sha256" },
                    "op": "EQUAL",
                    "value": { "stringValue": sha256 },
                }
            },
            "limit": 1,
        }),
    )?;
    in_use_reply(&reply)
}

/// Read the answer to "does any doc name this hash?" off the raw reply. Any
/// document counts, also one sync itself would not accept (filed under
/// another id, an odd path, a field missing): what it names may be the only
/// copy of somebody's file. And "no" is only taken from a reply that
/// plainly says so; anything else is an error, and the blob stays.
fn in_use_reply(reply: &serde_json::Value) -> anyhow::Result<bool> {
    let entries = reply
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("the cloud's answer to the blob check is not a list"))?;
    if entries.iter().any(|e| e.get("document").is_some()) {
        return Ok(true);
    }
    let said_no = entries.iter().all(|e| e.get("error").is_none())
        && entries.iter().any(|e| e.get("readTime").is_some());
    if said_no {
        Ok(false)
    } else {
        anyhow::bail!("the cloud's answer to the blob check could not be read")
    }
}

pub fn list_all(authed: &Authed) -> anyhow::Result<Vec<FileDoc>> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!("{}?pageSize=300", collection_url(authed));
        if let Some(t) = &page_token {
            url.push_str(&format!("&pageToken={}", urlencoding::encode(t)));
        }
        let resp = authed.get(&url)?;
        let v: serde_json::Value = resp.into_json()?;
        if let Some(docs) = v.get("documents").and_then(|d| d.as_array()) {
            out.extend(docs.iter().filter_map(from_document));
        }
        page_token = v
            .get("nextPageToken")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());
        if page_token.is_none() {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

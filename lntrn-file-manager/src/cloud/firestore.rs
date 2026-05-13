// Firestore REST. One doc per synced file at
//   /users/{uid}/files/{pathEncoded}
//
// Document fields (Firestore typed-value JSON):
//   path        : string   — relative path inside ~/Cloud/, forward-slashed
//   sha256      : string   — hash of current contents
//   size        : integer  — bytes
//   mtime       : integer  — unix seconds (informational only; sha is the source of truth)
//   mime        : string
//   device      : string   — uploader's hostname (used for conflict-rename label)
//   deleted     : bool     — tombstone for deletions
//   updated_at  : timestamp — server-side; used for incremental pull
//
// pathEncoded replaces '/' with '__' since Firestore doc IDs can't contain '/'.
// Listing uses GET /users/{uid}/files (paginated by pageToken).

use serde::{Deserialize, Serialize};

use super::http::Authed;

pub fn doc_id_from_path(rel: &str) -> String {
    // Firestore forbids '/' in doc IDs and treats '.' / '..' specially.
    // Use a reversible encoding: percent-encode then swap '/' for '__'.
    let enc = urlencoding::encode(rel).into_owned();
    enc.replace('/', "__")
}

pub fn path_from_doc_id(id: &str) -> String {
    let with_slash = id.replace("__", "/");
    urlencoding::decode(&with_slash)
        .map(|s| s.into_owned())
        .unwrap_or(with_slash)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDoc {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub mtime: u64,
    pub mime: String,
    pub device: String,
    pub deleted: bool,
}

fn base_url(authed: &Authed) -> String {
    format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents",
        authed.cfg.project_id
    )
}

fn collection_url(authed: &Authed) -> String {
    format!("{}/users/{}/files", base_url(authed), authed.user_id())
}

fn doc_url(authed: &Authed, rel_path: &str) -> String {
    format!("{}/{}", collection_url(authed), doc_id_from_path(rel_path))
}

// ── Firestore typed value JSON helpers ─────────────────────────────────────

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

fn from_fields(fields: &serde_json::Value) -> Option<FileDoc> {
    let s = |k: &str| fields.get(k)?.get("stringValue")?.as_str().map(|s| s.to_string());
    let i = |k: &str| {
        fields
            .get(k)?
            .get("integerValue")?
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
    };
    let b = |k: &str| fields.get(k)?.get("booleanValue")?.as_bool();
    Some(FileDoc {
        path: s("path")?,
        sha256: s("sha256")?,
        size: i("size")?,
        mtime: i("mtime")?,
        mime: s("mime").unwrap_or_default(),
        device: s("device").unwrap_or_default(),
        deleted: b("deleted").unwrap_or(false),
    })
}

// ── CRUD ───────────────────────────────────────────────────────────────────

pub fn put(authed: &Authed, doc: &FileDoc) -> anyhow::Result<()> {
    let url = doc_url(authed, &doc.path);
    let body = serde_json::json!({ "fields": to_fields(doc) });
    authed.patch_json(&url, body)?;
    Ok(())
}

pub fn get(authed: &Authed, rel_path: &str) -> anyhow::Result<Option<FileDoc>> {
    let url = doc_url(authed, rel_path);
    match authed.get(&url) {
        Ok(resp) => {
            let v: serde_json::Value = resp.into_json()?;
            Ok(v.get("fields").and_then(from_fields))
        }
        Err(e) => {
            if format!("{e}").contains("404") {
                Ok(None)
            } else {
                Err(e)
            }
        }
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
            for d in docs {
                if let Some(fields) = d.get("fields") {
                    if let Some(fd) = from_fields(fields) {
                        out.push(fd);
                    }
                }
            }
        }
        page_token = v.get("nextPageToken").and_then(|t| t.as_str()).map(|s| s.to_string());
        if page_token.is_none() {
            break;
        }
    }
    Ok(out)
}

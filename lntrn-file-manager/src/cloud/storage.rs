// Firebase Storage REST. Blobs are content-addressed (sha256) under
// users/{uid}/blobs2/{sha256}. Storing by hash gives us free dedup.
// ("blobs2": the folder of this document scheme, see firestore.rs. The
// clean-up lists this folder only, so it never meets a blob an older build
// uploaded.)
//
// Endpoints:
//   Upload:   POST   https://firebasestorage.googleapis.com/v0/b/{bucket}/o?name={url_encoded_object}
//   Download: GET    https://firebasestorage.googleapis.com/v0/b/{bucket}/o/{url_encoded_object}?alt=media
//   Metadata: GET    .../o/{url_encoded_object}
//   Delete:   DELETE .../o/{url_encoded_object}
//   List:     GET    .../o?prefix={url_encoded_prefix}&delimiter=/&maxResults=N[&pageToken=T]
//
// All require Authorization: Bearer {id_token}.
//
// Both transfers stream: an upload is read from the file as it is sent, a
// download is written out as it arrives. Neither is held in memory.

use std::io::{Read, Write};

use super::hash::is_sha256_hex;
use super::http::{status_of, Authed};
use super::stamp::{self, Micros};
use super::transfer::BlobSource;

/// storage.rules refuses objects of this size and up. A file that big is
/// never read, hashed or sent: it is reported as one that cannot be synced.
/// Keep the two in step.
pub const MAX_BLOB_BYTES: u64 = 1024 * 1024 * 1024;

fn blob_prefix(uid: &str) -> String {
    format!("users/{uid}/blobs2/")
}

fn objects_url(authed: &Authed) -> String {
    format!("{}/v0/b/{}/o", authed.cfg.storage_url, authed.cfg.storage_bucket)
}

fn object_url(authed: &Authed, sha256: &str) -> String {
    let name = format!("{}{sha256}", blob_prefix(&authed.user_id()));
    format!("{}/{}", objects_url(authed), urlencoding::encode(&name))
}

/// Store the file behind `src` as the blob named by its hash. The body is
/// only ever complete if its bytes hash to that name (see transfer.rs); when
/// the upload fails, `src.changed()` says whether that was why.
pub fn upload_blob(authed: &Authed, src: &BlobSource, mime: &str) -> anyhow::Result<()> {
    let name = format!("{}{}", blob_prefix(&authed.user_id()), src.sha256);
    let url = format!("{}?name={}", objects_url(authed), urlencoding::encode(&name));
    authed.post_stream(&url, mime, src.size, &mut || {
        src.open().map(|r| Box::new(r) as Box<dyn Read + '_>)
    })?;
    Ok(())
}

/// Write the blob `sha256` into `out`. The caller verifies what arrived.
pub fn download_blob(authed: &Authed, sha256: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    let url = format!("{}?alt=media", object_url(authed, sha256));
    let resp = authed.get(&url)?;
    std::io::copy(&mut resp.into_reader(), out)?;
    Ok(())
}

/// The names (sha256) of every blob the account holds.
pub fn list_blobs(authed: &Authed) -> anyhow::Result<Vec<String>> {
    let prefix = blob_prefix(&authed.user_id());
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!(
            "{}?prefix={}&delimiter=%2F&maxResults=1000",
            objects_url(authed),
            urlencoding::encode(&prefix)
        );
        if let Some(t) = &page_token {
            url.push_str(&format!("&pageToken={}", urlencoding::encode(t)));
        }
        let v: serde_json::Value = authed.get(&url)?.into_json()?;
        out.extend(blob_names(&v, &prefix));
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

/// The blob names in one page of a listing. Anything that is not
/// `{prefix}{sha256}` is not a blob of ours and is left out (and so left
/// alone).
fn blob_names(page: &serde_json::Value, prefix: &str) -> Vec<String> {
    page.get("items")
        .and_then(|i| i.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name")?.as_str()?.strip_prefix(prefix))
        .filter(|name| is_sha256_hex(name))
        .map(|name| name.to_string())
        .collect()
}

/// When the blob was last written, server clock. `None`: no such blob, or
/// the metadata named no time that can be read.
pub fn blob_written_at(authed: &Authed, sha256: &str) -> anyhow::Result<Option<Micros>> {
    match authed.get(&object_url(authed, sha256)) {
        Ok(resp) => Ok(written_at(&resp.into_json()?)),
        Err(e) if status_of(&e).is_some_and(|st| st.code == 404) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The later of an object's `updated` and `timeCreated`: an upload over an
/// existing name makes a new object, so either says "written again".
fn written_at(meta: &serde_json::Value) -> Option<Micros> {
    let time = |k: &str| meta.get(k).and_then(|t| t.as_str()).and_then(stamp::parse);
    match (time("updated"), time("timeCreated")) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobDelete {
    /// Deleted, or already gone.
    Gone,
    /// The Storage rules do not allow deletes (yet): see storage.rules.
    Forbidden,
}

pub fn delete_blob(authed: &Authed, sha256: &str) -> anyhow::Result<BlobDelete> {
    match authed.delete(&object_url(authed, sha256)) {
        Ok(_) => Ok(BlobDelete::Gone),
        Err(e) => match status_of(&e).map(|st| st.code) {
            Some(404) => Ok(BlobDelete::Gone),
            Some(403) => Ok(BlobDelete::Forbidden),
            _ => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_yields_only_blobs_under_the_prefix() {
        let sha = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let page = serde_json::json!({
            "prefixes": ["users/u/blobs/sub/"],
            "items": [
                { "name": format!("users/u/blobs/{sha}"), "bucket": "b" },
                { "name": "users/u/blobs/not-a-hash", "bucket": "b" },
                { "name": format!("users/other/blobs/{sha}"), "bucket": "b" },
                { "bucket": "b" },
            ],
        });
        assert_eq!(blob_names(&page, "users/u/blobs/"), [sha]);
        // An empty folder, or a reply of another shape: nothing.
        assert!(blob_names(&serde_json::json!({}), "users/u/blobs/").is_empty());
        assert!(blob_names(&serde_json::json!([1, 2]), "users/u/blobs/").is_empty());
    }

    #[test]
    fn written_at_is_the_latest_time_the_metadata_names() {
        let meta = serde_json::json!({
            "name": "users/u/blobs/x",
            "timeCreated": "2026-10-01T00:00:00.000Z",
            "updated": "2026-10-02T09:15:04.123Z",
        });
        assert_eq!(written_at(&meta), stamp::parse("2026-10-02T09:15:04.123Z"));
        let only_created = serde_json::json!({ "timeCreated": "2026-10-01T00:00:00.000Z" });
        assert_eq!(written_at(&only_created), stamp::parse("2026-10-01T00:00:00Z"));
        // No readable time: unknown, which is never "old enough to delete".
        assert_eq!(written_at(&serde_json::json!({ "updated": "yesterday" })), None);
    }
}

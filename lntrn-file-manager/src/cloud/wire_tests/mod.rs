// The requests as they go over the wire, against a small HTTP server on
// localhost that plays Firestore and Storage: what is sent (method, path,
// headers, body), and what each kind of answer is taken to mean. The answers
// are written after the REST references of the two services; that they match
// what Google really sends is the one thing only the real cloud can prove.

mod server;

use super::firestore::{self, Expect, FileDoc, Put};
use super::hash::sha256_bytes;
use super::http::status_of;
use super::stamp;
use super::storage::{self, BlobDelete};
use super::transfer::{BlobSource, HashingWriter};
use super::TestDir;

use server::Server;

fn doc(path: &str) -> FileDoc {
    FileDoc {
        path: path.to_string(),
        sha256: sha256_bytes(b"content"),
        size: 7,
        mtime: 1_790_000_000,
        mime: "text/plain".to_string(),
        device: "pc".to_string(),
        deleted: false,
        updated_at: None,
        version: None,
    }
}

fn document_json(path: &str, version: &str) -> serde_json::Value {
    serde_json::json!({
        "name": format!(
            "projects/proj/databases/(default)/documents/users/uid1/files2/{}",
            firestore::doc_id_from_path(path)
        ),
        "fields": {
            "path":    { "stringValue": path },
            "sha256":  { "stringValue": sha256_bytes(b"content") },
            "size":    { "integerValue": "7" },
            "mtime":   { "integerValue": "1790000000" },
            "mime":    { "stringValue": "text/plain" },
            "device":  { "stringValue": "laptop" },
            "deleted": { "booleanValue": false },
            "updated_at": { "timestampValue": "2026-10-02T09:15:04.123Z" },
        },
        "createTime": "2026-10-01T00:00:00.000001Z",
        "updateTime": version,
    })
}

fn error_json(code: u16, status: &str) -> String {
    serde_json::json!({ "error": { "code": code, "message": "refused", "status": status } })
        .to_string()
}

const DOCS: &str = "/v1/projects/proj/databases/(default)/documents";

#[test]
fn a_doc_write_is_conditional_and_stamped_by_the_server() {
    let written = serde_json::json!({
        "writeResults": [{
            "updateTime": "2026-10-02T09:15:04.130001Z",
            "transformResults": [{ "timestampValue": "2026-10-02T09:15:04.123Z" }],
        }],
        "commitTime": "2026-10-02T09:15:04.130001Z",
    })
    .to_string();
    let server = Server::scripted(vec![
        (200, written.clone()),
        (200, written),
        (400, error_json(400, "FAILED_PRECONDITION")),
        (409, error_json(409, "ALREADY_EXISTS")),
        (403, error_json(403, "PERMISSION_DENIED")),
    ]);
    let authed = server.authed();
    let new = doc("docs/a b.txt");

    // A new doc: must not exist yet.
    let put = firestore::put(&authed, &new, &Expect::Absent).unwrap();
    let Put::Written(stored) = put else {
        panic!("not written");
    };
    assert_eq!(stored.updated_at, stamp::parse("2026-10-02T09:15:04.123Z"));
    assert_eq!(stored.version.as_deref(), Some("2026-10-02T09:15:04.130001Z"));
    assert_eq!(stored.path, new.path);

    // A replacement: must still be the version that was seen.
    let seen_version = "2026-10-01T10:00:00.000001Z".to_string();
    firestore::put(&authed, &new, &Expect::Version(seen_version.clone())).unwrap();
    // Refused preconditions are "moved", not errors...
    for expect in [Expect::Version(seen_version), Expect::Absent] {
        assert_eq!(firestore::put(&authed, &new, &expect).unwrap(), Put::Moved);
    }
    // ...and a refusal by the rules is an error that keeps its status.
    let e = firestore::put(&authed, &new, &Expect::Absent).unwrap_err();
    assert_eq!(status_of(&e).unwrap().status, "PERMISSION_DENIED");

    let seen = server.seen();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].target, format!("{DOCS}:commit"));
    assert_eq!(seen[0].header("authorization"), Some("bearer the-id-token"));
    let write = &seen[0].json()["writes"][0];
    assert_eq!(
        write["update"]["name"],
        format!(
            "projects/proj/databases/(default)/documents/users/uid1/files2/{}",
            firestore::doc_id_from_path("docs/a b.txt")
        )
    );
    assert_eq!(write["update"]["fields"]["path"]["stringValue"], "docs/a b.txt");
    assert_eq!(write["update"]["fields"]["size"]["integerValue"], "7");
    // The stamp is the server's to set, never sent.
    assert!(write["update"]["fields"].get("updated_at").is_none());
    assert_eq!(
        write["updateTransforms"],
        serde_json::json!([{ "fieldPath": "updated_at", "setToServerValue": "REQUEST_TIME" }])
    );
    assert_eq!(write["currentDocument"], serde_json::json!({ "exists": false }));
    assert_eq!(
        seen[1].json()["writes"][0]["currentDocument"],
        serde_json::json!({ "updateTime": "2026-10-01T10:00:00.000001Z" })
    );
}

#[test]
fn a_doc_is_fetched_and_deleted_by_the_id_of_its_path() {
    let version = "2026-10-02T09:15:04.130001Z";
    let server = Server::scripted(vec![
        (200, document_json("docs/a.txt", version).to_string()),
        (404, error_json(404, "NOT_FOUND")),
        (500, error_json(500, "INTERNAL")),
        (200, serde_json::json!({ "writeResults": [{}], "commitTime": version }).to_string()),
        (400, error_json(400, "FAILED_PRECONDITION")),
        (404, error_json(404, "NOT_FOUND")),
    ]);
    let authed = server.authed();

    let got = firestore::get(&authed, "docs/a.txt").unwrap().unwrap();
    assert_eq!(got.device, "laptop");
    assert_eq!(got.version.as_deref(), Some(version));
    assert_eq!(firestore::get(&authed, "docs/a.txt").unwrap(), None);
    // A server error is not "there is no doc".
    assert!(firestore::get(&authed, "docs/a.txt").is_err());

    assert!(firestore::delete(&authed, "docs/a.txt", version).unwrap());
    // Changed since, or gone already: not deleted, not an error.
    assert!(!firestore::delete(&authed, "docs/a.txt", version).unwrap());
    assert!(!firestore::delete(&authed, "docs/a.txt", version).unwrap());

    let seen = server.seen();
    let id = firestore::doc_id_from_path("docs/a.txt");
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].target, format!("{DOCS}/users/uid1/files2/{id}"));
    let write = &seen[3].json()["writes"][0];
    assert_eq!(
        write["delete"],
        format!("projects/proj/databases/(default)/documents/users/uid1/files2/{id}")
    );
    assert_eq!(write["currentDocument"], serde_json::json!({ "updateTime": version }));
}

#[test]
fn queries_ask_on_the_servers_clock_and_report_its_time() {
    let read_time = "2026-10-02T09:20:00.500000Z";
    let server = Server::scripted(vec![
        (
            200,
            serde_json::json!([
                { "document": document_json("a.txt", "2026-10-02T09:15:04.130001Z"), "readTime": read_time },
            ])
            .to_string(),
        ),
        (200, serde_json::json!([{ "readTime": read_time }]).to_string()),
        (200, serde_json::json!([{ "readTime": read_time }]).to_string()),
        (
            200,
            serde_json::json!([
                { "document": document_json("a.txt", "2026-10-02T09:15:04.130001Z"), "readTime": read_time },
            ])
            .to_string(),
        ),
        (429, format!("[{}]", error_json(429, "RESOURCE_EXHAUSTED"))),
        (400, format!("[{}]", error_json(400, "INVALID_ARGUMENT"))),
        (503, "unavailable".to_string()),
    ]);
    let authed = server.authed();
    let since = stamp::parse("2026-10-02T09:13:00Z").unwrap();

    let pull = firestore::query_changed_since(&authed, since).unwrap();
    assert_eq!(pull.docs.len(), 1);
    assert_eq!(pull.read_at, stamp::parse(read_time));
    assert_eq!(firestore::server_time(&authed).unwrap(), stamp::parse(read_time));
    let sha = sha256_bytes(b"content");
    assert!(!firestore::sha_in_use(&authed, &sha).unwrap());
    assert!(firestore::sha_in_use(&authed, &sha).unwrap());
    // Out of quota, on a streaming call: still recognisably so.
    let e = firestore::query_changed_since(&authed, since).unwrap_err();
    assert!(super::reconcile::is_quota_error(&format!("{e:#}")));
    assert_eq!(status_of(&e).unwrap().status, "RESOURCE_EXHAUSTED");
    // The server not taking the time question is "no time", so the list
    // that follows still runs; the server being down is an error.
    assert_eq!(firestore::server_time(&authed).unwrap(), None);
    assert!(firestore::server_time(&authed).is_err());

    let seen = server.seen();
    assert_eq!(seen[0].target, format!("{DOCS}/users/uid1:runQuery"));
    let query = &seen[0].json()["structuredQuery"];
    assert_eq!(query["from"], serde_json::json!([{ "collectionId": "files2" }]));
    let filter = &query["where"]["fieldFilter"];
    assert_eq!(filter["field"]["fieldPath"], "updated_at");
    assert_eq!(filter["op"], "GREATER_THAN");
    assert_eq!(filter["value"]["timestampValue"], "2026-10-02T09:13:00.000000Z");
    assert_eq!(seen[1].json()["structuredQuery"]["limit"], 1);
    let by_sha = &seen[2].json()["structuredQuery"];
    assert_eq!(by_sha["where"]["fieldFilter"]["field"]["fieldPath"], "sha256");
    assert_eq!(by_sha["where"]["fieldFilter"]["value"]["stringValue"], sha);
}

#[test]
fn a_full_list_follows_the_pages() {
    let v = "2026-10-02T09:15:04.130001Z";
    let server = Server::scripted(vec![
        (
            200,
            serde_json::json!({
                "documents": [document_json("a.txt", v), document_json("b.txt", v)],
                "nextPageToken": "page 2/token",
            })
            .to_string(),
        ),
        (200, serde_json::json!({ "documents": [document_json("c.txt", v)] }).to_string()),
        // An empty collection is an empty object.
        (200, "{}".to_string()),
    ]);
    let authed = server.authed();
    let docs = firestore::list_all(&authed).unwrap();
    let paths: Vec<&str> = docs.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "b.txt", "c.txt"]);
    assert!(firestore::list_all(&authed).unwrap().is_empty());

    let seen = server.seen();
    assert_eq!(seen[0].target, format!("{DOCS}/users/uid1/files2?pageSize=300"));
    assert_eq!(
        seen[1].target,
        format!("{DOCS}/users/uid1/files2?pageSize=300&pageToken=page%202%2Ftoken")
    );
}

#[test]
fn an_upload_streams_the_file_and_never_completes_a_changed_one() {
    let dir = TestDir::new("wire-upload");
    let path = dir.join("big.bin");
    // Bigger than any buffer on the way, so it really goes out in pieces.
    let content: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &content).unwrap();
    let sha = sha256_bytes(&content);
    let server = Server::scripted(vec![(200, "{}".to_string()), (200, "{}".to_string())]);
    let authed = server.authed();

    let src = BlobSource::new(&path, &sha, content.len() as u64);
    storage::upload_blob(&authed, &src, "application/octet-stream").unwrap();
    let seen = server.seen();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].target,
        format!("/v0/b/bucket/o?name=users%2Fuid1%2Fblobs2%2F{sha}")
    );
    assert_eq!(seen[0].header("content-length"), Some("3000000"));
    assert_eq!(seen[0].header("content-type"), Some("application/octet-stream"));
    assert!(seen[0].header("transfer-encoding").is_none());
    assert!(seen[0].complete);
    assert_eq!(sha256_bytes(&seen[0].body), sha);

    // The same file, one byte different from what was hashed: the request
    // ends before its last bytes, and the server has no complete body to
    // store under the old name.
    let mut changed = content.clone();
    changed[1_500_000] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    let src = BlobSource::new(&path, &sha, content.len() as u64);
    assert!(storage::upload_blob(&authed, &src, "application/octet-stream").is_err());
    assert!(src.changed());
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert!(!seen[1].complete);
    assert!(seen[1].body.len() < content.len());

    // A file that cannot be opened is an error about the file.
    let gone = dir.join("gone.bin");
    let src = BlobSource::new(&gone, &sha, 5);
    let e = storage::upload_blob(&authed, &src, "text/plain").unwrap_err();
    assert!(!src.changed());
    assert!(e.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(server.seen().len(), 2);
}

#[test]
fn a_download_streams_into_the_writer_it_is_given() {
    let content = "x".repeat(200_000);
    let server = Server::scripted(vec![(200, content.clone()), (404, "{}".to_string())]);
    let authed = server.authed();
    let sha = sha256_bytes(content.as_bytes());

    let mut out = HashingWriter::new(Vec::new());
    storage::download_blob(&authed, &sha, &mut out).unwrap();
    assert_eq!(out.finish(), (sha.clone(), 200_000));
    let seen = server.seen();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(
        seen[0].target,
        format!("/v0/b/bucket/o/users%2Fuid1%2Fblobs2%2F{sha}?alt=media")
    );

    let mut out = Vec::new();
    let e = storage::download_blob(&authed, &sha, &mut out).unwrap_err();
    assert_eq!(status_of(&e).unwrap().code, 404);
    assert!(out.is_empty());
}

#[test]
fn blobs_are_listed_dated_and_deleted() {
    let (a, b) = (sha256_bytes(b"a"), sha256_bytes(b"b"));
    let server = Server::scripted(vec![
        (
            200,
            serde_json::json!({
                "prefixes": [],
                "items": [{ "name": format!("users/uid1/blobs2/{a}"), "bucket": "bucket" }],
                "nextPageToken": "next",
            })
            .to_string(),
        ),
        (
            200,
            serde_json::json!({
                "items": [{ "name": format!("users/uid1/blobs2/{b}"), "bucket": "bucket" }],
            })
            .to_string(),
        ),
        (
            200,
            serde_json::json!({
                "name": format!("users/uid1/blobs2/{a}"),
                "size": "1",
                "timeCreated": "2026-10-01T08:00:00.000Z",
                "updated": "2026-10-01T08:00:00.000Z",
            })
            .to_string(),
        ),
        (404, serde_json::json!({ "error": { "code": 404, "message": "Not Found." } }).to_string()),
        (204, String::new()),
        (404, serde_json::json!({ "error": { "code": 404, "message": "Not Found." } }).to_string()),
        (403, serde_json::json!({ "error": { "code": 403, "message": "Permission denied." } }).to_string()),
        (503, "busy".to_string()),
    ]);
    let authed = server.authed();

    assert_eq!(storage::list_blobs(&authed).unwrap(), [a.clone(), b.clone()]);
    assert_eq!(
        storage::blob_written_at(&authed, &a).unwrap(),
        stamp::parse("2026-10-01T08:00:00Z")
    );
    assert_eq!(storage::blob_written_at(&authed, &b).unwrap(), None);
    assert_eq!(storage::delete_blob(&authed, &a).unwrap(), BlobDelete::Gone);
    assert_eq!(storage::delete_blob(&authed, &a).unwrap(), BlobDelete::Gone);
    // The rule that allows deletes is not published: told apart from an
    // error, so the caller can note it once and carry on.
    assert_eq!(storage::delete_blob(&authed, &a).unwrap(), BlobDelete::Forbidden);
    assert!(storage::delete_blob(&authed, &a).is_err());

    let seen = server.seen();
    assert_eq!(
        seen[0].target,
        "/v0/b/bucket/o?prefix=users%2Fuid1%2Fblobs2%2F&delimiter=%2F&maxResults=1000"
    );
    assert!(seen[1].target.ends_with("&pageToken=next"));
    assert_eq!(seen[2].target, format!("/v0/b/bucket/o/users%2Fuid1%2Fblobs2%2F{a}"));
    assert_eq!((seen[4].method.as_str(), &seen[4].target), ("DELETE", &seen[2].target));
}

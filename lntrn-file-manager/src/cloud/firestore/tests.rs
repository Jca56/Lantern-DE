use super::*;

#[test]
fn rejects_paths_that_escape_the_root() {
    assert!(is_safe_rel_path("a.txt"));
    assert!(is_safe_rel_path("dir/sub/a b.txt"));
    assert!(is_safe_rel_path("..hidden/a..b"));
    assert!(!is_safe_rel_path(""));
    assert!(!is_safe_rel_path("/etc/passwd"));
    assert!(!is_safe_rel_path("../.zshrc"));
    assert!(!is_safe_rel_path("a/../../b"));
    assert!(!is_safe_rel_path("a/./b"));
    assert!(!is_safe_rel_path("a//b"));
    assert!(!is_safe_rel_path("a/"));
    assert!(!is_safe_rel_path("a\0b"));
}

fn document(path: &str, id_of: &str) -> serde_json::Value {
    serde_json::json!({
        "name": format!(
            "projects/p/databases/(default)/documents/users/u/files2/{}",
            doc_id_from_path(id_of)
        ),
        "fields": {
            "path":    { "stringValue": path },
            "sha256":  { "stringValue": "ab" },
            "size":    { "integerValue": "12" },
            "mtime":   { "integerValue": "34" },
            "mime":    { "stringValue": "text/plain" },
            "device":  { "stringValue": "pc" },
            "deleted": { "booleanValue": false },
            "updated_at": { "timestampValue": "2026-10-02T09:15:04.123Z" },
        },
        "createTime": "2026-10-01T00:00:00.000001Z",
        "updateTime": "2026-10-02T09:15:04.130001Z",
    })
}

#[test]
fn a_document_carries_its_server_stamp_and_version() {
    let doc = from_document(&document("docs/a b.txt", "docs/a b.txt")).unwrap();
    assert_eq!(doc.path, "docs/a b.txt");
    assert_eq!((doc.size, doc.mtime, doc.deleted), (12, 34, false));
    assert_eq!(doc.updated_at, Some(1_790_932_504_123_000));
    assert_eq!(doc.version.as_deref(), Some("2026-10-02T09:15:04.130001Z"));
    assert_eq!(doc.stamp(), doc.updated_at);

    // Without the stamp field the update time stands in for it.
    let mut raw = document("a.txt", "a.txt");
    raw["fields"].as_object_mut().unwrap().remove("updated_at");
    let doc = from_document(&raw).unwrap();
    assert_eq!(doc.updated_at, None);
    assert_eq!(doc.stamp(), Some(1_790_932_504_130_001));
}

#[test]
fn a_document_filed_under_the_wrong_id_or_path_is_ignored() {
    assert!(from_document(&document("a.txt", "b.txt")).is_none());
    assert!(from_document(&document("../a.txt", "../a.txt")).is_none());
    assert!(from_document(&serde_json::json!({ "name": "x" })).is_none());
    // Ids never need escaping, whatever the path holds.
    let id = doc_id_from_path("dir/ü %2F?#.txt");
    assert!(super::super::hash::is_sha256_hex(&id));
    assert_ne!(id, doc_id_from_path("dir/ü /?#.txt"));
}

#[test]
fn a_query_reply_gives_docs_and_the_servers_time() {
    // Results: each entry has its own read time; the earliest counts.
    let reply = serde_json::json!([
        { "document": document("a.txt", "a.txt"), "readTime": "2026-10-02T09:20:00.000002Z" },
        { "document": document("b.txt", "b.txt"), "readTime": "2026-10-02T09:20:00.000001Z" },
        { "document": document("bad", "other"), "readTime": "2026-10-02T09:20:00.000003Z" },
    ]);
    let pull = parse_query(&reply);
    assert_eq!(pull.docs.len(), 2);
    assert_eq!(pull.read_at, stamp::parse("2026-10-02T09:20:00.000001Z"));

    // Nothing matched: one entry with just the time.
    let pull = parse_query(&serde_json::json!([{ "readTime": "2026-10-02T09:20:00Z" }]));
    assert!(pull.docs.is_empty());
    assert_eq!(pull.read_at, stamp::parse("2026-10-02T09:20:00Z"));

    // A reply sync cannot read is no docs and no time, not a crash.
    assert_eq!(parse_query(&serde_json::json!({ "error": 1 })), Pull::default());
}

#[test]
fn a_write_result_completes_the_doc() {
    let sent = FileDoc {
        path: "a.txt".into(),
        sha256: "ab".into(),
        size: 1,
        mtime: 2,
        mime: String::new(),
        device: "pc".into(),
        deleted: false,
        updated_at: None,
        version: None,
    };
    let result = serde_json::json!({
        "updateTime": "2026-10-02T09:15:04.130001Z",
        "transformResults": [{ "timestampValue": "2026-10-02T09:15:04.123Z" }],
    });
    let doc = stored(&sent, &result);
    assert_eq!(doc.updated_at, Some(1_790_932_504_123_000));
    assert_eq!(doc.version.as_deref(), Some("2026-10-02T09:15:04.130001Z"));
    // A reply without them leaves both unknown; the next pull fills
    // them in.
    let doc = stored(&sent, &serde_json::Value::Null);
    assert_eq!((doc.updated_at, doc.version), (None, None));
    // The write itself never sends a stamp.
    assert!(to_fields(&sent).get("updated_at").is_none());
}

#[test]
fn only_precondition_refusals_count_as_moved() {
    assert!(is_precondition_refusal(400, "FAILED_PRECONDITION"));
    assert!(is_precondition_refusal(409, "ALREADY_EXISTS"));
    assert!(is_precondition_refusal(404, "NOT_FOUND"));
    assert!(is_precondition_refusal(409, "ABORTED"));
    assert!(is_precondition_refusal(412, ""));
    // A malformed request, a rules refusal, quota, the server: errors.
    assert!(!is_precondition_refusal(400, "INVALID_ARGUMENT"));
    assert!(!is_precondition_refusal(400, ""));
    assert!(!is_precondition_refusal(403, "PERMISSION_DENIED"));
    assert!(!is_precondition_refusal(429, "RESOURCE_EXHAUSTED"));
    assert!(!is_precondition_refusal(503, "UNAVAILABLE"));
}

#[test]
fn a_blob_counts_as_used_by_any_document_and_unused_only_on_a_plain_no() {
    let read_time = "2026-10-02T09:15:04.130001Z";
    // A document sync itself would not accept (no fields at all) still
    // names the blob.
    let odd = serde_json::json!([{ "document": { "name": "x/y/z" }, "readTime": read_time }]);
    assert!(in_use_reply(&odd).unwrap());
    let none = serde_json::json!([{ "readTime": read_time }]);
    assert!(!in_use_reply(&none).unwrap());
    // Anything that is not a plain "no": the blob is kept.
    for reply in [
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!([{ "error": { "code": 500 } }]),
        serde_json::json!([{ "readTime": read_time }, { "error": { "code": 500 } }]),
    ] {
        assert!(in_use_reply(&reply).is_err(), "{reply}");
    }
}

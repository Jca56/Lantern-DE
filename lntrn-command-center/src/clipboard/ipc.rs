//! Synchronous client for the compositor's clipboard IPC socket.
//!
//! Reconnects on each request to keep the protocol stateless from our
//! side. Calls block for at most [`TIMEOUT_MS`]; on timeout / failure
//! the caller decides whether to show stale state.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use super::Entry;

const TIMEOUT_MS: u64 = 250;

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-clipboard.sock", uid))
}

fn connect() -> Option<UnixStream> {
    let stream = UnixStream::connect(socket_path()).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(TIMEOUT_MS)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(TIMEOUT_MS)))
        .ok();
    Some(stream)
}

fn round_trip(req_json: &str) -> Option<String> {
    let stream = connect()?;
    let mut writer = stream.try_clone().ok()?;
    let mut reader = BufReader::new(stream);
    writeln!(writer, "{}", req_json).ok()?;
    writer.flush().ok()?;
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    Some(line)
}

#[derive(Deserialize)]
struct EntryDto {
    id: u64,
    kind: String,
    preview: Option<String>,
    timestamp_ms: u128,
    pinned: bool,
    image_path: Option<String>,
}

#[derive(Deserialize)]
struct ListResponse {
    entries: Vec<EntryDto>,
}

/// `{"op":"list"}` → entries (newest first). Returns empty vec on
/// transport failure rather than blocking the caller.
pub fn list() -> Vec<Entry> {
    let Some(line) = round_trip(r#"{"op":"list"}"#) else {
        return Vec::new();
    };
    let parsed: serde_json::Result<ListResponse> = serde_json::from_str(&line);
    let Ok(resp) = parsed else { return Vec::new() };
    resp.entries
        .into_iter()
        .map(|e| Entry {
            id: e.id,
            kind: e.kind,
            preview: e.preview,
            timestamp_ms: e.timestamp_ms,
            pinned: e.pinned,
            image_path: e.image_path,
        })
        .collect()
}

/// Re-publish `id` as the active selection.
pub fn copy(id: u64) -> bool {
    let req = format!(r#"{{"op":"copy","id":{}}}"#, id);
    expect_ok(&req)
}

pub fn delete(id: u64) -> bool {
    let req = format!(r#"{{"op":"delete","id":{}}}"#, id);
    expect_ok(&req)
}

pub fn clear() -> bool {
    expect_ok(r#"{"op":"clear"}"#)
}

pub fn pin(id: u64, value: bool) -> bool {
    let req = format!(r#"{{"op":"pin","id":{},"value":{}}}"#, id, value);
    expect_ok(&req)
}

fn expect_ok(req: &str) -> bool {
    let Some(line) = round_trip(req) else {
        return false;
    };
    line.contains("\"ok\":true")
}

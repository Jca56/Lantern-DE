//! The compositor's clipboard history, as the launcher's clipboard page
//! shows it: the entries (`lntrn-clipboard.sock`, newline-delimited JSON,
//! see lntrn-compositor's `clipboard_ipc.rs`), and copying one back,
//! pinning, deleting and clearing. Each ask runs on a thread of its own
//! with a short timeout and wakes the loop with what it read, so a stuck
//! compositor never stalls a frame.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_data::{Doc, json};

/// How long one ask may take before it gives up.
const TIMEOUT: Duration = Duration::from_millis(800);

/// One thing copied.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEntry {
    pub id: u64,
    /// The start of the text; empty for a picture.
    pub text: String,
    /// The picture's file, for a picture.
    pub image: Option<PathBuf>,
    /// When it was copied, ms since the epoch.
    pub at_ms: u64,
    /// Kept however many more are copied.
    pub pinned: bool,
}

/// What can be done to the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipOp {
    /// Put this entry back on the clipboard.
    Copy(u64),
    Pin(u64, bool),
    Delete(u64),
    Clear,
}

/// What the last ask left.
#[derive(Default)]
struct Shared {
    /// The entries, pinned first, the newest first; `None` until read.
    entries: Option<Vec<ClipEntry>>,
    /// Why the last ask failed.
    failed: Option<String>,
}

#[derive(Default)]
pub struct ClipHistory {
    shared: Arc<Mutex<Shared>>,
    waker: Arc<OnceLock<Waker>>,
    /// Never asks the compositor (the tests').
    offline: bool,
}

impl ClipHistory {
    /// A history that never asks the compositor, holding `entries`.
    #[cfg(test)]
    pub fn holding(entries: Vec<ClipEntry>) -> ClipHistory {
        let h = ClipHistory { offline: true, ..ClipHistory::default() };
        h.shared.lock().unwrap().entries = Some(entries);
        h
    }

    pub fn set_waker(&self, waker: Waker) {
        let _ = self.waker.set(waker);
    }

    /// Read the entries afresh.
    pub fn refresh(&self) {
        self.ask(None);
    }

    /// Do `op`, then read the entries afresh.
    pub fn run(&self, op: ClipOp) {
        self.ask(Some(op));
    }

    /// The entries as last read; `None` until the first read is in.
    pub fn entries(&self) -> Option<Vec<ClipEntry>> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).entries.clone()
    }

    /// Why the last ask failed, when it did.
    pub fn failed(&self) -> Option<String> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).failed.clone()
    }

    fn ask(&self, op: Option<ClipOp>) {
        if self.offline {
            return;
        }
        let (shared, waker) = (self.shared.clone(), self.waker.clone());
        let spawned = std::thread::Builder::new().name("clipboard".into()).spawn(move || {
            let read = op.map_or(Ok(()), |op| request(&line_for(op)).map(drop)).and_then(|()| request("{\"op\":\"list\"}"));
            let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
            match read.map(|doc| entries_of(&doc)) {
                Ok(entries) => (s.entries, s.failed) = (Some(entries), None),
                Err(e) => {
                    log_warn!("clipboard: {e}");
                    s.failed = Some(e);
                }
            }
            drop(s);
            if let Some(w) = waker.get() {
                w.wake();
            }
        });
        if let Err(e) = spawned {
            log_warn!("clipboard: no thread to ask on: {e}");
        }
    }
}

/// The request line for `op`.
fn line_for(op: ClipOp) -> String {
    match op {
        ClipOp::Copy(id) => format!("{{\"op\":\"copy\",\"id\":{id}}}"),
        ClipOp::Pin(id, on) => format!("{{\"op\":\"pin\",\"id\":{id},\"value\":{on}}}"),
        ClipOp::Delete(id) => format!("{{\"op\":\"delete\",\"id\":{id}}}"),
        ClipOp::Clear => "{\"op\":\"clear\"}".to_owned(),
    }
}

/// Send one request and read its one answer; an `{"error": …}` answer is
/// the error.
fn request(line: &str) -> Result<Doc, String> {
    let path = crate::compositor::compositor_socket("lntrn-clipboard.sock");
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    stream.set_read_timeout(Some(TIMEOUT)).and_then(|()| stream.set_write_timeout(Some(TIMEOUT))).map_err(|e| e.to_string())?;
    stream.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).map_err(|e| e.to_string())?;
    let doc = json::parse(&answer).map_err(|e| format!("an answer that won't read: {e}"))?;
    match doc.get("error").and_then(Doc::as_str) {
        Some(why) => Err(why.to_owned()),
        None => Ok(doc),
    }
}

/// The entries a `list` answer holds, pinned first and otherwise as the
/// compositor keeps them (the newest first). One that won't read is left
/// out.
fn entries_of(doc: &Doc) -> Vec<ClipEntry> {
    let list = doc.get("entries").and_then(Doc::as_list).unwrap_or_default();
    let mut entries: Vec<ClipEntry> = list
        .iter()
        .filter_map(|e| {
            let id = e.get("id").and_then(Doc::as_i64).and_then(|id| u64::try_from(id).ok())?;
            let image = (e.get("kind").and_then(Doc::as_str) == Some("image")).then(|| e.get("image_path").and_then(Doc::as_str).map(PathBuf::from)).flatten();
            let text = if image.is_some() { String::new() } else { e.get("preview").and_then(Doc::as_str).unwrap_or_default().to_owned() };
            let at_ms = e.get("timestamp_ms").and_then(Doc::as_i64).map_or(0, |t| t.max(0) as u64);
            let pinned = e.get("pinned").and_then(Doc::as_bool).unwrap_or(false);
            Some(ClipEntry { id, text, image, at_ms, pinned })
        })
        .collect();
    entries.sort_by_key(|e| !e.pinned);
    entries
}

/// How long ago `at_ms` was at `now_ms`, shortly: "now", "5 min", "3 h",
/// "2 d".
pub fn age(at_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(at_ms) / 1000;
    match s {
        0..60 => "now".to_owned(),
        60..3600 => format!("{} min", s / 60),
        3600..86_400 => format!("{} h", s / 3600),
        _ => format!("{} d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_answer_reads_pinned_first_and_pictures_by_their_file() {
        let doc = json::parse(
            r#"{"entries":[
                {"id":9,"kind":"text","preview":"newest","timestamp_ms":3000,"pinned":false,"image_path":null},
                {"id":4,"kind":"image","preview":null,"timestamp_ms":2000,"pinned":false,"image_path":"/run/user/1000/lntrn-clipboard/4.png"},
                {"id":2,"kind":"text","preview":"kept","timestamp_ms":1000,"pinned":true,"image_path":null},
                {"kind":"text","preview":"no id"}
            ]}"#,
        )
        .unwrap();
        let got = entries_of(&doc);
        assert_eq!(got.iter().map(|e| e.id).collect::<Vec<_>>(), [2, 9, 4], "pinned first, then as kept; one without an id left out");
        assert_eq!(got[1].text, "newest");
        assert_eq!(got[2].image.as_deref(), Some(std::path::Path::new("/run/user/1000/lntrn-clipboard/4.png")));
        assert!(got[2].text.is_empty());
        assert!(entries_of(&json::parse("{}").unwrap()).is_empty());
    }

    #[test]
    fn each_op_asks_as_the_compositor_reads_it() {
        assert_eq!(line_for(ClipOp::Copy(5)), r#"{"op":"copy","id":5}"#);
        assert_eq!(line_for(ClipOp::Pin(5, true)), r#"{"op":"pin","id":5,"value":true}"#);
        assert_eq!(line_for(ClipOp::Delete(7)), r#"{"op":"delete","id":7}"#);
        assert_eq!(line_for(ClipOp::Clear), r#"{"op":"clear"}"#);
    }

    #[test]
    fn ages_read_shortly() {
        assert_eq!(age(10_000, 30_000), "now");
        assert_eq!(age(0, 5 * 60_000), "5 min");
        assert_eq!(age(0, 3 * 3_600_000), "3 h");
        assert_eq!(age(0, 2 * 86_400_000), "2 d");
        assert_eq!(age(5_000, 1_000), "now", "a clock that went back");
    }
}

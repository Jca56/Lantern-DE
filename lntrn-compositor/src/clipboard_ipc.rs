//! Unix-socket IPC for CC ↔ compositor clipboard history.
//!
//! Socket: `/run/user/{uid}/lntrn-clipboard.sock` — peer-verified via
//! `SO_PEERCRED`. Newline-delimited JSON.
//!
//!   # Requests (CC → compositor)
//!   {"op":"list"}
//!   {"op":"copy","id":<u64>}
//!   {"op":"delete","id":<u64>}
//!   {"op":"clear"}
//!   {"op":"pin","id":<u64>,"value":<bool>}
//!
//!   # Responses (compositor → CC) — one JSON object per request
//!   list   → {"entries":[{...},{...}]}
//!   action → {"ok":true} or {"error":"<reason>"}
//!
//! Entry shape:
//!   {"id":N,"kind":"text"|"image","preview":"...","timestamp_ms":N,
//!    "pinned":bool,"image_path":"/run/user/UID/lntrn-clipboard/N.png"|null}

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::Lantern;

const MAX_PREVIEW_BYTES: usize = 200;

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut ucred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (ret == 0).then_some(ucred.uid)
}

fn our_uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn socket_path() -> PathBuf {
    PathBuf::from(format!("/run/user/{}/lntrn-clipboard.sock", our_uid()))
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    List,
    Copy { id: u64 },
    Delete { id: u64 },
    Clear,
    Pin { id: u64, value: bool },
}

#[derive(Serialize)]
struct EntryDto {
    id: u64,
    kind: &'static str, // "text" | "image"
    preview: Option<String>,
    timestamp_ms: u128,
    pinned: bool,
    image_path: Option<String>,
}

#[derive(Serialize)]
struct ListResponse {
    entries: Vec<EntryDto>,
}

pub struct ClipboardIpc {
    listener: Option<UnixListener>,
    clients: Vec<ClientConn>,
    next_client_id: u64,
    /// Clients accepted since the last `take_new_clients` (see `ipc_source`).
    new_clients: Vec<(u64, OwnedFd)>,
}

struct ClientConn {
    id: u64,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl ClipboardIpc {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(?e, "failed to chmod 0600 on clipboard IPC socket");
                }
                tracing::info!(?path, "clipboard IPC socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind clipboard IPC socket");
                None
            }
        };
        Self {
            listener,
            clients: Vec::new(),
            next_client_id: 1,
            new_clients: Vec::new(),
        }
    }

    // ── Event-loop hooks (see `ipc_source`) ──────────────────────────────

    pub fn listener_fd(&self) -> Option<OwnedFd> {
        self.listener.as_ref().and_then(crate::ipc_source::dup_fd)
    }

    pub fn take_new_clients(&mut self) -> Vec<(u64, OwnedFd)> {
        std::mem::take(&mut self.new_clients)
    }

    pub fn has_client(&self, id: u64) -> bool {
        self.clients.iter().any(|c| c.id == id)
    }
}

/// Drain pending IPC traffic — accept new clients, read requests, dispatch
/// against the live `ClipboardManager`, write responses. Driven by the
/// event-loop sources registered in `ipc_source` (listener + one per client).
pub fn poll(state: &mut Lantern) {
    // Accept new connections first.
    if let Some(listener) = &state.clipboard_ipc.listener {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    match peer_uid(&stream) {
                        Some(uid) if uid == our_uid() => {}
                        other => {
                            tracing::warn!(
                                ?other,
                                "rejecting clipboard IPC connection from foreign uid"
                            );
                            continue;
                        }
                    }
                    stream.set_nonblocking(true).ok();
                    let writer = match stream.try_clone() {
                        Ok(w) => w,
                        Err(_) => continue,
                    };
                    let id = state.clipboard_ipc.next_client_id;
                    state.clipboard_ipc.next_client_id += 1;
                    if let Some(fd) = crate::ipc_source::dup_fd(&stream) {
                        state.clipboard_ipc.new_clients.push((id, fd));
                    }
                    state.clipboard_ipc.clients.push(ClientConn {
                        id,
                        reader: BufReader::new(stream),
                        writer,
                    });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    // Drain reads.
    let mut to_drop: Vec<usize> = Vec::new();
    for i in 0..state.clipboard_ipc.clients.len() {
        let lines = read_lines(&mut state.clipboard_ipc.clients[i].reader);
        let dropped = lines.is_none();
        let lines = lines.unwrap_or_default();
        for line in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Request>(trimmed) {
                Ok(req) => handle_request(state, req),
                Err(e) => {
                    tracing::warn!(?e, raw = trimmed, "clipboard IPC bad request");
                    serde_json::json!({"error": "bad_request"}).to_string()
                }
            };
            let client = &mut state.clipboard_ipc.clients[i];
            let _ = writeln!(client.writer, "{}", response);
            let _ = client.writer.flush();
        }
        if dropped {
            to_drop.push(i);
        }
    }
    for idx in to_drop.into_iter().rev() {
        state.clipboard_ipc.clients.remove(idx);
    }
    for (id, fd) in state.clipboard_ipc.take_new_clients() {
        crate::ipc_source::register_client(
            state,
            fd,
            "clipboard",
            id,
            poll,
            |s: &Lantern, id: u64| s.clipboard_ipc.has_client(id),
        );
    }
}

/// Reads all available newline-delimited messages from a non-blocking
/// reader. Returns `None` if the peer has hung up.
fn read_lines(reader: &mut BufReader<UnixStream>) -> Option<Vec<String>> {
    let mut out = Vec::new();
    loop {
        let mut buf = String::new();
        match reader.read_line(&mut buf) {
            Ok(0) => return None,
            Ok(_) => out.push(buf),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Some(out),
            Err(_) => return None,
        }
    }
}

fn handle_request(state: &mut Lantern, req: Request) -> String {
    match req {
        Request::List => {
            let entries = build_entry_list(state);
            serde_json::to_string(&ListResponse { entries }).unwrap_or_else(|_| "{}".to_string())
        }
        Request::Copy { id } => {
            if crate::clipboard_manager::republish(state, id) {
                serde_json::json!({"ok": true}).to_string()
            } else {
                serde_json::json!({"error": "not_found"}).to_string()
            }
        }
        Request::Delete { id } => {
            if state.clipboard_manager.forget(id) {
                serde_json::json!({"ok": true}).to_string()
            } else {
                serde_json::json!({"error": "not_found"}).to_string()
            }
        }
        Request::Clear => {
            state.clipboard_manager.clear();
            serde_json::json!({"ok": true}).to_string()
        }
        Request::Pin { id, value } => match state.clipboard_manager.set_pinned(id, value) {
            Some(_) => serde_json::json!({"ok": true}).to_string(),
            None => serde_json::json!({"error": "not_found"}).to_string(),
        },
    }
}

fn build_entry_list(state: &Lantern) -> Vec<EntryDto> {
    state
        .clipboard_manager
        .history()
        .iter()
        .map(|e| {
            let kind = if e.preview.is_some() { "text" } else { "image" };
            let preview = e.preview.as_ref().map(|s| {
                let mut s = s.clone();
                if s.len() > MAX_PREVIEW_BYTES {
                    // Truncate on a char boundary.
                    let mut end = MAX_PREVIEW_BYTES;
                    while !s.is_char_boundary(end) && end > 0 {
                        end -= 1;
                    }
                    s.truncate(end);
                    s.push('…');
                }
                s
            });
            let timestamp_ms = e
                .timestamp
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let image_path = if e.has_image() {
                Some(
                    crate::clipboard_manager::image_cache_path(e.id)
                        .to_string_lossy()
                        .into_owned(),
                )
            } else {
                None
            };
            EntryDto {
                id: e.id,
                kind,
                preview,
                timestamp_ms,
                pinned: e.pinned,
                image_path,
            }
        })
        .collect()
}

//! Unix-socket IPC for bar ↔ compositor workspace state & commands.
//!
//! Protocol (newline-delimited UTF-8 over `/run/user/{uid}/lntrn-workspaces.sock`):
//!
//!   # Compositor → Bar (on connect + on every state change):
//!   `state:<output>:<active>:<id1>,<id2>,<id3>`
//!   `covered:<output>:<0|1>`      # 1 while a maximized or fullscreen window
//!                                 # shows on the output's active workspace
//!                                 # (a floating bar docks under one)
//!   `win:<id>\t<flags>\t<app_id>\t<title>`  # a window opened or changed;
//!                                 # flags any of m(inimized) a(ctive)
//!                                 # x (maximized) f(ullscreen), or `-`
//!   `win-gone:<id>`               # a window closed
//!
//!   # Bar → Compositor:
//!   `switch:<output>:<target>`    # switch active WS
//!   `move:<output>:<target>`      # move focused window to WS (stay on current)
//!   `cycle:<output>:<direction>`  # direction = -1 or 1, cycles populated WSes
//!   `activate:<id>`               # show and focus a window, wherever it is
//!   `minimize:<id>` / `close:<id>`
//!   `icons:<namespace>\t<app_id> <x> <y> <w> <h>\t…`  # where each app's
//!                                 # icon is on the layer surface called
//!                                 # <namespace> (its logical px): minimizing
//!                                 # flies a window into its app's icon
//!   `thumbs:<namespace>\t<id> <x> <y> <w> <h>[ <cx> <cy> <cw> <ch> <0|1>]\t…`
//!                                 # live thumbnails of windows, drawn over
//!                                 # that surface, each with a red close
//!                                 # button where asked (lit while hovered);
//!                                 # none listed clears them
//!
//! Multiple bars may connect simultaneously (multi-monitor). Window ids and
//! the feed: `window_management/dock_link.rs`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use smithay::utils::{Logical, Rectangle};

/// Read SO_PEERCRED on a stream socket. Returns the peer's uid, or
/// `None` if the lookup fails (treat as untrusted).
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
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-workspaces.sock", uid))
}

pub enum IpcCommand {
    Switch { output: String, target: u32 },
    Move { output: String, target: u32 },
    Cycle { output: String, direction: i32 },
    Activate(u64),
    Minimize(u64),
    Close(u64),
    Icons { namespace: String, icons: Vec<(String, Rectangle<i32, Logical>)> },
    Thumbs { namespace: String, slots: Vec<DockThumb> },
}

/// A window's live thumbnail over a dock's surface, in its logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct DockThumb {
    pub id: u64,
    pub rect: Rectangle<i32, Logical>,
    /// Its close button, and whether the pointer is on it.
    pub close: Option<(Rectangle<i32, Logical>, bool)>,
}

pub struct WorkspaceIpc {
    listener: Option<UnixListener>,
    clients: Vec<ClientConn>,
    next_client_id: u64,
    /// Clients accepted since the last `take_new_clients` — the event loop
    /// registers a readiness source for each (see `ipc_source`).
    new_clients: Vec<(u64, OwnedFd)>,
    /// What clients were last told each output's `covered` is, so only a
    /// change goes out (see `window_management/covered.rs`).
    pub covered: HashMap<String, bool>,
    /// The `win:` line clients were last told for each window id.
    pub windows: HashMap<u64, String>,
    /// Each layer namespace's app icons: app_id → rect on that surface.
    pub icons: HashMap<String, HashMap<String, Rectangle<i32, Logical>>>,
    /// Each layer namespace's thumbnail slots.
    pub thumbs: HashMap<String, Vec<DockThumb>>,
}

struct ClientConn {
    id: u64,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// Needs initial state after connect.
    needs_initial: bool,
}

impl WorkspaceIpc {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(?e, "failed to chmod 0600 on workspaces IPC socket");
                }
                tracing::info!(?path, "workspaces IPC socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind workspaces IPC socket");
                None
            }
        };
        Self {
            listener,
            clients: Vec::new(),
            next_client_id: 1,
            new_clients: Vec::new(),
            covered: HashMap::new(),
            windows: HashMap::new(),
            icons: HashMap::new(),
            thumbs: HashMap::new(),
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

    /// Accept new connections and read pending messages. Returns decoded commands.
    /// Also returns true in the second slot if a new client needs the initial state.
    pub fn poll(&mut self) -> (Vec<IpcCommand>, bool) {
        let mut commands = Vec::new();
        let mut new_client_pending = false;

        // Accept new connections
        if let Some(ref listener) = self.listener {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // Reject any peer whose uid doesn't match ours.
                        // $XDG_RUNTIME_DIR is mode 0700 in normal sessions, but
                        // belt-and-suspenders: we never want a different local
                        // user (or sandboxed app under a different uid) issuing
                        // workspace commands.
                        match peer_uid(&stream) {
                            Some(uid) if uid == our_uid() => {}
                            other => {
                                tracing::warn!(
                                    ?other,
                                    "rejecting workspace IPC connection from foreign uid"
                                );
                                continue;
                            }
                        }
                        stream.set_nonblocking(true).ok();
                        let writer = match stream.try_clone() {
                            Ok(w) => w,
                            Err(_) => continue,
                        };
                        let id = self.next_client_id;
                        self.next_client_id += 1;
                        if let Some(fd) = crate::ipc_source::dup_fd(&stream) {
                            self.new_clients.push((id, fd));
                        }
                        self.clients.push(ClientConn {
                            id,
                            reader: BufReader::new(stream),
                            writer,
                            needs_initial: true,
                        });
                        new_client_pending = true;
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!(?e, "workspaces IPC accept error");
                        break;
                    }
                }
            }
        }

        // Read pending messages from each client
        let mut drop_indexes = Vec::new();
        for (i, client) in self.clients.iter_mut().enumerate() {
            let mut line = String::new();
            loop {
                line.clear();
                match client.reader.read_line(&mut line) {
                    Ok(0) => {
                        drop_indexes.push(i);
                        break;
                    }
                    Ok(_) => {
                        if let Some(cmd) = parse_command(line.trim()) {
                            commands.push(cmd);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        drop_indexes.push(i);
                        break;
                    }
                }
            }
        }
        for i in drop_indexes.into_iter().rev() {
            self.clients.swap_remove(i);
        }

        (commands, new_client_pending)
    }

    pub fn has_clients(&self) -> bool {
        !self.clients.is_empty()
    }

    pub fn has_pending_initial(&self) -> bool {
        self.clients.iter().any(|c| c.needs_initial)
    }

    /// Send a pre-formatted state line to all connected clients.
    pub fn broadcast_line(&mut self, line: &str) {
        let payload = if line.ends_with('\n') {
            line.to_string()
        } else {
            format!("{}\n", line)
        };
        let mut drop_indexes = Vec::new();
        for (i, client) in self.clients.iter_mut().enumerate() {
            if client.writer.write_all(payload.as_bytes()).is_err() {
                drop_indexes.push(i);
                continue;
            }
            client.needs_initial = false;
        }
        for i in drop_indexes.into_iter().rev() {
            self.clients.swap_remove(i);
        }
    }

    /// Mark all pending-initial clients as satisfied. Call after broadcast_line
    /// has sent a full snapshot.
    pub fn mark_initial_delivered(&mut self) {
        for c in &mut self.clients {
            c.needs_initial = false;
        }
    }
}

fn parse_command(msg: &str) -> Option<IpcCommand> {
    if let Some(cmd) = parse_window_command(msg) {
        return Some(cmd);
    }
    let parts: Vec<&str> = msg.splitn(3, ':').collect();
    if parts.len() != 3 {
        return None;
    }
    match parts[0] {
        "switch" => {
            let target: u32 = parts[2].parse().ok()?;
            Some(IpcCommand::Switch {
                output: parts[1].to_string(),
                target,
            })
        }
        "move" => {
            let target: u32 = parts[2].parse().ok()?;
            Some(IpcCommand::Move {
                output: parts[1].to_string(),
                target,
            })
        }
        "cycle" => {
            let direction: i32 = parts[2].parse().ok()?;
            Some(IpcCommand::Cycle {
                output: parts[1].to_string(),
                direction,
            })
        }
        _ => None,
    }
}

/// The window commands: by id, or a namespace's icons or thumbnails.
fn parse_window_command(msg: &str) -> Option<IpcCommand> {
    let (verb, rest) = msg.split_once(':')?;
    let id = || rest.trim().parse::<u64>().ok();
    match verb {
        "activate" => id().map(IpcCommand::Activate),
        "minimize" => id().map(IpcCommand::Minimize),
        "close" => id().map(IpcCommand::Close),
        "icons" => {
            let (namespace, records) = records(rest)?;
            let icons = records.into_iter().map(|(name, rect)| (name.to_string(), rect)).collect();
            Some(IpcCommand::Icons { namespace, icons })
        }
        "thumbs" => {
            let mut fields = rest.split('\t');
            let namespace = fields.next().filter(|n| !n.is_empty())?.to_string();
            let slots = fields.filter_map(dock_thumb).collect();
            Some(IpcCommand::Thumbs { namespace, slots })
        }
        _ => None,
    }
}

/// `<namespace>\t<key> <x> <y> <w> <h>\t…`: the namespace, and each
/// well-formed record's key and rect.
fn records(rest: &str) -> Option<(String, Vec<(&str, Rectangle<i32, Logical>)>)> {
    let mut fields = rest.split('\t');
    let namespace = fields.next().filter(|n| !n.is_empty())?.to_string();
    let parsed = fields
        .filter_map(|record| {
            let mut p = record.split_whitespace();
            let key = p.next()?;
            let mut n = || p.next()?.parse::<i32>().ok();
            let (x, y, w, h) = (n()?, n()?, n()?, n()?);
            Some((key, Rectangle::new((x, y).into(), (w.max(1), h.max(1)).into())))
        })
        .collect();
    Some((namespace, parsed))
}

/// `<id> <x> <y> <w> <h>`, then optionally its close button's
/// `<cx> <cy> <cw> <ch> <0|1>`.
fn dock_thumb(record: &str) -> Option<DockThumb> {
    let n: Vec<&str> = record.split_whitespace().collect();
    let int = |i: usize| n.get(i)?.parse::<i32>().ok();
    let rect = |at: usize| Some(Rectangle::new((int(at)?, int(at + 1)?).into(), (int(at + 2)?.max(1), int(at + 3)?.max(1)).into()));
    let close = (n.len() >= 10).then(|| Some((rect(5)?, n[9] == "1"))).flatten();
    Some(DockThumb { id: n.first()?.parse().ok()?, rect: rect(1)?, close })
}

/// A window's feed line: `win:<id>\t<flags>\t<app_id>\t<title>`. Tabs and
/// line breaks in the names would break the line, so they go as spaces.
pub fn format_window_line(id: u64, states: &[u32], app_id: &str, title: &str) -> String {
    // zwlr_foreign_toplevel_handle_v1's state numbers.
    let letters: String = states
        .iter()
        .filter_map(|s| match s {
            0 => Some('x'),
            1 => Some('m'),
            2 => Some('a'),
            3 => Some('f'),
            _ => None,
        })
        .collect();
    let flags = if letters.is_empty() { "-".to_string() } else { letters };
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    format!("win:{id}\t{flags}\t{}\t{}", clean(app_id), clean(title))
}

/// Format a cover line for broadcast: `covered:<output>:<0|1>`.
pub fn format_covered_line(output: &str, covered: bool) -> String {
    format!("covered:{output}:{}", u8::from(covered))
}

/// Format a state line for broadcast: `state:<output>:<active>:<id1>,<id2>,...`.
pub fn format_state_line(output: &str, active: u32, ids: &[u32]) -> String {
    let mut line = String::with_capacity(64);
    line.push_str("state:");
    line.push_str(output);
    line.push(':');
    line.push_str(&active.to_string());
    line.push(':');
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        line.push_str(&id.to_string());
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_lines_say_the_state_and_keep_to_one_line() {
        assert_eq!(format_window_line(7, &[2, 0], "firefox", "GitHub"), "win:7\tax\tfirefox\tGitHub");
        assert_eq!(format_window_line(3, &[], "foot", "a\tb\nc"), "win:3\t-\tfoot\ta b c");
    }

    #[test]
    fn window_commands_parse_and_bad_records_are_skipped() {
        assert!(matches!(parse_command("activate:12"), Some(IpcCommand::Activate(12))));
        assert!(matches!(parse_command("close: 4"), Some(IpcCommand::Close(4))));
        assert!(parse_command("minimize:x").is_none());
        let Some(IpcCommand::Icons { namespace, icons }) = parse_command("icons:lntrn-control-center\tfirefox 10 20 56 56\tbroken 1 2\tfoot 70 20 56 56") else {
            panic!("icons");
        };
        assert_eq!(namespace, "lntrn-control-center");
        assert_eq!(icons.iter().map(|(a, r)| (a.as_str(), r.loc.x)).collect::<Vec<_>>(), vec![("firefox", 10), ("foot", 70)]);
        let Some(IpcCommand::Thumbs { slots, .. }) = parse_command("thumbs:preview") else { panic!("thumbs") };
        assert!(slots.is_empty(), "none listed clears them");
        let Some(IpcCommand::Thumbs { slots, .. }) = parse_command("thumbs:preview\t7 10 20 400 213 374 26 30 30 1\t8 420 20 400 213\tjunk") else { panic!("thumbs") };
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].close, Some((Rectangle::new((374, 26).into(), (30, 30).into()), true)));
        assert_eq!((slots[1].id, slots[1].close), (8, None));
        // The old commands still read.
        assert!(matches!(parse_command("switch:DP-1:3"), Some(IpcCommand::Switch { target: 3, .. })));
    }
}

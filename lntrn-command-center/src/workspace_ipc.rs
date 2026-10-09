//! Workspace IPC client — connects to the compositor at
//! `/run/user/{uid}/lntrn-workspaces.sock` and tracks the active
//! workspace id per output.
//!
//! The compositor pushes lines of the form
//! `state:<output>:<active>:<id1>,<id2>,...` on every change; we send
//! `cycle:<output>:<direction>` when the workspace tile is clicked.
//! See `lntrn-compositor/src/workspace_ipc.rs` for the protocol.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-workspaces.sock", uid))
}

#[derive(Clone, Debug, Default)]
struct OutputState {
    active: u32,
    /// Populated workspace ids, ascending — the set `cycle` walks.
    ids: Vec<u32>,
}

pub struct WorkspaceIpc {
    reader: Option<BufReader<UnixStream>>,
    writer: Option<UnixStream>,
    state: HashMap<String, OutputState>,
    retry_deadline: Instant,
}

impl WorkspaceIpc {
    pub fn new() -> Self {
        Self {
            reader: None,
            writer: None,
            state: HashMap::new(),
            retry_deadline: Instant::now(),
        }
    }

    fn try_connect(&mut self) {
        let now = Instant::now();
        if now < self.retry_deadline {
            return;
        }
        self.retry_deadline = now + Duration::from_secs(2);

        let path = socket_path();
        match UnixStream::connect(&path) {
            Ok(stream) => {
                stream.set_nonblocking(true).ok();
                self.writer = stream.try_clone().ok();
                self.reader = Some(BufReader::new(stream));
                tracing::info!(?path, "connected to workspaces IPC");
            }
            Err(e) => {
                tracing::debug!(?e, "workspaces IPC not ready, will retry");
            }
        }
    }

    /// Drain pending state lines. Cheap to call every frame.
    pub fn poll(&mut self) {
        if self.reader.is_none() {
            self.try_connect();
        }
        let Some(reader) = &mut self.reader else {
            return;
        };

        let mut line = String::new();
        let mut disconnect = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    disconnect = true;
                    break;
                }
                Ok(_) => {
                    parse_state_line(line.trim(), &mut self.state);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    disconnect = true;
                    break;
                }
            }
        }
        if disconnect {
            self.disconnect();
        }
    }

    fn disconnect(&mut self) {
        self.reader = None;
        self.writer = None;
    }

    /// Active workspace id for a specific output (by connector name, e.g.
    /// "DP-1"). On a multi-monitor setup each output has its own active
    /// workspace, so the caller must pass the output our surface is on —
    /// otherwise we'd return an arbitrary monitor's workspace (HashMap
    /// iteration order), which is why the tile used to get stuck on "1".
    ///
    /// `output` is `None` before we know which monitor we're on; in that
    /// case we only answer when there's a single output (unambiguous),
    /// returning `None` rather than guessing wrong on multi-monitor.
    pub fn active_id_for(&self, output: Option<&str>) -> Option<u32> {
        let name = self.resolve_output(output)?;
        self.state.get(name).map(|s| s.active)
    }

    /// Whether `output` has more than one populated workspace, i.e. a
    /// `send_cycle` would actually go somewhere.
    pub fn can_cycle(&self, output: Option<&str>) -> bool {
        self.resolve_output(output)
            .and_then(|name| self.state.get(name))
            .is_some_and(|s| s.ids.len() > 1)
    }

    /// Ask the compositor to step `output` to its next (`1`) or previous
    /// (`-1`) populated workspace, wrapping at the ends. `output` resolves
    /// like `active_id_for`. The local `active` moves optimistically so the
    /// tile reacts on this frame; the compositor pushes the canonical state
    /// right back. Best-effort: if the compositor is gone, drop the
    /// connection and let poll() retry.
    pub fn send_cycle(&mut self, output: Option<&str>, direction: i32) {
        let Some(name) = self.resolve_output(output).map(str::to_owned) else {
            return;
        };
        let msg = format!("cycle:{name}:{direction}\n");
        let ok = self
            .writer
            .as_mut()
            .map(|w| w.write_all(msg.as_bytes()).is_ok())
            .unwrap_or(false);
        if !ok {
            self.disconnect();
            return;
        }
        if let Some(s) = self.state.get_mut(&name) {
            s.active = neighbor_id(&s.ids, s.active, direction);
        }
    }

    /// The state key for `output`: its own entry when we have one, else the
    /// only output we know about. `None` when it would be a guess.
    fn resolve_output<'a>(&'a self, output: Option<&'a str>) -> Option<&'a str> {
        if let Some(name) = output {
            if self.state.contains_key(name) {
                return Some(name);
            }
        }
        if self.state.len() == 1 {
            return self.state.keys().next().map(String::as_str);
        }
        None
    }
}

/// The compositor's `neighbor_id`: step through the ascending ids from
/// `active`, wrapping. Stays put when there's nowhere else to go.
fn neighbor_id(ids: &[u32], active: u32, direction: i32) -> u32 {
    if ids.len() <= 1 {
        return active;
    }
    let idx = ids.iter().position(|id| *id == active).unwrap_or(0);
    ids[(idx as i32 + direction).rem_euclid(ids.len() as i32) as usize]
}

fn parse_state_line(msg: &str, state: &mut HashMap<String, OutputState>) {
    let parts: Vec<&str> = msg.splitn(4, ':').collect();
    if parts.len() != 4 || parts[0] != "state" {
        return;
    }
    let output = parts[1].to_string();
    let active: u32 = match parts[2].parse() {
        Ok(v) => v,
        Err(_) => return,
    };
    let ids = parts[3].split(',').filter_map(|id| id.parse().ok()).collect();
    state.insert(output, OutputState { active, ids });
}

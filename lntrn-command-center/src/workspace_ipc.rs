//! Workspace IPC client — connects to the compositor at
//! `/run/user/{uid}/lntrn-workspaces.sock` and tracks the active
//! workspace id per output.
//!
//! The compositor pushes lines of the form
//! `state:<output>:<active>:<id1>,<id2>,...` on every change.
//! See `lntrn-compositor/src/workspace_ipc.rs` for the protocol.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind};
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
}

pub struct WorkspaceIpc {
    reader: Option<BufReader<UnixStream>>,
    state: HashMap<String, OutputState>,
    retry_deadline: Instant,
}

impl WorkspaceIpc {
    pub fn new() -> Self {
        Self {
            reader: None,
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
        let Some(reader) = &mut self.reader else { return };

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
            self.reader = None;
        }
    }

    /// Active workspace id for the first output we've seen state from,
    /// or `None` if we haven't received anything yet.
    pub fn active_id(&self) -> Option<u32> {
        self.state.values().next().map(|s| s.active)
    }
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
    state.insert(output, OutputState { active });
}

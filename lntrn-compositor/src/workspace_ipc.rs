//! Unix-socket IPC for bar ↔ compositor workspace state & commands.
//!
//! Protocol (newline-delimited UTF-8 over `/run/user/{uid}/lntrn-workspaces.sock`):
//!
//!   # Compositor → Bar (on connect + on every state change):
//!   `state:<output>:<active>:<id1>,<id2>,<id3>`
//!
//!   # Bar → Compositor:
//!   `switch:<output>:<target>`    # switch active WS
//!   `move:<output>:<target>`      # move focused window to WS (stay on current)
//!   `cycle:<output>:<direction>`  # direction = -1 or 1, cycles populated WSes
//!
//! Multiple bars may connect simultaneously (multi-monitor).

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-workspaces.sock", uid))
}

pub enum IpcCommand {
    Switch { output: String, target: u32 },
    Move { output: String, target: u32 },
    Cycle { output: String, direction: i32 },
}

pub struct WorkspaceIpc {
    listener: Option<UnixListener>,
    clients: Vec<ClientConn>,
}

struct ClientConn {
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
                tracing::info!(?path, "workspaces IPC socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind workspaces IPC socket");
                None
            }
        };
        Self { listener, clients: Vec::new() }
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
                        stream.set_nonblocking(true).ok();
                        let writer = match stream.try_clone() {
                            Ok(w) => w,
                            Err(_) => continue,
                        };
                        self.clients.push(ClientConn {
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
                    Ok(0) => { drop_indexes.push(i); break; }
                    Ok(_) => {
                        if let Some(cmd) = parse_command(line.trim()) {
                            commands.push(cmd);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => { drop_indexes.push(i); break; }
                }
            }
        }
        for i in drop_indexes.into_iter().rev() {
            self.clients.swap_remove(i);
        }

        (commands, new_client_pending)
    }

    pub fn has_clients(&self) -> bool { !self.clients.is_empty() }

    pub fn has_pending_initial(&self) -> bool {
        self.clients.iter().any(|c| c.needs_initial)
    }

    /// Send a pre-formatted state line to all connected clients.
    pub fn broadcast_line(&mut self, line: &str) {
        let payload = if line.ends_with('\n') { line.to_string() } else { format!("{}\n", line) };
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
    let parts: Vec<&str> = msg.splitn(3, ':').collect();
    if parts.len() != 3 { return None; }
    match parts[0] {
        "switch" => {
            let target: u32 = parts[2].parse().ok()?;
            Some(IpcCommand::Switch { output: parts[1].to_string(), target })
        }
        "move" => {
            let target: u32 = parts[2].parse().ok()?;
            Some(IpcCommand::Move { output: parts[1].to_string(), target })
        }
        "cycle" => {
            let direction: i32 = parts[2].parse().ok()?;
            Some(IpcCommand::Cycle { output: parts[1].to_string(), direction })
        }
        _ => None,
    }
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
        if i > 0 { line.push(','); }
        line.push_str(&id.to_string());
    }
    line
}

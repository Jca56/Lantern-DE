//! Unix-socket IPC for Command Center ↔ compositor Gaming Mode control.
//!
//! Mirrors `hdr_ipc.rs`: newline-delimited UTF-8 over
//! `/run/user/{uid}/lntrn-gaming.sock`, nonblocking, SO_PEERCRED uid-gated.
//!
//!   # Compositor → client (on connect and on every state change):
//!   `gaming:<0|1>`
//!
//!   # Client → compositor:
//!   `toggle`
//!
//! Gaming Mode state lives in the compositor (`Lantern::gaming_mode`); this
//! socket lets the Command Center tile show it live and flip it on click,
//! staying in lock-step with the Super+G keybind.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

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
    PathBuf::from(format!("/run/user/{}/lntrn-gaming.sock", uid))
}

pub enum GamingCommand {
    Toggle,
}

pub struct GamingIpc {
    listener: Option<UnixListener>,
    clients: Vec<ClientConn>,
    /// Current gaming-mode state, replayed to newly-connecting clients.
    state: bool,
}

struct ClientConn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    needs_state: bool,
}

impl GamingIpc {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(?e, "failed to chmod 0600 on gaming IPC socket");
                }
                tracing::info!(?path, "gaming IPC socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind gaming IPC socket");
                None
            }
        };
        Self {
            listener,
            clients: Vec::new(),
            state: false,
        }
    }

    /// Record the new state and push it to all connected clients.
    pub fn broadcast_state(&mut self, enabled: bool) {
        self.state = enabled;
        let line = state_line(enabled);
        let mut drop_indexes = Vec::new();
        for (i, client) in self.clients.iter_mut().enumerate() {
            // Clients awaiting their initial snapshot get it in poll().
            if client.needs_state {
                continue;
            }
            if client.writer.write_all(line.as_bytes()).is_err() {
                drop_indexes.push(i);
            }
        }
        for i in drop_indexes.into_iter().rev() {
            self.clients.swap_remove(i);
        }
    }

    /// Accept connections, replay state to new clients, and return any
    /// pending commands.
    pub fn poll(&mut self) -> Vec<GamingCommand> {
        let mut commands = Vec::new();

        if let Some(ref listener) = self.listener {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        match peer_uid(&stream) {
                            Some(uid) if uid == our_uid() => {}
                            other => {
                                tracing::warn!(
                                    ?other,
                                    "rejecting gaming IPC connection from foreign uid"
                                );
                                continue;
                            }
                        }
                        stream.set_nonblocking(true).ok();
                        let writer = match stream.try_clone() {
                            Ok(w) => w,
                            Err(_) => continue,
                        };
                        self.clients.push(ClientConn {
                            reader: BufReader::new(stream),
                            writer,
                            needs_state: true,
                        });
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!(?e, "gaming IPC accept error");
                        break;
                    }
                }
            }
        }

        let snapshot = state_line(self.state);
        let mut drop_indexes = Vec::new();
        for (i, client) in self.clients.iter_mut().enumerate() {
            if client.needs_state {
                if client.writer.write_all(snapshot.as_bytes()).is_ok() {
                    client.needs_state = false;
                } else {
                    drop_indexes.push(i);
                    continue;
                }
            }

            let mut line = String::new();
            loop {
                line.clear();
                match client.reader.read_line(&mut line) {
                    Ok(0) => {
                        drop_indexes.push(i);
                        break;
                    }
                    Ok(_) => {
                        if line.trim() == "toggle" {
                            commands.push(GamingCommand::Toggle);
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

        commands
    }
}

fn state_line(enabled: bool) -> String {
    format!("gaming:{}\n", if enabled { 1 } else { 0 })
}

impl crate::state::Lantern {
    /// Drain the gaming IPC socket: accept new clients and execute any
    /// toggle commands from the Command Center tile.
    pub fn poll_gaming_ipc(&mut self) {
        let commands = self.gaming_ipc.poll();
        for cmd in commands {
            match cmd {
                GamingCommand::Toggle => self.toggle_gaming_mode(),
            }
        }
    }
}

//! Unix-socket IPC for System Settings ↔ compositor HDR control.
//!
//! Mirrors `workspace_ipc.rs`: newline-delimited UTF-8 over
//! `/run/user/{uid}/lntrn-hdr.sock`, nonblocking, SO_PEERCRED uid-gated.
//!
//! The settings app can't learn a display's HDR capability from
//! wlr-output-management (it has no HDR fields), and it needs a live channel to
//! request enable/disable. This socket carries both:
//!
//!   # Compositor → Settings (on connect, one line per output):
//!   `caps:<output>:<hdr_capable>:<max_nits>:<min_milli_nits>`
//!     hdr_capable = 0|1, max_nits = peak luminance, min_milli_nits = min*1000
//!
//!   # Settings → Compositor:
//!   `set:<output>:<enable>:<sdr_nits>`   # enable = 0|1
//!   `confirm:<output>`                   # keep HDR (cancel auto-revert)
//!
//!   # Compositor → Settings (HDR confirmation lifecycle):
//!   `pending:<output>:<seconds>`   # HDR engaged, awaiting "Keep" within N s
//!   `confirmed:<output>`           # kept
//!   `reverted:<output>`            # auto-reverted (timed out / failed)
//!
//! The compositor still reads `hdr`/`sdr_brightness` from lantern.toml at
//! startup; this socket only carries *live* changes + capability discovery.

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
    PathBuf::from(format!("/run/user/{}/lntrn-hdr.sock", uid))
}

/// A live HDR request from the settings app: either set enable/sdr, or a bare
/// "keep it" confirmation (`confirm = true`, other fields ignored).
pub struct HdrSetCommand {
    pub output: String,
    pub enable: bool,
    pub sdr_nits: u32,
    pub confirm: bool,
}

/// Capability snapshot for one output, cached so newly-connecting clients get
/// the full picture immediately.
#[derive(Clone)]
pub struct OutputCaps {
    pub output: String,
    pub hdr_capable: bool,
    pub max_nits: u32,
    pub min_milli_nits: u32,
}

pub struct HdrIpc {
    listener: Option<UnixListener>,
    clients: Vec<ClientConn>,
    /// Latest per-output capability snapshot, replayed to new clients.
    caps: Vec<OutputCaps>,
}

struct ClientConn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    needs_caps: bool,
}

impl HdrIpc {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(?e, "failed to chmod 0600 on HDR IPC socket");
                }
                tracing::info!(?path, "HDR IPC socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind HDR IPC socket");
                None
            }
        };
        Self {
            listener,
            clients: Vec::new(),
            caps: Vec::new(),
        }
    }

    /// Update (or insert) the cached capability for an output and push it to any
    /// already-connected clients. Called when an output is detected/removed.
    pub fn update_caps(&mut self, caps: OutputCaps) {
        let line = format_caps_line(&caps);
        match self.caps.iter_mut().find(|c| c.output == caps.output) {
            Some(slot) => *slot = caps,
            None => self.caps.push(caps),
        }
        self.broadcast(&line);
    }

    /// Drop an output's cached capability (output disconnected).
    pub fn remove_output(&mut self, output: &str) {
        self.caps.retain(|c| c.output != output);
    }

    /// Accept connections, replay caps to new clients, and return any pending
    /// `set:` commands.
    pub fn poll(&mut self) -> Vec<HdrSetCommand> {
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
                                    "rejecting HDR IPC connection from foreign uid"
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
                            needs_caps: true,
                        });
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!(?e, "HDR IPC accept error");
                        break;
                    }
                }
            }
        }

        // Replay the cached caps to any client that just connected.
        let snapshot: Vec<String> = self.caps.iter().map(format_caps_line).collect();
        let mut drop_indexes = Vec::new();
        for (i, client) in self.clients.iter_mut().enumerate() {
            if client.needs_caps {
                let mut ok = true;
                for line in &snapshot {
                    if client.writer.write_all(line.as_bytes()).is_err() {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    client.needs_caps = false;
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
                        if let Some(cmd) = parse_set_command(line.trim()) {
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

        commands
    }

    /// Tell the settings app HDR just engaged on `output` and a confirmation is
    /// expected within `secs` seconds (else it auto-reverts).
    pub fn notify_pending(&mut self, output: &str, secs: u32) {
        self.broadcast(&format!("pending:{output}:{secs}"));
    }

    /// Tell the settings app HDR was confirmed kept.
    pub fn notify_confirmed(&mut self, output: &str) {
        self.broadcast(&format!("confirmed:{output}"));
    }

    /// Tell the settings app HDR was auto-reverted.
    pub fn notify_reverted(&mut self, output: &str) {
        self.broadcast(&format!("reverted:{output}"));
    }

    fn broadcast(&mut self, line: &str) {
        let payload = if line.ends_with('\n') {
            line.to_string()
        } else {
            format!("{}\n", line)
        };
        let mut drop_indexes = Vec::new();
        for (i, client) in self.clients.iter_mut().enumerate() {
            // Don't write to clients that haven't had their initial snapshot yet
            // — poll() will send them the full caps list (including this one).
            if client.needs_caps {
                continue;
            }
            if client.writer.write_all(payload.as_bytes()).is_err() {
                drop_indexes.push(i);
            }
        }
        for i in drop_indexes.into_iter().rev() {
            self.clients.swap_remove(i);
        }
    }
}

fn format_caps_line(caps: &OutputCaps) -> String {
    format!(
        "caps:{}:{}:{}:{}\n",
        caps.output,
        if caps.hdr_capable { 1 } else { 0 },
        caps.max_nits,
        caps.min_milli_nits
    )
}

fn parse_set_command(msg: &str) -> Option<HdrSetCommand> {
    // confirm:<output>
    if let Some(output) = msg.strip_prefix("confirm:") {
        if output.is_empty() {
            return None;
        }
        return Some(HdrSetCommand {
            output: output.to_string(),
            enable: false,
            sdr_nits: 0,
            confirm: true,
        });
    }
    // set:<output>:<enable>:<sdr_nits>
    let parts: Vec<&str> = msg.splitn(4, ':').collect();
    if parts.len() != 4 || parts[0] != "set" {
        return None;
    }
    Some(HdrSetCommand {
        output: parts[1].to_string(),
        enable: parts[2] == "1",
        sdr_nits: parts[3].parse().ok()?,
        confirm: false,
    })
}

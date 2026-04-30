//! Single-instance IPC for `lntrn-command-center`.
//!
//! Pattern: send-or-become-daemon. The first invocation binds the
//! socket and becomes the long-lived daemon process; subsequent
//! invocations send a one-byte command to that socket and exit.
//!
//! Path: `/run/user/{uid}/lntrn-command-center.sock` — matches the
//! Lantern runtime-socket convention from `workspace_ipc.rs` and
//! `hover_preview.rs`.

use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

use anyhow::Result;

/// Compute the canonical socket path for this user.
pub fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-command-center.sock", uid))
}

/// One-byte commands the daemon understands.
pub mod cmd {
    pub const TOGGLE: &[u8] = b"T";
    pub const SHOW: &[u8] = b"S";
    pub const HIDE: &[u8] = b"H";
}

/// Try to send a command to a running daemon. Returns `Ok(true)` if a
/// daemon was found and accepted the message; `Ok(false)` if no daemon
/// is listening (socket missing, connection refused, etc.) — in that
/// case the caller should fall back to becoming the daemon.
pub fn send(msg: &[u8]) -> Result<bool> {
    let path = socket_path();
    if !path.exists() {
        return Ok(false);
    }
    let client = UnixDatagram::unbound()?;
    match client.send_to(msg, &path) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Bind the socket and become the daemon. Caller is responsible for
/// keeping the returned `UnixDatagram` alive for the daemon's lifetime
/// and for polling it (non-blocking) on each render-loop iteration.
pub fn bind_daemon() -> Result<UnixDatagram> {
    let path = socket_path();
    // Stale socket from a crashed previous daemon — clear it.
    let _ = std::fs::remove_file(&path);
    let sock = UnixDatagram::bind(&path)?;
    sock.set_nonblocking(true)?;
    tracing::info!(?path, "command-center daemon bound socket");
    Ok(sock)
}

/// Drain all queued messages from the socket. Each is a single command
/// byte (see `cmd::*`). Returns the most recent command if any arrived.
pub fn drain(sock: &UnixDatagram) -> Option<Cmd> {
    let mut buf = [0u8; 32];
    let mut latest: Option<Cmd> = None;
    while let Ok(n) = sock.recv(&mut buf) {
        if n == 0 {
            continue;
        }
        latest = Cmd::parse(buf[0]);
    }
    latest
}

/// Parsed daemon command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Toggle,
    Show,
    Hide,
}

impl Cmd {
    fn parse(b: u8) -> Option<Self> {
        match b {
            b'T' => Some(Cmd::Toggle),
            b'S' => Some(Cmd::Show),
            b'H' => Some(Cmd::Hide),
            _ => None,
        }
    }
}

/// Best-effort cleanup on daemon shutdown — remove the socket file so
/// the next invocation doesn't try to connect to a dead one.
pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}

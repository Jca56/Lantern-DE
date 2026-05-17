//! Single-instance IPC for `lntrn-command-center`.
//!
//! Pattern: send-or-become-daemon. The first invocation binds the
//! socket and becomes the long-lived daemon process; subsequent
//! invocations connect, send a one-byte command, and exit.
//!
//! Path: `/run/user/{uid}/lntrn-command-center.sock` — matches the
//! Lantern runtime-socket convention from `workspace_ipc.rs` and
//! `hover_preview.rs`.
//!
//! ## Why SOCK_STREAM (not SOCK_DGRAM)
//!
//! Datagram peers cannot be authenticated via `SO_PEERCRED`, which only
//! works on connected stream sockets. Switching to stream lets the
//! daemon reject any peer whose uid doesn't match ours, even if
//! `$XDG_RUNTIME_DIR` perms are misconfigured.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

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
        tracing::debug!(?path, "no daemon socket present");
        return Ok(false);
    }
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(?e, ?path, "connect to daemon failed; treating as no daemon");
            return Ok(false);
        }
    };
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    match stream.write_all(msg) {
        Ok(_) => Ok(true),
        Err(e) => {
            tracing::warn!(?e, "write to daemon failed; treating as no daemon");
            Ok(false)
        }
    }
}

/// Bind the socket and become the daemon. Caller is responsible for
/// keeping the returned `UnixListener` alive for the daemon's lifetime
/// and for polling it (non-blocking) on each render-loop iteration.
pub fn bind_daemon() -> Result<UnixListener> {
    let path = socket_path();
    // If a socket file already exists, probe before removing it. Connect
    // success means a healthy daemon is alive on that inode — clobbering
    // the path would orphan its listener and leave two daemons fighting
    // for the same address (next invocation lands on whichever inode the
    // path currently points to). Bail loudly instead.
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            anyhow::bail!(
                "another command-center daemon is already listening at {} — refusing to clobber",
                path.display()
            );
        }
        // Connect failed → stale socket from a crashed previous daemon. Safe to clear.
        let _ = fs::remove_file(&path);
    }
    let sock = UnixListener::bind(&path)?;
    sock.set_nonblocking(true)?;
    // Belt-and-suspenders: $XDG_RUNTIME_DIR is already 0700, but make
    // the socket itself 0600 too so an unusual umask can't expose it.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    tracing::info!(?path, "command-center daemon bound socket");
    Ok(sock)
}

/// Read SO_PEERCRED on a connected stream. Returns `None` on failure.
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

/// Accept all pending connections, read their command bytes, and
/// return the most recent valid command.
///
/// Each connection is expected to send 1+ command bytes and close.
/// Connections from a uid different from ours are dropped silently
/// (and logged at warn level once per occurrence).
pub fn drain(sock: &UnixListener) -> Option<Cmd> {
    let our_uid = unsafe { libc::getuid() };
    let mut latest: Option<Cmd> = None;

    loop {
        match sock.accept() {
            Ok((mut stream, _)) => {
                match peer_uid(&stream) {
                    Some(uid) if uid == our_uid => {}
                    other => {
                        tracing::warn!(
                            ?other,
                            "rejecting command-center IPC connection from foreign uid"
                        );
                        continue;
                    }
                }
                stream
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .ok();
                let mut buf = [0u8; 32];
                if let Ok(n) = stream.read(&mut buf) {
                    for b in buf.iter().take(n) {
                        if let Some(c) = Cmd::parse(*b) {
                            latest = Some(c);
                        }
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => {
                tracing::warn!(?e, "command-center accept error");
                break;
            }
        }
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
    let _ = fs::remove_file(socket_path());
}

//! Trusted-client allowlist for privileged Wayland protocols.
//!
//! Resolves the calling client's `/proc/<pid>/exe` symlink and checks
//! whether the resulting canonical path is either:
//!   1. Inside `~/.lantern/bin/`, or
//!   2. Listed (full canonical path) in `~/.lantern/config/trusted-clients.toml`.
//!
//! Results are cached per `ClientId` so repeated bind/dispatch calls
//! don't hammer `/proc`. The cache is cleared lazily; entries belonging
//! to disconnected clients sit until manually pruned, which is fine
//! because `ClientId`s are unique-per-connection.

#![allow(dead_code)] // Wave 2.1 lays foundation; callers land in 2.2-2.5.

use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use smithay::reexports::wayland_server::Client;

/// Optional extra-allowlist parsed once from
/// `~/.lantern/config/trusted-clients.toml`. The TOML schema is just:
///
/// ```toml
/// paths = ["/usr/bin/grim", "/usr/bin/wl-copy"]
/// ```
///
/// Stored as canonicalized `PathBuf`s.
static EXTRA_ALLOWLIST: OnceLock<HashSet<PathBuf>> = OnceLock::new();

/// Cached canonical path of `~/.lantern/bin/`.
static LANTERN_BIN: OnceLock<Option<PathBuf>> = OnceLock::new();

fn lantern_bin() -> Option<&'static Path> {
    LANTERN_BIN
        .get_or_init(|| {
            let home = std::env::var_os("HOME")?;
            let path = PathBuf::from(home).join(".lantern/bin");
            std::fs::canonicalize(&path).ok()
        })
        .as_deref()
}

fn extra_allowlist() -> &'static HashSet<PathBuf> {
    EXTRA_ALLOWLIST.get_or_init(load_extra_allowlist)
}

fn load_extra_allowlist() -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let Some(home) = std::env::var_os("HOME") else { return set; };
    let toml_path = PathBuf::from(home).join(".lantern/config/trusted-clients.toml");
    let Ok(contents) = std::fs::read_to_string(&toml_path) else {
        // No file = empty allowlist. Not an error.
        return set;
    };
    // Tiny hand-parser — config-format is fixed: `paths = ["...", "..."]`.
    // Avoiding a `toml` crate dep just for this two-line file.
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Pull out anything between matched double-quotes.
        let mut chars = line.chars().peekable();
        let mut current = String::new();
        let mut in_quote = false;
        while let Some(c) = chars.next() {
            match c {
                '"' if !in_quote => {
                    in_quote = true;
                    current.clear();
                }
                '"' if in_quote => {
                    in_quote = false;
                    if let Ok(canonical) = std::fs::canonicalize(&current) {
                        set.insert(canonical);
                    } else {
                        tracing::warn!(
                            path = %current,
                            "trusted-clients.toml entry could not be canonicalized (file missing?), ignoring"
                        );
                    }
                    current.clear();
                }
                _ if in_quote => current.push(c),
                _ => {}
            }
        }
    }
    if !set.is_empty() {
        tracing::info!(count = set.len(), "loaded extra trusted-clients allowlist");
    }
    set
}

/// Read `/proc/<pid>/exe` and canonicalize. Returns `None` for kernel
/// threads or processes we lack permission to inspect (shouldn't happen
/// for our own children, but be defensive).
fn exe_for_pid(pid: i32) -> Option<PathBuf> {
    let link = format!("/proc/{pid}/exe");
    let target = std::fs::read_link(&link).ok()?;
    // `read_link` returns the symlink target; for `/proc/<pid>/exe` that
    // is already absolute, but `canonicalize` resolves any further
    // symlinks (e.g. `~/.lantern/bin/foo -> /home/.../target/release/foo`).
    std::fs::canonicalize(&target).ok()
}

/// Read SO_PEERCRED on a client's Unix socket. Captures the connecting
/// process at the *exact moment of connect*, before it has had a chance
/// to fork/exec. This is the race-resistant identification point.
fn peer_pid(stream: &UnixStream) -> Option<i32> {
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
    if ret == 0 {
        Some(ucred.pid)
    } else {
        None
    }
}

/// Compute trust for a freshly-connecting client.
///
/// Called from the Wayland listener once per connection, before any
/// protocol traffic, with the raw `UnixStream` we just accepted. Stores
/// the resulting bool in `ClientState::is_trusted` so subsequent global
/// filters can check via `client.get_data::<ClientState>()`.
pub fn compute_trust_at_connect(stream: &UnixStream) -> bool {
    let Some(pid) = peer_pid(stream) else {
        tracing::warn!("could not read SO_PEERCRED for new client; treating as untrusted");
        return false;
    };
    let Some(exe) = exe_for_pid(pid) else {
        tracing::warn!(pid, "could not resolve /proc/<pid>/exe; treating as untrusted");
        return false;
    };

    let trusted = if let Some(bin) = lantern_bin() {
        exe.starts_with(bin) || extra_allowlist().contains(&exe)
    } else {
        // No ~/.lantern/bin → everything is "untrusted". This is the
        // safe failure mode but unlikely in practice.
        extra_allowlist().contains(&exe)
    };

    if !trusted {
        tracing::warn!(
            pid,
            exe = ?exe,
            "untrusted Wayland client connected; privileged protocols will be denied"
        );
    } else {
        tracing::debug!(pid, exe = ?exe, "trusted Wayland client");
    }

    trusted
}

/// Filter callback for Smithay's `Client`-aware globals. Reads the
/// `is_trusted` bit that was computed in `compute_trust_at_connect` and
/// stored on `ClientState`. Returns `false` (untrusted) on any failure
/// to look up state.
pub fn is_trusted_client(client: &Client) -> bool {
    client
        .get_data::<crate::state::ClientState>()
        .map(|d| d.is_trusted)
        .unwrap_or(false)
}

/// Surface-level wrapper: resolves the surface's client and returns true
/// iff that client passed the trust check at connect time. Untrusted (or
/// no-client) surfaces return false.
pub fn is_trusted_surface(surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> bool {
    use smithay::reexports::wayland_server::Resource;
    surface.client().map_or(false, |c| is_trusted_client(&c))
}

/// Force-reload the extra allowlist. Useful if you want to add a runtime
/// command later to re-read the TOML without restarting the compositor.
/// Currently a no-op; kept here so the API exists for a future migration
/// from `OnceLock` to `Mutex<HashSet>`.
pub fn reload_extra_allowlist() {}

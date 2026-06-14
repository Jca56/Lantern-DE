//! lntrn-afterglow — Lantern's hardware tuning app (Afterburner, but cozier).
//!
//! Two binaries share this crate: `lntrn-afterglow` (GUI) and
//! `lntrn-afterglowd` (root helper daemon that owns all NVML/RAPL/cpufreq
//! writes). They talk over a Unix socket with a one-line-request /
//! one-line-response text protocol.

pub mod hw;
pub mod ipc;
pub mod profile;
pub mod protocol;

/// Root daemon socket. Lives outside /run/user so the root daemon can own it;
/// it gets chowned to the session user and peer-cred checked on connect.
pub const SOCKET_PATH: &str = "/run/lntrn-afterglowd.sock";

pub fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".lantern/config/lntrn-afterglow/settings.toml")
}

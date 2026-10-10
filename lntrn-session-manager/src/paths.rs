//! Where the session's files live.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/root".into()))
}

/// `~/.lantern/bin`: the Lantern binaries.
pub fn bin_dir() -> PathBuf {
    home().join(".lantern").join("bin")
}

/// `~/.lantern/log`: this session's log and its children's.
pub fn log_dir() -> PathBuf {
    home().join(".lantern").join("log")
}

/// `$XDG_RUNTIME_DIR`, or where logind puts it for this user.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", crate::procfs::own_uid())))
}

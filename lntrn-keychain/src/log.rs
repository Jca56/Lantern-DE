//! Tiny file logger to ~/.lantern/log/keychain.log.

use std::io::Write;

fn log_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".lantern/log/keychain.log")
}

fn write(level: &str, msg: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {level} {msg}");
    }
}

pub fn info(msg: &str) {
    write("INFO ", msg);
}
pub fn error(msg: &str) {
    write("ERROR", msg);
}

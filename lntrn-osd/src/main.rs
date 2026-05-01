mod layershell;
mod svg_icon;

use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

/// Canonical socket path for the OSD daemon.
///
/// Lives in `$XDG_RUNTIME_DIR` (typically `/run/user/<uid>/`, mode 0700) so
/// only the owning user can reach it. Falls back to `/tmp` only if the env
/// var is unset, which shouldn't happen in a real session.
pub fn socket_path() -> PathBuf {
    if let Ok(dir) = env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("lntrn-osd.sock");
        }
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/lntrn-osd.sock"))
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let msg = build_message(&args);
    let path = socket_path();

    // Try sending to an existing daemon (SOCK_STREAM connect+write+close).
    if path.exists() {
        if let Ok(mut stream) = UnixStream::connect(&path) {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
            if stream.write_all(msg.as_bytes()).is_ok() {
                return Ok(());
            }
        }
    }

    // No daemon running — become the daemon.
    let _ = fs::remove_file(&path);
    let sock = UnixListener::bind(&path)?;
    // Explicit 0600 — XDG_RUNTIME_DIR is already 0700, but belt-and-suspenders
    // protects against tmp fallback or unusual umasks.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    sock.set_nonblocking(true)?;

    let osd = parse_message(&msg);
    layershell::run(osd, sock)
}

fn build_message(args: &[String]) -> String {
    match args.get(1).map(|s| s.as_str()) {
        Some("mute") => "mute".to_string(),
        Some("volume") => {
            let vol = args.get(2)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0)
                .min(100);
            format!("volume {vol}")
        }
        Some("brightness") => {
            let val = args.get(2)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(50)
                .min(100);
            format!("brightness {val}")
        }
        _ => "volume 50".to_string(),
    }
}

#[derive(Clone, Copy)]
pub enum OsdMode {
    Volume { level: u32, muted: bool },
    Brightness { level: u32 },
}

pub fn parse_message(msg: &str) -> OsdMode {
    let msg = msg.trim();
    if msg == "mute" {
        OsdMode::Volume { level: 0, muted: true }
    } else if let Some(rest) = msg.strip_prefix("volume ") {
        let vol = rest.parse::<u32>().unwrap_or(0).min(100);
        OsdMode::Volume { level: vol, muted: false }
    } else if let Some(rest) = msg.strip_prefix("brightness ") {
        let val = rest.parse::<u32>().unwrap_or(50).min(100);
        OsdMode::Brightness { level: val }
    } else {
        OsdMode::Volume { level: 50, muted: false }
    }
}

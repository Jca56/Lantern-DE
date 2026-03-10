mod layershell;
mod svg_icon;

use std::env;
use std::os::unix::net::UnixDatagram;

const SOCK_PATH: &str = "/tmp/lntrn-osd.sock";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let msg = build_message(&args);

    // Try sending to an existing daemon
    if let Ok(client) = UnixDatagram::unbound() {
        if client.send_to(msg.as_bytes(), SOCK_PATH).is_ok() {
            return Ok(());
        }
    }

    // No daemon running — become the daemon
    let _ = std::fs::remove_file(SOCK_PATH);
    let sock = UnixDatagram::bind(SOCK_PATH)?;
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

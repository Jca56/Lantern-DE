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

    let (volume, muted) = parse_message(&msg);
    layershell::run(volume, muted, sock)
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
        _ => "volume 50".to_string(),
    }
}

pub fn parse_message(msg: &str) -> (u32, bool) {
    let msg = msg.trim();
    if msg == "mute" {
        (0, true)
    } else if let Some(rest) = msg.strip_prefix("volume ") {
        let vol = rest.parse::<u32>().unwrap_or(0).min(100);
        (vol, false)
    } else {
        (50, false)
    }
}

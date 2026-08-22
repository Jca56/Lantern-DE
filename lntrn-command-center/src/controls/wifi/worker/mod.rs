//! Worker thread for the WiFi tile.
//!
//! Splits responsibility:
//!   - This module owns the polling loop, command dispatch, and the
//!     Mullvad VPN status piggyback.
//!   - [`iwd`] handles iwd over D-Bus (`net.connman.iwd`) — the only
//!     supported backend. Both Lantern hosts (Arch laptop + Gentoo
//!     desktop) run iwd.
//!
//! The render thread doesn't know how the bytes get on the air; it just
//! sends [`super::WifiCmd`]s and receives [`super::WifiEvent`]s.

mod iwd;

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::{Network, WifiCmd, WifiEvent, WifiState};

pub(self) use super::{Band, BandEntry, Profile};

/// True if iwd is owned on the system bus and we can talk to it. The
/// WiFi tile hides itself entirely when this returns false.
pub(crate) fn is_available() -> bool {
    iwd::is_available()
}

/// Cheap status poll — drives the toolbar icon (connected ssid + bars).
const STATUS_INTERVAL: Duration = Duration::from_secs(1);
/// Full scan — drives the expanded network list. `Station.Scan` is
/// 1-2s, so this runs less often than the status poll.
const SCAN_INTERVAL: Duration = Duration::from_secs(8);

/// Worker entry point. Spawned from [`crate::controls::wifi::Wifi::new`].
pub(crate) fn run(tx: mpsc::Sender<WifiEvent>, cmd_rx: mpsc::Receiver<WifiCmd>) {
    let _ = tx.send(WifiEvent::Status(iwd::poll_status()));
    let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status()));
    let _ = tx.send(WifiEvent::Networks(iwd::scan_networks()));

    let mut last_status = Instant::now();
    let mut last_scan = Instant::now();
    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            iwd::handle_cmd(cmd, &tx);
            let _ = tx.send(WifiEvent::Networks(iwd::scan_networks()));
            let _ = tx.send(WifiEvent::Status(iwd::poll_status()));
            last_status = Instant::now();
            last_scan = Instant::now();
        }

        if last_status.elapsed() >= STATUS_INTERVAL {
            let _ = tx.send(WifiEvent::Status(iwd::poll_status()));
            let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status()));
            last_status = Instant::now();
        }
        if last_scan.elapsed() >= SCAN_INTERVAL {
            let _ = tx.send(WifiEvent::Networks(iwd::scan_networks()));
            last_scan = Instant::now();
            last_status = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

/// Check Mullvad VPN connection state. Returns `None` if the `mullvad`
/// CLI isn't installed or the daemon isn't responding (so the indicator
/// can hide cleanly). `Some(true)` = Connected, `Some(false)` = anything
/// else (Disconnected, Connecting, Blocked).
fn poll_vpn_status() -> Option<bool> {
    let out = Command::new("mullvad").arg("status").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    Some(first.trim().starts_with("Connected"))
}

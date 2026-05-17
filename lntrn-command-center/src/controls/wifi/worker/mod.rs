//! Backend-agnostic worker thread for the WiFi tile.
//!
//! Splits responsibility:
//!   - This module owns the polling loop, command dispatch, and the
//!     Mullvad VPN status piggyback (backend-agnostic).
//!   - [`nm`] handles NetworkManager (the laptop on Arch).
//!   - [`iwd`] handles iwd over D-Bus (the desktop on Gentoo).
//!
//! The render thread doesn't know or care which backend is in use; it
//! just sends [`super::WifiCmd`]s and receives [`super::WifiEvent`]s.

mod iwd;
mod nm;

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::{Network, WifiCmd, WifiEvent, WifiState};

// Re-export the types each backend module references.
pub(self) use super::{Band, BandEntry, Profile};

/// Which backend is talking to the WiFi stack on this machine.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Backend {
    /// NetworkManager via `nmcli` shellouts.
    Nm,
    /// iwd via its system-bus D-Bus interface (`net.connman.iwd`).
    Iwd,
}

impl Backend {
    /// Probe what's actually installed and running. iwd wins when both
    /// are present — that's the Lantern preference on the Gentoo desktop
    /// and matches the autostart wired up in `/etc/runlevels/default`.
    pub(crate) fn detect() -> Option<Backend> {
        if iwd::is_available() {
            return Some(Backend::Iwd);
        }
        if nm::is_available() {
            return Some(Backend::Nm);
        }
        None
    }
}

/// Cheap status poll — drives the toolbar icon (connected ssid + bars).
const STATUS_INTERVAL: Duration = Duration::from_secs(1);
/// Full scan — drives the expanded network list. Blocks the worker for
/// a few seconds (nmcli `--rescan yes` actually re-probes the air; iwd
/// `Station.Scan` is faster but still 1-2s), so this runs less often.
const SCAN_INTERVAL: Duration = Duration::from_secs(8);

/// Worker entry point. Spawned from [`crate::controls::wifi::Wifi::new`].
pub(crate) fn run(
    backend: Backend,
    tx: mpsc::Sender<WifiEvent>,
    cmd_rx: mpsc::Receiver<WifiCmd>,
) {
    // Prime the UI with whatever we can read right away.
    let _ = tx.send(WifiEvent::Status(poll_status(backend)));
    let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status()));
    let _ = tx.send(WifiEvent::Networks(scan_networks(backend)));

    let mut last_status = Instant::now();
    let mut last_scan = Instant::now();
    loop {
        // Drain pending commands first. Each command gets a fresh
        // Networks+Status refresh after it returns so the panel reflects
        // the new world state immediately.
        while let Ok(cmd) = cmd_rx.try_recv() {
            dispatch_cmd(backend, cmd, &tx);
            let _ = tx.send(WifiEvent::Networks(scan_networks(backend)));
            let _ = tx.send(WifiEvent::Status(poll_status(backend)));
            last_status = Instant::now();
            last_scan = Instant::now();
        }

        // Quick status poll → toolbar icon stays fresh every second.
        // Mullvad VPN status is cheap (~10ms IPC) so we piggyback it here.
        if last_status.elapsed() >= STATUS_INTERVAL {
            let _ = tx.send(WifiEvent::Status(poll_status(backend)));
            let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status()));
            last_status = Instant::now();
        }
        // Full scan → expanded network list.
        if last_scan.elapsed() >= SCAN_INTERVAL {
            let _ = tx.send(WifiEvent::Networks(scan_networks(backend)));
            last_scan = Instant::now();
            // The scan we just ran also drains a status snapshot; reset
            // the status timer so we don't immediately fire another one.
            last_status = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

fn poll_status(b: Backend) -> WifiState {
    match b {
        Backend::Nm => nm::poll_status(),
        Backend::Iwd => iwd::poll_status(),
    }
}

fn scan_networks(b: Backend) -> Vec<Network> {
    match b {
        Backend::Nm => nm::scan_networks(),
        Backend::Iwd => iwd::scan_networks(),
    }
}

fn dispatch_cmd(b: Backend, cmd: WifiCmd, tx: &mpsc::Sender<WifiEvent>) {
    match b {
        Backend::Nm => nm::handle_cmd(cmd, tx),
        Backend::Iwd => iwd::handle_cmd(cmd, tx),
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
    // First non-empty line is the tunnel state — typically "Connected",
    // "Disconnected", or "Connecting…".
    let first = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    Some(first.trim().starts_with("Connected"))
}

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
//!
//! One system-bus connection is held for the worker's lifetime (it used
//! to open a fresh one for every poll, roughly once a second), the
//! status cadence follows panel visibility, network-list refreshes only
//! run while the panel is on screen, and the `mullvad` binary is looked
//! up once instead of being spawned-and-failed every second on a
//! machine that doesn't have it.

mod iwd;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

use super::{Network, WifiCmd, WifiEvent, WifiState};
use crate::panel_visible::VisGate;

pub(self) use super::{Band, BandEntry, Profile};

/// True if iwd is owned on the system bus and we can talk to it. The
/// WiFi tile hides itself entirely when this returns false.
pub(crate) fn is_available() -> bool {
    iwd::is_available()
}

/// Cheap status poll — drives the toolbar icon (connected ssid + bars).
const STATUS_INTERVAL: Duration = Duration::from_secs(1);
/// Status poll while the panel is hidden — nothing is drawn, this only
/// keeps the cached state from going stale before the show-burst.
const HIDDEN_STATUS_INTERVAL: Duration = Duration::from_secs(15);
/// Full scan — drives the expanded network list. `Station.Scan` is
/// 1-2s, so this runs less often than the status poll.
const SCAN_INTERVAL: Duration = Duration::from_secs(8);
/// Backoff between attempts to reopen a dropped system-bus connection.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(3);

/// Worker entry point. Spawned from [`crate::controls::wifi::Wifi::new`].
pub(crate) fn run(tx: mpsc::Sender<WifiEvent>, cmd_rx: mpsc::Receiver<WifiCmd>) {
    let mullvad = mullvad_path();
    let mut conn: Option<Connection> = Connection::system().ok();
    let mut last_conn_try = Instant::now();
    let mut gate = VisGate::new();

    if let Some(c) = &conn {
        let _ = tx.send(WifiEvent::Status(iwd::poll_status(c)));
        let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status(mullvad.as_deref())));
        let _ = tx.send(WifiEvent::Networks(iwd::scan_networks(c)));
    }

    let mut last_status = Instant::now();
    let mut last_scan = Instant::now();
    loop {
        if conn.is_none() && last_conn_try.elapsed() >= RECONNECT_INTERVAL {
            conn = Connection::system().ok();
            last_conn_try = Instant::now();
        }
        let (visible, just_shown) = gate.poll();

        while let Ok(cmd) = cmd_rx.try_recv() {
            match &conn {
                Some(c) => {
                    iwd::handle_cmd(c, cmd, &tx);
                    let _ = tx.send(WifiEvent::Networks(iwd::scan_networks(c)));
                    let _ = tx.send(WifiEvent::Status(iwd::poll_status(c)));
                }
                None => {
                    if matches!(
                        cmd,
                        WifiCmd::Connect { .. } | WifiCmd::ActivateProfile { .. }
                    ) {
                        let _ =
                            tx.send(WifiEvent::ConnectFail("system bus unavailable".into()));
                    }
                }
            }
            last_status = Instant::now();
            last_scan = Instant::now();
        }

        let status_interval = if visible {
            STATUS_INTERVAL
        } else {
            HIDDEN_STATUS_INTERVAL
        };
        if just_shown || last_status.elapsed() >= status_interval {
            let mut drop_conn = false;
            if let Some(c) = &conn {
                let st = iwd::poll_status(c);
                // `Off` is also what a dead connection produces. Tell the
                // two apart before reporting, so a dbus restart triggers
                // a reconnect instead of a permanent "off" tile.
                if matches!(st, WifiState::Off) && !iwd::bus_alive(c) {
                    drop_conn = true;
                } else {
                    let _ = tx.send(WifiEvent::Status(st));
                }
            }
            if drop_conn {
                conn = None;
                last_conn_try = Instant::now();
            }
            let _ = tx.send(WifiEvent::VpnStatus(poll_vpn_status(mullvad.as_deref())));
            last_status = Instant::now();
        }
        // The network list only matters while someone can see it.
        if visible && (just_shown || last_scan.elapsed() >= SCAN_INTERVAL) {
            if let Some(c) = &conn {
                let _ = tx.send(WifiEvent::Networks(iwd::scan_networks(c)));
            }
            last_scan = Instant::now();
            last_status = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

/// Locate the `mullvad` CLI on `$PATH` once. `None` = not installed on
/// this machine, so the VPN indicator hides and we never fork for it.
fn mullvad_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("mullvad"))
        .find(|cand| cand.is_file())
}

/// Check Mullvad VPN connection state. Returns `None` if the `mullvad`
/// CLI isn't installed or the daemon isn't responding (so the indicator
/// can hide cleanly). `Some(true)` = Connected, `Some(false)` = anything
/// else (Disconnected, Connecting, Blocked).
fn poll_vpn_status(bin: Option<&Path>) -> Option<bool> {
    let bin = bin?;
    let out = Command::new(bin).arg("status").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    Some(first.trim().starts_with("Connected"))
}

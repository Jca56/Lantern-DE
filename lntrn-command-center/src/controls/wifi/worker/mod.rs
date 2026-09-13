//! Worker thread for the WiFi tile.
//!
//! Splits responsibility:
//!   - This module owns the polling loop and command dispatch.
//!   - [`iwd`] handles iwd over D-Bus (`net.connman.iwd`) — the only
//!     supported backend. Both Lantern hosts (Arch laptop + Gentoo
//!     desktop) run iwd.
//!
//! The render thread doesn't know how the bytes get on the air; it just
//! sends [`super::WifiCmd`]s and receives [`super::WifiEvent`]s.
//!
//! One system-bus connection is held for the worker's lifetime (it used
//! to open a fresh one for every poll, roughly once a second), the
//! status cadence follows panel visibility, and network-list refreshes
//! only run while the panel is on screen.

mod iwd;

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
    let mut conn: Option<Connection> = Connection::system().ok();
    let mut last_conn_try = Instant::now();
    let mut gate = VisGate::new();

    if let Some(c) = &conn {
        let _ = tx.send(WifiEvent::Status(iwd::poll_status(c)));
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
            let is_rescan = matches!(cmd, WifiCmd::Rescan);
            match &conn {
                Some(c) => {
                    iwd::handle_cmd(c, cmd, &tx);
                    let _ = tx.send(WifiEvent::Networks(iwd::scan_networks(c)));
                    let _ = tx.send(WifiEvent::Status(iwd::poll_status(c)));
                }
                None => match cmd {
                    WifiCmd::Connect { .. } | WifiCmd::ActivateProfile { .. } => {
                        let _ =
                            tx.send(WifiEvent::ConnectFail("system bus unavailable".into()));
                    }
                    WifiCmd::Disconnect => {
                        let _ = tx.send(WifiEvent::DisconnectDone(Err(
                            "system bus unavailable".into(),
                        )));
                    }
                    WifiCmd::Rescan | WifiCmd::DeleteProfile { .. } => {}
                },
            }
            // Sent after the fresh list so the spinner never stops
            // before the rows it was waiting for have landed.
            if is_rescan {
                let _ = tx.send(WifiEvent::ScanDone);
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

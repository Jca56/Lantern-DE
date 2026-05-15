//! Bluetooth worker thread — drives `bluetoothctl` for power / discoverable
//! / scan / connect / disconnect / pair operations, and runs the long-lived
//! interactive obexctl agent for incoming file transfers + the obexctl
//! shell-out for outgoing transfers.
//!
//! Communicates with `super::Bluetooth` via two mpsc channels:
//! - `BtCmd` flows in from the UI thread
//! - `BtEvent` flows back out so `Bluetooth::tick` can update its mirror of
//!   reality on each render frame.
//!
//! All bluetoothctl / obexctl calls are synchronous; the worker thread
//! is what keeps them off the render thread.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::obex::{self, ObexCmd};
use super::{BtCmd, BtEvent};

mod devices;
mod legacy_obex;
mod pair;

use devices::{forward_bt_result, read_devices, read_discoverable, read_powered};
use pair::interactive_pair;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn worker(tx: mpsc::Sender<BtEvent>, cmd_rx: mpsc::Receiver<BtCmd>) {
    // Initial poll.
    let _ = tx.send(BtEvent::Powered(read_powered()));
    let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
    let _ = tx.send(BtEvent::Devices(read_devices()));

    // Spawn the OBEX D-Bus thread. It handles both sends and the
    // incoming-file agent over a single org.bluez.obex session-bus
    // connection. The legacy obexctl helpers below
    // (`obex_incoming_agent`, `obex_send`) are kept in the file per the
    // never-delete rule but are no longer wired up.
    let obex_tx = obex::spawn(tx.clone());

    // Long-running `bluetoothctl scan on` child, kept alive while the
    // user has Scan toggled on. None when scan is off.
    let mut scan_child: Option<std::process::Child> = None;

    let mut last_poll = Instant::now();
    // While scanning, poll the device list more often so newly
    // discovered devices appear quickly.
    let mut scan_poll = Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                BtCmd::SetPowered(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl")
                        .args(["power", arg])
                        .output();
                    thread::sleep(Duration::from_millis(300));
                    let _ = tx.send(BtEvent::Powered(read_powered()));
                    let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::SetDiscoverable(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl")
                        .args(["discoverable", arg])
                        .output();
                    thread::sleep(Duration::from_millis(200));
                    let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
                }
                BtCmd::SetScan(on) => {
                    if on {
                        // Start a long-running `scan on` child (interactive).
                        // Killing this child via `disown`-style detach isn't
                        // necessary; we hold the Child handle and kill on
                        // SetScan(false) or worker shutdown.
                        if scan_child.is_none() {
                            use std::process::Stdio;
                            let child = Command::new("bluetoothctl")
                                .args(["--", "scan", "on"])
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .stdin(Stdio::null())
                                .spawn();
                            match child {
                                Ok(c) => {
                                    scan_child = Some(c);
                                    let _ = tx.send(BtEvent::Scan(true));
                                    scan_poll = Instant::now();
                                }
                                Err(e) => {
                                    let _ = tx.send(BtEvent::Error(format!(
                                        "Failed to start scan: {e}"
                                    )));
                                }
                            }
                        }
                    } else if let Some(mut child) = scan_child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                        // Tell bluetoothctl to definitely stop scanning,
                        // since killing the interactive process can leave
                        // discovery running on rare bluez builds.
                        let _ = Command::new("bluetoothctl")
                            .args(["scan", "off"])
                            .output();
                        let _ = tx.send(BtEvent::Scan(false));
                        let _ = tx.send(BtEvent::Devices(read_devices()));
                    }
                }
                BtCmd::Connect(mac) => {
                    let result = Command::new("bluetoothctl")
                        .args(["connect", &mac])
                        .output();
                    forward_bt_result(&tx, result);
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::Disconnect(mac) => {
                    let result = Command::new("bluetoothctl")
                        .args(["disconnect", &mac])
                        .output();
                    forward_bt_result(&tx, result);
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::Pair(mac) => {
                    // Run the interactive pair flow on a dedicated child
                    // bluetoothctl process. The reader thread inside
                    // `interactive_pair` parses prompts and emits PairPrompt
                    // events; replies come back through PairReply / PairCancel
                    // commands which we forward via the helper's stdin
                    // channel.
                    interactive_pair(&tx, &cmd_rx, &mac);
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::PairReply(_) | BtCmd::PairCancel => {
                    // Late reply with no active session — ignore.
                }
                BtCmd::SendFileToDevice { mac } => {
                    // 1) Pop the file picker by spawning lntrn-file-manager.
                    let picker = Command::new("lntrn-file-manager")
                        .args(["--pick", "--title", "Send via Bluetooth"])
                        .output();
                    let path = match picker {
                        Ok(o) if o.status.success() => {
                            let s = String::from_utf8_lossy(&o.stdout);
                            s.lines().next().unwrap_or("").to_string()
                        }
                        Ok(_) => {
                            // Cancelled — that's not a failure, just
                            // clear the inline "Picking…" badge so the
                            // row goes back to normal Connect/Connected.
                            let _ = tx.send(BtEvent::SendCleared {
                                mac: mac.clone(),
                            });
                            String::new()
                        }
                        Err(e) => {
                            let _ = tx.send(BtEvent::SendFailed {
                                mac: mac.clone(),
                                msg: format!("file picker: {e}"),
                            });
                            String::new()
                        }
                    };
                    if path.is_empty() {
                        continue;
                    }

                    // 2) Hand off to the D-Bus OBEX thread.
                    let _ = obex_tx.send(ObexCmd::SendFile {
                        mac: mac.clone(),
                        file_path: path,
                    });
                }
                BtCmd::CancelSend { mac: _ } => {
                    // The obex_send loop checks for a cancel flag via
                    // a shared atomic — but since obex_send is blocking
                    // on the worker thread, a CancelSend received while
                    // it's running gets queued; once obex_send returns
                    // we just pop it. Lifecycle: only one send at a time
                    // with this simple loop. Adequate for v1.
                }
                BtCmd::IncomingReply { accept } => {
                    // The UI tracks a single pending request; route to
                    // the latest auth on the obex side.
                    let cmd = if accept {
                        ObexCmd::AuthorizeReceive { auth_id: None }
                    } else {
                        ObexCmd::RejectReceive { auth_id: None }
                    };
                    let _ = obex_tx.send(cmd);
                }
            }
        }

        // Faster device polling while scanning so newly discovered
        // devices appear quickly.
        if scan_child.is_some() && scan_poll.elapsed() >= Duration::from_millis(1500) {
            let _ = tx.send(BtEvent::Devices(read_devices()));
            scan_poll = Instant::now();
        }

        if last_poll.elapsed() >= POLL_INTERVAL {
            let _ = tx.send(BtEvent::Powered(read_powered()));
            let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
            let _ = tx.send(BtEvent::Devices(read_devices()));
            last_poll = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

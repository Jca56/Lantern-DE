//! Tray worker thread — owns the session-bus connection, pumps the SNI
//! host, and shuttles commands / events across the channels in
//! [`super`]. Blocks in `poll()` on the bus fd so it costs nothing
//! while idle; the 100 ms cap is only so queued commands get picked up
//! promptly when the bus is quiet.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::sni::{HostEvent, SniHost};
use super::{dbusmenu, TrayCmd, TrayEvent};

/// Backoff between attempts to reach the bus / claim the watcher name.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
/// Upper bound on one poll() so commands never wait longer than this.
const POLL_CAP_MS: i32 = 100;

pub(super) fn run(tx: mpsc::Sender<TrayEvent>, cmd_rx: mpsc::Receiver<TrayCmd>) {
    let mut host: Option<SniHost> = None;
    let mut last_try = Instant::now() - RECONNECT_INTERVAL;

    loop {
        if host.is_none() {
            if last_try.elapsed() < RECONNECT_INTERVAL {
                // Nothing to do without a bus; drain commands so the
                // channel doesn't back up, then nap.
                while cmd_rx.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            last_try = Instant::now();
            match SniHost::connect() {
                Ok(h) => host = Some(h),
                Err(e) => {
                    tracing::warn!("tray: {e}; retrying in {RECONNECT_INTERVAL:?}");
                    continue;
                }
            }
        }
        let h = host.as_mut().expect("host set above");

        let fd = h.dbus_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, POLL_CAP_MS) };
        if rc < 0 || pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            tracing::warn!("tray: bus connection lost; reconnecting");
            host = None;
            let _ = tx.send(TrayEvent::Items(Vec::new()));
            continue;
        }

        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                TrayCmd::Activate { bus, x, y } => h.activate(&bus, x, y),
                TrayCmd::RequestMenu { bus, x, y } => h.request_menu(&bus, x, y),
                TrayCmd::MenuClick {
                    bus,
                    menu_path,
                    item_id,
                } => h.menu_click(&bus, &menu_path, item_id),
            }
        }

        let items_changed = h.poll();
        for ev in h.take_events() {
            match ev {
                HostEvent::MenuReady {
                    bus,
                    menu_path,
                    items,
                } => {
                    let items = dbusmenu::to_menu_items(&items);
                    if tx
                        .send(TrayEvent::MenuReady {
                            bus,
                            menu_path,
                            items,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        if items_changed && tx.send(TrayEvent::Items(h.items().to_vec())).is_err() {
            return;
        }
    }
}

//! Bluetooth discovery session over D-Bus.
//!
//! Why this exists instead of `bluetoothctl scan on`:
//! With `stdin = /dev/null`, bluetoothctl detects non-interactive mode
//! and exits immediately after issuing `StartDiscovery`. bluez then
//! tears the discovery back down on the next event loop tick — leaving
//! the panel with an empty device list and no way to find anything new.
//!
//! bluez tracks discovery clients per-D-Bus-connection: discovery stays
//! active as long as the caller's connection is alive. So we hold a
//! dedicated [`zbus::blocking::Connection`] for the lifetime of the
//! scan toggle. Dropping the [`ScanSession`] is sufficient to stop
//! discovery; we also call `StopDiscovery` explicitly first so bluez
//! doesn't have to wait for its disconnect-tracker to notice.

use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const BLUEZ_BUS: &str = "org.bluez";
const IFACE_ADAPTER: &str = "org.bluez.Adapter1";

pub(super) struct ScanSession {
    conn: Connection,
    adapter_path: OwnedObjectPath,
}

impl ScanSession {
    /// Open a fresh system-bus connection and start discovery on the
    /// first `org.bluez.Adapter1` we find (usually `/org/bluez/hci0`).
    pub(super) fn start() -> Result<Self, String> {
        let conn = Connection::system().map_err(|e| e.to_string())?;
        let adapter_path = find_adapter(&conn).ok_or_else(|| "no bluez adapter".to_string())?;
        conn.call_method(
            Some(BLUEZ_BUS),
            adapter_path.as_str(),
            Some(IFACE_ADAPTER),
            "StartDiscovery",
            &(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn, adapter_path })
    }

    /// Explicitly stop discovery, then drop the connection. Safe to
    /// call even if bluez has already cleaned us up.
    pub(super) fn stop(self) {
        let _ = self.conn.call_method(
            Some(BLUEZ_BUS),
            self.adapter_path.as_str(),
            Some(IFACE_ADAPTER),
            "StopDiscovery",
            &(),
        );
        // Connection drop closes the bus connection — bluez auto-cleans
        // any per-client discovery state at that point too.
    }
}

fn find_adapter(conn: &Connection) -> Option<OwnedObjectPath> {
    let reply = conn
        .call_method(
            Some(BLUEZ_BUS),
            "/",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .ok()?;
    let map: HashMap<
        OwnedObjectPath,
        HashMap<String, HashMap<String, OwnedValue>>,
    > = reply.body().deserialize().ok()?;
    for (path, ifaces) in map {
        if ifaces.contains_key(IFACE_ADAPTER) {
            return Some(path);
        }
    }
    None
}

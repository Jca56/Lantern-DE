//! iwd backend for the WiFi worker.
//!
//! Speaks `net.connman.iwd` directly on the system bus via zbus
//! (blocking API — the worker runs on its own OS thread, so a blocking
//! sync surface is the simplest match for the shared command-dispatch
//! loop in [`super`]).
//!
//! iwd's D-Bus API is intentionally smaller than NetworkManager's, so a
//! few `Network` fields stay empty here (per-BSSID frequency / channel /
//! bitrate, band-split tables). Existing UI code already gracefully
//! renders empty `bands` / `aps` vectors, so we leave them empty rather
//! than fake placeholders.
//!
//! Passphrase entry uses a transient `net.connman.iwd.Agent` registered
//! for the duration of a single `Connect` call. We hand iwd the password
//! the user typed when it asks via `RequestPassphrase`, then unregister.
//!
//! D-Bus reference: <https://git.kernel.org/pub/scm/network/wireless/iwd.git/tree/doc>

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use super::{Band, BandEntry, Network, Profile, WifiCmd, WifiEvent, WifiState};

const IWD_BUS: &str = "net.connman.iwd";
const IFACE_STATION: &str = "net.connman.iwd.Station";
const IFACE_STATION_DIAG: &str = "net.connman.iwd.StationDiagnostic";
const IFACE_NETWORK: &str = "net.connman.iwd.Network";
const IFACE_KNOWN: &str = "net.connman.iwd.KnownNetwork";
const IFACE_AGENT_MGR: &str = "net.connman.iwd.AgentManager";
const AGENT_PATH: &str = "/lntrn/wifi_agent";

/// Every call to iwd goes through here so the message carries D-Bus's
/// `NoAutoStart` flag. Without it, dbus-daemon spawns a fresh iwd the moment
/// the OpenRC-managed one goes away (we poll every second), which left
/// `rc-service iwd restart` reporting "crashed" next to a healthy orphan that
/// never re-read /etc/iwd/main.conf. Daemon absent → fast error instead.
fn iwd_call<B, R>(conn: &Connection, path: &str, iface: &str, method: &str, body: &B) -> zbus::Result<R>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    zbus::blocking::Proxy::new(conn, IWD_BUS, path, iface)?
        .call_with_flags(method, zbus::proxy::MethodFlags::NoAutoStart.into(), body)?
        .ok_or_else(|| zbus::Error::Failure("iwd: no reply".into()))
}

/// Cheap availability probe: open the system bus and check the well-known
/// iwd name is owned. False on any error — caller will fall back to NM.
pub(super) fn is_available() -> bool {
    let Ok(conn) = Connection::system() else {
        return false;
    };
    name_has_owner(&conn, IWD_BUS)
}

pub(super) fn poll_status() -> WifiState {
    let Ok(conn) = Connection::system() else {
        return WifiState::Off;
    };
    let Some(objects) = managed_objects(&conn) else {
        return WifiState::Off;
    };
    let Some((station_path, station_props)) = find_station(&objects) else {
        return WifiState::Off;
    };

    let state = string_prop(station_props, "State").unwrap_or_default();
    let connected_path = objpath_prop(station_props, "ConnectedNetwork");

    if state != "connected" || connected_path.is_none() {
        return WifiState::Disconnected;
    }
    let net_path = connected_path.unwrap();
    let ssid = objects
        .get(&net_path)
        .and_then(|ifaces| ifaces.get(IFACE_NETWORK))
        .and_then(|p| string_prop(p, "Name"))
        .unwrap_or_default();

    let signal = ordered_networks(&conn, &station_path)
        .into_iter()
        .find(|(p, _)| p == &net_path)
        .map(|(_, dbm)| dbm_to_pct(dbm))
        .unwrap_or(0);

    WifiState::Connected { ssid, signal }
}

pub(super) fn scan_networks() -> Vec<Network> {
    let Ok(conn) = Connection::system() else {
        return Vec::new();
    };
    let Some(objects) = managed_objects(&conn) else {
        return Vec::new();
    };
    let Some((station_path, _)) = find_station(&objects) else {
        return Vec::new();
    };

    let ordered = ordered_networks(&conn, &station_path);

    let mut nets = Vec::with_capacity(ordered.len());
    for (path, dbm) in ordered {
        let Some(ifaces) = objects.get(&path) else {
            continue;
        };
        let Some(net_props) = ifaces.get(IFACE_NETWORK) else {
            continue;
        };
        let name = string_prop(net_props, "Name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let kind = string_prop(net_props, "Type").unwrap_or_else(|| "open".to_string());
        let connected = bool_prop(net_props, "Connected").unwrap_or(false);
        let known_path = objpath_prop(net_props, "KnownNetwork");

        let mut profiles: Vec<Profile> = Vec::new();
        if let Some(kp) = &known_path {
            if let Some(kn) = objects.get(kp).and_then(|ifaces| ifaces.get(IFACE_KNOWN)) {
                profiles.push(Profile {
                    name: name.clone(),
                    // For iwd we use the KnownNetwork object path in
                    // place of NM's UUID — `WifiCmd::DeleteProfile`
                    // round-trips it back to `KnownNetwork.Forget`.
                    uuid: kp.to_string(),
                    pinned_bssid: None,
                    pinned_band: None,
                    timestamp: parse_iso_to_unix(
                        &string_prop(kn, "LastConnectedTime").unwrap_or_default(),
                    ),
                    active: connected,
                });
            }
        }

        let signal = dbm_to_pct(dbm);
        let (security, flags_summary) = security_strings(&kind);

        nets.push(Network {
            ssid: name,
            signal,
            security,
            in_use: connected,
            saved: known_path.is_some(),
            bssid: String::new(),
            mode: "Infra".to_string(),
            channel: String::new(),
            frequency: String::new(),
            rate: String::new(),
            profiles,
            // iwd's per-BasicServiceSet API only exposes the BSSID
            // address — no signal / frequency / channel / rate per AP.
            // Leave the band / AP tables empty; the UI hides those
            // sub-rows when there's nothing to show. The active row
            // gets enriched below from StationDiagnostic.
            bands: Vec::new(),
            aps: Vec::new(),
            selected_band: Band::G24,
            pinned_bssid: None,
            flags_summary,
        });
    }

    // Pull live diagnostics for the connected network and splice the
    // rich fields (BSSID, freq, channel, rate, cipher) into its row so
    // the expanded panel has something to show. iwd only exposes these
    // for the *active* connection — other rows stay sparse.
    if let Some(diag) = fetch_diagnostics(&conn, &station_path) {
        if let Some(net) = nets.iter_mut().find(|n| n.in_use) {
            apply_diagnostics(net, diag);
        }
    }

    nets
}

pub(super) fn handle_cmd(cmd: WifiCmd, tx: &mpsc::Sender<WifiEvent>) {
    match cmd {
        WifiCmd::Rescan => {
            if let Ok(conn) = Connection::system() {
                if let Some(objects) = managed_objects(&conn) {
                    if let Some((station_path, _)) = find_station(&objects) {
                        // Fire-and-forget; iwd refuses if a scan is already
                        // in progress, which is fine — the next periodic
                        // poll picks up whatever it produced.
                        let _: zbus::Result<()> =
                            iwd_call(&conn, station_path.as_str(), IFACE_STATION, "Scan", &());
                    }
                }
            }
            // Give the radio a beat to finish before the shared
            // dispatcher re-reads the ordered network list.
            std::thread::sleep(Duration::from_millis(500));
        }
        WifiCmd::Connect {
            ssid,
            password,
            band: _,
            bssid: _,
        } => {
            // iwd doesn't expose per-profile band / BSSID pinning the way
            // NetworkManager does, so those args are intentionally dropped.
            connect_by_ssid(&ssid, password.as_deref(), tx);
        }
        WifiCmd::DeleteProfile { uuid } => {
            // `uuid` is the KnownNetwork object path we stuffed in during
            // scan_networks. Call Forget on it.
            if let Ok(conn) = Connection::system() {
                let _: zbus::Result<()> = iwd_call(&conn, uuid.as_str(), IFACE_KNOWN, "Forget", &());
            }
        }
        WifiCmd::ActivateProfile { name } => {
            // iwd reuses the same Network object for both first-time
            // connect and reactivation, so just call Connect with no
            // passphrase — iwd uses the stored credentials.
            connect_by_ssid(&name, None, tx);
        }
    }
}

// ─── connect helpers ────────────────────────────────────────────────────────

fn connect_by_ssid(ssid: &str, password: Option<&str>, tx: &mpsc::Sender<WifiEvent>) {
    let Ok(conn) = Connection::system() else {
        let _ = tx.send(WifiEvent::ConnectFail("system bus unavailable".into()));
        return;
    };
    let Some(objects) = managed_objects(&conn) else {
        let _ = tx.send(WifiEvent::ConnectFail("iwd not responding".into()));
        return;
    };
    let Some(network_path) = find_network_path_by_name(&objects, ssid) else {
        let _ = tx.send(WifiEvent::ConnectFail(format!(
            "network '{}' not in range",
            ssid
        )));
        return;
    };

    // Register a transient passphrase agent if (and only if) we have a
    // password to provide. Open networks and reconnects to saved
    // networks skip this and let iwd do its thing.
    let agent_handle = match password {
        Some(pw) if !pw.is_empty() => match register_agent(&conn, pw.to_string()) {
            Ok(h) => Some(h),
            Err(e) => {
                let _ = tx.send(WifiEvent::ConnectFail(format!(
                    "agent register failed: {}",
                    e
                )));
                return;
            }
        },
        _ => None,
    };

    let result: zbus::Result<()> =
        iwd_call(&conn, network_path.as_str(), IFACE_NETWORK, "Connect", &());

    if let Some(h) = agent_handle {
        h.unregister(&conn);
    }

    match result {
        Ok(_) => {
            let _ = tx.send(WifiEvent::ConnectOk);
        }
        Err(e) => {
            let _ = tx.send(WifiEvent::ConnectFail(short_error(&e)));
        }
    }
}

struct AgentHandle;

impl AgentHandle {
    fn unregister(self, conn: &Connection) {
        let _: zbus::Result<()> = iwd_call(
            conn,
            "/net/connman/iwd",
            IFACE_AGENT_MGR,
            "UnregisterAgent",
            &(zbus::zvariant::ObjectPath::try_from(AGENT_PATH).unwrap(),),
        );
        let _ = conn
            .object_server()
            .remove::<PassphraseAgent, _>(AGENT_PATH);
    }
}

fn register_agent(conn: &Connection, passphrase: String) -> Result<AgentHandle, String> {
    let agent = PassphraseAgent {
        passphrase: Arc::new(Mutex::new(Some(passphrase))),
    };
    conn.object_server()
        .at(AGENT_PATH, agent)
        .map_err(|e| e.to_string())?;
    iwd_call::<_, ()>(
        conn,
        "/net/connman/iwd",
        IFACE_AGENT_MGR,
        "RegisterAgent",
        &(zbus::zvariant::ObjectPath::try_from(AGENT_PATH).map_err(|e| e.to_string())?,),
    )
    .map_err(|e| e.to_string())?;
    Ok(AgentHandle)
}

/// One-shot passphrase agent. iwd calls `RequestPassphrase` once per
/// `Network.Connect` attempt; we hand back the user's password and
/// then get unregistered by the caller.
struct PassphraseAgent {
    passphrase: Arc<Mutex<Option<String>>>,
}

#[zbus::interface(name = "net.connman.iwd.Agent")]
impl PassphraseAgent {
    fn release(&self) {}

    fn request_passphrase(
        &self,
        _network: zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::fdo::Result<String> {
        self.passphrase
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .ok_or_else(|| zbus::fdo::Error::Failed("no passphrase available".into()))
    }

    fn cancel(&self, _reason: String) {}
}

// ─── ObjectManager + property helpers ───────────────────────────────────────

type ObjectMap = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn managed_objects(conn: &Connection) -> Option<ObjectMap> {
    iwd_call(conn, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &()).ok()
}

fn find_station(objects: &ObjectMap) -> Option<(OwnedObjectPath, &HashMap<String, OwnedValue>)> {
    for (path, ifaces) in objects {
        if let Some(props) = ifaces.get(IFACE_STATION) {
            return Some((path.clone(), props));
        }
    }
    None
}

fn find_network_path_by_name(objects: &ObjectMap, ssid: &str) -> Option<OwnedObjectPath> {
    for (path, ifaces) in objects {
        if let Some(props) = ifaces.get(IFACE_NETWORK) {
            if string_prop(props, "Name").as_deref() == Some(ssid) {
                return Some(path.clone());
            }
        }
    }
    None
}

/// Live snapshot of the connected AP from `StationDiagnostic.GetDiagnostics`.
/// Every field is optional because iwd documents the dict as "best effort"
/// — drivers may omit entries when info isn't available.
struct DiagSnapshot {
    bssid: Option<String>,
    frequency_mhz: Option<u32>,
    channel: Option<u16>,
    rssi_dbm: Option<i16>,
    rx_bitrate_100kbps: Option<u32>,
    tx_bitrate_100kbps: Option<u32>,
    security: Option<String>,
    pairwise_cipher: Option<String>,
    rx_mode: Option<String>,
}

fn fetch_diagnostics(conn: &Connection, station: &OwnedObjectPath) -> Option<DiagSnapshot> {
    let dict: HashMap<String, OwnedValue> =
        iwd_call(conn, station.as_str(), IFACE_STATION_DIAG, "GetDiagnostics", &()).ok()?;
    Some(DiagSnapshot {
        bssid: string_prop(&dict, "ConnectedBss"),
        frequency_mhz: dict
            .get("Frequency")
            .and_then(|v| u32::try_from(v.clone()).ok()),
        channel: dict
            .get("Channel")
            .and_then(|v| u16::try_from(v.clone()).ok()),
        rssi_dbm: dict.get("RSSI").and_then(|v| i16::try_from(v.clone()).ok()),
        rx_bitrate_100kbps: dict
            .get("RxBitrate")
            .and_then(|v| u32::try_from(v.clone()).ok()),
        tx_bitrate_100kbps: dict
            .get("TxBitrate")
            .and_then(|v| u32::try_from(v.clone()).ok()),
        security: string_prop(&dict, "Security"),
        pairwise_cipher: string_prop(&dict, "PairwiseCipher"),
        rx_mode: string_prop(&dict, "RxMode"),
    })
}

/// Splice live diagnostics into the active `Network` so the expanded
/// row shows real bssid / frequency / channel / bitrate / cipher /
/// PHY mode info instead of the empty defaults from the scan pass.
fn apply_diagnostics(net: &mut Network, diag: DiagSnapshot) {
    if let Some(bssid) = &diag.bssid {
        net.bssid = bssid.clone();
    }
    if let Some(mhz) = diag.frequency_mhz {
        net.frequency = format!("{} MHz", mhz);
    }
    if let Some(ch) = diag.channel {
        net.channel = ch.to_string();
    }
    // Bitrate is reported in 100 kbit/s units — divide by 10 for Mbit/s.
    // iwd gives the *measured* PHY rate (which fluctuates with airtime),
    // not the negotiated peak that nmcli displays — so the number will
    // run a bit lower than the NM-era headline. We pick the higher of
    // rx/tx to track the better direction.
    let best_kbps = diag
        .rx_bitrate_100kbps
        .into_iter()
        .chain(diag.tx_bitrate_100kbps)
        .max();
    if let Some(rate) = best_kbps {
        net.rate = format!("{} MBit/s", rate / 10);
    }
    if let (Some(sec), Some(cipher)) = (&diag.security, &diag.pairwise_cipher) {
        // Short label on the row ("WPA3"); detailed summary below.
        net.security = sec.split('-').next().unwrap_or(sec.as_str()).to_string();
        // Compose a Network Manager-style detail line that the existing
        // expanded panel knows how to render.
        let mut parts = vec![sec.clone(), cipher.clone()];
        if let Some(mode) = &diag.rx_mode {
            parts.push(mode.clone());
        }
        net.flags_summary = parts.join(" · ");
    }
    // Build a BandEntry so the band-selector + AP rows have something
    // real to draw. Even though iwd never gives us *multiple* bands per
    // SSID, a single populated entry lights up the channel/freq/rate
    // chips in the expanded card.
    if let Some(mhz) = diag.frequency_mhz {
        let band = Band::from_mhz(mhz).unwrap_or(Band::G24);
        let entry = BandEntry {
            band,
            signal: diag
                .rssi_dbm
                .map(|d| dbm_to_pct(d.saturating_mul(100)))
                .unwrap_or(net.signal),
            bssid: net.bssid.clone(),
            channel: net.channel.clone(),
            frequency: net.frequency.clone(),
            rate: net.rate.clone(),
        };
        // Sharper signal reading from diagnostics replaces the rougher
        // GetOrderedNetworks value for the connected row.
        if diag.rssi_dbm.is_some() {
            net.signal = entry.signal;
        }
        net.selected_band = band;
        net.bands = vec![entry.clone()];
        net.aps = vec![entry];
    }
}

fn ordered_networks(conn: &Connection, station: &OwnedObjectPath) -> Vec<(OwnedObjectPath, i16)> {
    iwd_call(conn, station.as_str(), IFACE_STATION, "GetOrderedNetworks", &()).unwrap_or_default()
}

fn name_has_owner(conn: &Connection, name: &str) -> bool {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(name,),
    )
    .ok()
    .and_then(|r| r.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

fn string_prop(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = props.get(key)?.clone();
    String::try_from(v).ok()
}

fn bool_prop(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    let v = props.get(key)?.clone();
    bool::try_from(v).ok()
}

fn objpath_prop(props: &HashMap<String, OwnedValue>, key: &str) -> Option<OwnedObjectPath> {
    let v = props.get(key)?.clone();
    OwnedObjectPath::try_from(v).ok()
}

// ─── small conversions ──────────────────────────────────────────────────────

/// iwd signal strengths are in `dBm × 100` (int16). Map linearly:
/// `-100 dBm → 0%`, `-50 dBm → 100%`, clamped.
fn dbm_to_pct(dbm_x100: i16) -> u32 {
    let dbm = dbm_x100 as f32 / 100.0;
    let pct = ((dbm + 100.0) * 2.0).clamp(0.0, 100.0);
    pct as u32
}

/// Parse iwd's ISO-8601 LastConnectedTime ("2026-05-17T16:40:57Z") to a
/// Unix timestamp. Best-effort — returns 0 on any parse failure, which
/// matches the "never activated" sentinel the rest of the UI expects.
fn parse_iso_to_unix(s: &str) -> i64 {
    if s.is_empty() {
        return 0;
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

/// Map iwd's `Type` field to NM-shaped `security` and `flags_summary`
/// strings the UI already knows how to render.
fn security_strings(kind: &str) -> (String, String) {
    match kind {
        "open" => (String::new(), "Open (no encryption)".to_string()),
        "psk" => ("WPA2".to_string(), "WPA2/WPA3 · PSK".to_string()),
        "8021x" => ("802.1X".to_string(), "WPA2 · 802.1X".to_string()),
        "wep" => ("WEP".to_string(), "WEP".to_string()),
        other => (other.to_string(), other.to_string()),
    }
}

/// Trim a zbus error down to a single human-friendly line for the UI.
fn short_error(e: &zbus::Error) -> String {
    let s = e.to_string();
    s.lines().next().unwrap_or("connect failed").to_string()
}

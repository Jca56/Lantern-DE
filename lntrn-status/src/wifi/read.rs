//! What iwd says, read into a [`WifiState`]: its device and station, the
//! networks in range in the order iwd ranks them, their signal as a
//! share; and whether a cable is plugged in, from sysfs.

use std::fs;
use std::path::Path;

use lntrn_dbus::Value;

use super::{DEVICE, IWD, NETWORK, Network, STATION, Security, WifiState, Link};
use crate::bus::{self, Bus, CallError, Objects};

/// The WiFi device: the first by path.
pub(super) fn device_path(objects: &Objects) -> Option<String> {
    let mut devices: Vec<&String> = objects.iter().filter(|(_, ifaces)| ifaces.contains_key(DEVICE)).map(|(p, _)| p).collect();
    devices.sort();
    devices.first().map(|p| (*p).clone())
}

/// Everything iwd says now.
pub(super) fn snapshot(bus: &mut Bus) -> Result<WifiState, CallError> {
    let objects = bus.objects(IWD)?;
    let Some(path) = device_path(&objects) else { return Ok(WifiState::default()) };
    let device = &objects[&path][DEVICE];
    let Some(station) = objects[&path].get(STATION) else {
        return Ok(WifiState { present: true, powered: bus::flag(device, "Powered"), ..WifiState::default() });
    };
    let ordered = bus.call0(IWD, &path, STATION, "GetOrderedNetworks").map(|r| ordered(r.first())).unwrap_or_default();
    Ok(read(&objects, station, &ordered, bus::flag(device, "Powered")))
}

/// `GetOrderedNetworks`' `a(on)`: each network's path and its signal in
/// hundredths of a dBm.
fn ordered(value: Option<&Value>) -> Vec<(String, i32)> {
    let Some(list) = value.and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|v| match v {
            Value::Struct(f) if f.len() == 2 => Some((f[0].as_str()?.to_owned(), f[1].as_i32()?)),
            _ => None,
        })
        .collect()
}

/// iwd's signal (`-50 dBm` ×100) as 0 to 100: -100 dBm and below none,
/// -50 and above full.
fn strength(centi_dbm: i32) -> u32 {
    let dbm = f64::from(centi_dbm) / 100.0;
    ((dbm + 100.0) * 2.0).clamp(0.0, 100.0).round() as u32
}

/// The state from iwd's objects: its station's, and the networks in
/// range in `ordered`.
fn read(objects: &Objects, station: &bus::Props, ordered: &[(String, i32)], powered: bool) -> WifiState {
    let state = bus::text(station, "State").unwrap_or_default();
    let in_use = bus::text(station, "ConnectedNetwork");
    let mut networks: Vec<Network> = ordered
        .iter()
        .filter_map(|(net, signal)| {
            let props = objects.get(net)?.get(NETWORK)?;
            let name = bus::text(props, "Name").filter(|n| !n.is_empty())?;
            let security = Security::from_type(&bus::text(props, "Type").unwrap_or_default());
            Some(Network { path: net.clone(), name, security, known: props.contains_key("KnownNetwork"), connected: bus::flag(props, "Connected"), strength: strength(*signal) })
        })
        .collect();
    // In use first; iwd orders the rest by signal already.
    networks.sort_by_key(|n| !n.connected);
    let link = match state.as_str() {
        "connected" | "roaming" => Link::Connected,
        "connecting" => Link::Connecting,
        _ => Link::Disconnected,
    };
    let current = in_use.as_ref().and_then(|p| objects.get(p)?.get(NETWORK));
    let ssid = current.and_then(|p| bus::text(p, "Name"));
    let strength = in_use.as_ref().and_then(|p| ordered.iter().find(|(n, _)| n == p)).map_or(0, |(_, s)| strength(*s));
    WifiState { present: true, powered, link, ssid, strength, scanning: bus::flag(station, "Scanning"), networks, ..WifiState::default() }
}

/// The saved network behind the network in range at `network`, by path.
pub(super) fn known(objects: &Objects, network: &str) -> Option<String> {
    bus::text(objects.get(network)?.get(NETWORK)?, "KnownNetwork")
}

/// Whether a physical wired port under `root` (`/sys/class/net`) has a
/// cable in and its link up: not WiFi, not a bridge, tunnel or the like.
pub(super) fn wired(root: &Path) -> bool {
    fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).any(|dev| {
        let word = |name: &str| fs::read_to_string(dev.join(name)).map(|s| s.trim().to_owned()).unwrap_or_default();
        // ARPHRD_ETHER, on a real device, and no radio.
        word("type") == "1" && dev.join("device").exists() && !dev.join("wireless").exists() && !dev.join("phy80211").exists() && word("carrier") == "1" && word("operstate") == "up"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, Value)]) -> bus::Props {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    }

    #[test]
    fn signal_reads_as_a_share_and_as_bars() {
        assert_eq!((strength(-5000), strength(-7500), strength(-10500), strength(-3000)), (100, 50, 0, 100));
        let at = |strength, link| WifiState { strength, link, ..WifiState::default() }.bars();
        assert_eq!((at(100, Link::Connected), at(50, Link::Connected), at(10, Link::Connected)), (4, 3, 1));
        assert_eq!(at(100, Link::Disconnected), 0);
    }

    #[test]
    fn the_station_and_its_networks_read_with_the_one_in_use_first() {
        let s = |v: &str| Value::String(v.to_owned());
        let o = |v: &str| Value::ObjectPath(v.to_owned());
        let dev = "/net/connman/iwd/0/4";
        let (home, cafe, open) = (format!("{dev}/486f6d65_psk"), format!("{dev}/43616665_psk"), format!("{dev}/46726565_open"));
        let mut objects: Objects = HashMap::new();
        let station = props(&[("State", s("connected")), ("ConnectedNetwork", o(&home)), ("Scanning", Value::Bool(false))]);
        objects.insert(dev.into(), HashMap::from([(DEVICE.to_owned(), props(&[("Powered", Value::Bool(true))])), (STATION.to_owned(), station.clone())]));
        objects.insert(home.clone(), HashMap::from([(NETWORK.to_owned(), props(&[("Name", s("Home")), ("Type", s("psk")), ("Connected", Value::Bool(true)), ("KnownNetwork", o("/k/1"))]))]));
        objects.insert(cafe.clone(), HashMap::from([(NETWORK.to_owned(), props(&[("Name", s("Cafe")), ("Type", s("psk")), ("Connected", Value::Bool(false))]))]));
        objects.insert(open.clone(), HashMap::from([(NETWORK.to_owned(), props(&[("Name", s("Free")), ("Type", s("open")), ("Connected", Value::Bool(false))]))]));
        assert_eq!(device_path(&objects).as_deref(), Some(dev));
        let ordered = vec![(cafe.clone(), -4000), (home.clone(), -6000), (open.clone(), -8000)];
        let state = read(&objects, &station, &ordered, true);
        assert_eq!((state.link, state.ssid.as_deref(), state.strength, state.powered), (Link::Connected, Some("Home"), 80, true));
        let names: Vec<&str> = state.networks.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Home", "Cafe", "Free"]);
        assert!(state.networks[0].known && state.networks[0].joinable());
        assert!(!state.networks[1].joinable(), "a password iwd doesn't have");
        assert!(state.networks[1].asks_password(), "but one it can be told");
        assert!(state.networks[2].joinable(), "open");
        // The saved network behind one in range, to forget.
        assert_eq!((known(&objects, &home).as_deref(), known(&objects, &cafe)), (Some("/k/1"), None));
    }

    #[test]
    fn a_plugged_in_port_counts_and_a_radio_or_a_bridge_doesnt() {
        let root = std::env::temp_dir().join(format!("lntrn-status-net-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let port = |name: &str, files: &[(&str, &str)], dirs: &[&str]| {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            for (f, t) in files {
                fs::write(dir.join(f), format!("{t}\n")).unwrap();
            }
            for d in dirs {
                fs::create_dir_all(dir.join(d)).unwrap();
            }
        };
        port("wlan0", &[("type", "1"), ("carrier", "1"), ("operstate", "up")], &["device", "wireless"]);
        port("br0", &[("type", "1"), ("carrier", "1"), ("operstate", "up")], &[]);
        port("enp5s0", &[("type", "1"), ("carrier", "0"), ("operstate", "down")], &["device"]);
        assert!(!wired(&root));
        port("enp5s0", &[("carrier", "1"), ("operstate", "up")], &[]);
        assert!(wired(&root));
    }
}

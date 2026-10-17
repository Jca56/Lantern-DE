//! What BlueZ says, read into a [`BluetoothState`]: the first adapter and
//! what phones see it as, its paired devices (connected first, then by
//! name) with their batteries, and the devices in range that aren't
//! paired, nearest first. A device in range is one BlueZ hears now (it
//! has a signal); one with no name of its own (a beacon, a stranger's
//! tag) is left out.

use lntrn_dbus::Value;

use super::{ADAPTER, BATTERY, BluetoothState, DEVICE, Device};
use crate::bus::{self, Objects};

/// OBEX Object Push: a device that has it takes files.
const OBJECT_PUSH: &str = "00001105-0000-1000-8000-00805f9b34fb";
/// Devices in range shown, at most.
const NEARBY_MAX: usize = 24;

/// The first adapter, by path.
pub(super) fn adapter(objects: &Objects) -> Option<String> {
    let mut adapters: Vec<&String> = objects.iter().filter(|(_, i)| i.contains_key(ADAPTER)).map(|(p, _)| p).collect();
    adapters.sort();
    adapters.first().map(|p| (*p).clone())
}

/// What BlueZ's objects say now.
pub(super) fn read(objects: &Objects) -> BluetoothState {
    let Some(path) = adapter(objects) else { return BluetoothState::default() };
    let a = &objects[&path][ADAPTER];
    let ours = format!("{path}/");
    let (mut devices, mut nearby) = (Vec::new(), Vec::new());
    for (p, ifaces) in objects.iter().filter(|(p, _)| p.starts_with(&ours)) {
        let Some(d) = ifaces.get(DEVICE) else { continue };
        let connected = bus::flag(d, "Connected");
        let own_name = bus::text(d, "Name").filter(|n| !n.is_empty());
        let address = bus::text(d, "Address").unwrap_or_default();
        let device = Device {
            path: p.clone(),
            name: bus::text(d, "Alias").or_else(|| own_name.clone()).unwrap_or_else(|| address.clone()),
            address,
            icon: bus::text(d, "Icon").unwrap_or_default(),
            connected,
            battery: ifaces.get(BATTERY).and_then(|b| bus::number(b, "Percentage")).map(|v| v.clamp(0, 100) as u8).filter(|_| connected),
            takes_files: d.get("UUIDs").and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|id| id.as_str().is_some_and(|id| id.eq_ignore_ascii_case(OBJECT_PUSH)))),
            rssi: bus::number(d, "RSSI").map(|v| v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16),
        };
        if bus::flag(d, "Paired") {
            devices.push(device);
        } else if own_name.is_some() && device.rssi.is_some() {
            nearby.push(device);
        }
    }
    let by_name = |a: &Device, b: &Device| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| by_name(a, b)));
    nearby.sort_by(|a, b| b.rssi.cmp(&a.rssi).then_with(|| by_name(a, b)));
    nearby.truncate(NEARBY_MAX);
    let name = bus::text(a, "Alias").or_else(|| bus::text(a, "Name")).unwrap_or_default();
    BluetoothState { present: true, powered: bus::flag(a, "Powered"), name, devices, nearby, ..BluetoothState::default() }
}

/// The device at `path`'s name, as `state` knows it, or its address read
/// from the path (`dev_AA_BB_…`).
pub(super) fn name_of(state: &BluetoothState, path: &str) -> String {
    if let Some(d) = state.devices.iter().chain(&state.nearby).find(|d| d.path == path) {
        return d.name.clone();
    }
    path.rsplit_once("/dev_").map_or_else(|| "A device".to_owned(), |(_, mac)| mac.replace('_', ":"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, Value)]) -> bus::Props {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    }

    #[test]
    fn paired_devices_and_the_ones_in_range_read_apart() {
        let s = |v: &str| Value::String(v.to_owned());
        let mut objects: Objects = HashMap::new();
        objects.insert("/org/bluez/hci0".into(), HashMap::from([(ADAPTER.to_owned(), props(&[("Powered", Value::Bool(true)), ("Alias", s("lantern"))]))]));
        let device = |name: &str, paired: bool, connected: bool, rssi: Option<i32>| {
            let mut d = props(&[("Alias", s(name)), ("Name", s(name)), ("Icon", s("audio-headset")), ("Paired", Value::Bool(paired)), ("Connected", Value::Bool(connected))]);
            if let Some(rssi) = rssi {
                d.insert("RSSI".into(), Value::Int32(rssi));
            }
            d
        };
        objects.insert("/org/bluez/hci0/dev_AA".into(), HashMap::from([(DEVICE.to_owned(), device("Keyboard", true, false, None))]));
        objects.insert(
            "/org/bluez/hci0/dev_BB".into(),
            HashMap::from([(DEVICE.to_owned(), device("Buds", true, true, Some(-50))), (BATTERY.to_owned(), props(&[("Percentage", Value::Uint32(64))]))]),
        );
        let mut phone = device("Pixel 9", false, false, Some(-60));
        phone.insert("UUIDs".into(), Value::Array(vec![s("0000110A-0000-1000-8000-00805F9B34FB"), s("00001105-0000-1000-8000-00805F9B34FB")]));
        objects.insert("/org/bluez/hci0/dev_CC".into(), HashMap::from([(DEVICE.to_owned(), phone)]));
        objects.insert("/org/bluez/hci0/dev_DD".into(), HashMap::from([(DEVICE.to_owned(), device("Speaker", false, false, Some(-40)))]));
        // Heard before but not now, and a nameless beacon.
        objects.insert("/org/bluez/hci0/dev_EE".into(), HashMap::from([(DEVICE.to_owned(), device("Old TV", false, false, None))]));
        objects.insert("/org/bluez/hci0/dev_FF".into(), HashMap::from([(DEVICE.to_owned(), props(&[("Alias", s("FF-FF")), ("RSSI", Value::Int32(-30))]))]));
        let state = read(&objects);
        assert!(state.present && state.powered && state.name == "lantern");
        let names = |list: &[Device]| list.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&state.devices), ["Buds", "Keyboard"]);
        assert_eq!(names(&state.nearby), ["Speaker", "Pixel 9"], "nearest first, only named ones heard now");
        assert_eq!((state.devices[0].battery, state.connected()), (Some(64), 1));
        assert!(state.nearby[1].takes_files && !state.nearby[0].takes_files);
        assert_eq!(read(&HashMap::new()), BluetoothState::default(), "no adapter");
        assert_eq!((name_of(&state, "/org/bluez/hci0/dev_CC"), name_of(&state, "/org/bluez/hci0/dev_12_34")), ("Pixel 9".to_owned(), "12:34".to_owned()));
    }
}

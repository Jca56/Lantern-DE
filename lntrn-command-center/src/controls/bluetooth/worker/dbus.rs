//! BlueZ state snapshot over D-Bus.
//!
//! One `org.freedesktop.DBus.ObjectManager.GetManagedObjects` call on
//! the system bus returns the adapter (`Powered` / `Discoverable`) and
//! every known device with all of its `Device1` properties — the same
//! information the worker used to assemble from `bluetoothctl show`,
//! three `bluetoothctl devices …` variants and one `bluetoothctl info`
//! per device: ten-plus process spawns per poll, ~150 ms of CPU, every
//! 5 s, forever. This is ~1 ms and no forks.
//!
//! Field mapping is kept identical to what the CLI parser produced so
//! the render / filter code (`has_real_name`, `supports_file_transfer`,
//! the detail card) doesn't notice the swap. The CLI readers in
//! `devices.rs` stay as the fallback for a bus that's unreachable.

use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::controls::bluetooth::Device;

const BLUEZ_BUS: &str = "org.bluez";
const IFACE_ADAPTER: &str = "org.bluez.Adapter1";
const IFACE_DEVICE: &str = "org.bluez.Device1";
const IFACE_BATTERY: &str = "org.bluez.Battery1";

type Props = HashMap<String, OwnedValue>;
type ObjectMap = HashMap<OwnedObjectPath, HashMap<String, Props>>;

/// Everything the periodic poll reports in one go.
pub(super) struct Snapshot {
    pub powered: bool,
    pub discoverable: bool,
    pub devices: Vec<Device>,
}

pub(super) enum SnapError {
    /// The bus answered, but bluetoothd isn't there (or rejected the
    /// call). Report "off / no devices"; nothing to reconnect.
    NoBluez,
    /// The connection itself failed — drop it and reconnect later.
    Bus,
}

/// A long-lived system-bus connection for polling BlueZ.
pub(super) struct BluezBus {
    conn: Connection,
}

impl BluezBus {
    pub(super) fn connect() -> Option<Self> {
        Connection::system().ok().map(|conn| Self { conn })
    }

    pub(super) fn snapshot(&self) -> Result<Snapshot, SnapError> {
        let reply = self
            .conn
            .call_method(
                Some(BLUEZ_BUS),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .map_err(|e| match e {
                // A D-Bus error reply (ServiceUnknown when bluetoothd is
                // down, NoReply, …) means the bus itself is fine.
                zbus::Error::MethodError(..) => SnapError::NoBluez,
                _ => SnapError::Bus,
            })?;
        let map: ObjectMap = reply.body().deserialize().map_err(|_| SnapError::Bus)?;

        let mut powered = false;
        let mut discoverable = false;
        for ifaces in map.values() {
            if let Some(a) = ifaces.get(IFACE_ADAPTER) {
                powered = bool_prop(a, "Powered");
                discoverable = bool_prop(a, "Discoverable");
                break;
            }
        }

        let mut devices: Vec<Device> = map
            .values()
            .filter_map(|ifaces| {
                let d = ifaces.get(IFACE_DEVICE)?;
                let battery = ifaces.get(IFACE_BATTERY);
                device_from_props(d, battery)
            })
            .collect();

        // Connected → paired → alphabetical. Same order the CLI path
        // produced so rows don't reshuffle between the two.
        devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then(b.paired.cmp(&a.paired))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(Snapshot {
            powered,
            discoverable,
            devices,
        })
    }
}

fn device_from_props(d: &Props, battery: Option<&Props>) -> Option<Device> {
    let mac = string_prop(d, "Address")?;
    let alias = string_prop(d, "Alias").unwrap_or_default();
    // `bluetoothctl devices` prints the Alias, which BlueZ itself falls
    // back to the dash-joined address for nameless devices — exactly
    // what `has_real_name()` expects to filter.
    let name = if !alias.is_empty() {
        alias.clone()
    } else {
        string_prop(d, "Name").unwrap_or_else(|| mac.replace(':', "-"))
    };
    // `bluetoothctl info` prints `Class: 0x00240404 (2360324)`.
    let class = u32_prop(d, "Class")
        .map(|c| format!("0x{:08x} ({})", c, c))
        .unwrap_or_default();
    let uuids = d
        .get("UUIDs")
        .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
        .map(|list| list.iter().map(|u| uuid_name(u)).collect())
        .unwrap_or_default();

    Some(Device {
        mac,
        name,
        connected: bool_prop(d, "Connected"),
        paired: bool_prop(d, "Paired"),
        alias,
        icon: string_prop(d, "Icon").unwrap_or_default(),
        class,
        address_type: string_prop(d, "AddressType").unwrap_or_default(),
        trusted: bool_prop(d, "Trusted"),
        bonded: bool_prop(d, "Bonded"),
        blocked: bool_prop(d, "Blocked"),
        battery_percent: battery.and_then(|b| u8_prop(b, "Percentage")),
        rssi: i16_prop(d, "RSSI").map(i32::from),
        uuids,
    })
}

fn string_prop(p: &Props, key: &str) -> Option<String> {
    p.get(key).and_then(|v| String::try_from(v.clone()).ok())
}

fn bool_prop(p: &Props, key: &str) -> bool {
    p.get(key)
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false)
}

fn u32_prop(p: &Props, key: &str) -> Option<u32> {
    p.get(key).and_then(|v| u32::try_from(v.clone()).ok())
}

fn u8_prop(p: &Props, key: &str) -> Option<u8> {
    p.get(key).and_then(|v| u8::try_from(v.clone()).ok())
}

fn i16_prop(p: &Props, key: &str) -> Option<i16> {
    p.get(key).and_then(|v| i16::try_from(v.clone()).ok())
}

/// Human-friendly profile name for a service UUID, matching the labels
/// `bluetoothctl info` prints (the detail card shows them, and
/// `supports_file_transfer` looks for "OBEX Object Push"). Unknown
/// Bluetooth-SIG 16-bit UUIDs show as their hex id; anything outside
/// the SIG base UUID is "Vendor specific", like the CLI.
fn uuid_name(uuid: &str) -> String {
    let u = uuid.to_ascii_lowercase();
    let Some(id) = sig_uuid16(&u) else {
        return "Vendor specific".to_string();
    };
    let name = match id {
        0x1000 => "Service Discovery Server Service Class",
        0x1001 => "Browse Group Descriptor",
        0x1101 => "Serial Port",
        0x1102 => "LAN Access Using PPP",
        0x1103 => "Dialup Networking",
        0x1104 => "IrMC Sync",
        0x1105 => "OBEX Object Push",
        0x1106 => "OBEX File Transfer",
        0x1107 => "IrMC Sync Command",
        0x1108 => "Headset",
        0x1109 => "Cordless Telephony",
        0x110a => "Audio Source",
        0x110b => "Audio Sink",
        0x110c => "A/V Remote Control Target",
        0x110d => "Advanced Audio Distribution",
        0x110e => "A/V Remote Control",
        0x110f => "A/V Remote Control Controller",
        0x1110 => "Intercom",
        0x1111 => "Fax",
        0x1112 => "Headset AG",
        0x1115 => "PANU",
        0x1116 => "NAP",
        0x1117 => "GN",
        0x1118 => "Direct Printing",
        0x111a => "Basic Imaging Profile",
        0x111b => "Imaging Responder",
        0x111e => "Handsfree",
        0x111f => "Handsfree Audio Gateway",
        0x1124 => "Human Interface Device",
        0x1125 => "Hardcopy Cable Replacement",
        0x112d => "SIM Access",
        0x112e => "Phonebook Access Client",
        0x112f => "Phonebook Access Server",
        0x1130 => "Phonebook Access",
        0x1131 => "Headset HS",
        0x1132 => "Message Access Server",
        0x1133 => "Message Notification Server",
        0x1134 => "Message Access Profile",
        0x1200 => "PnP Information",
        0x1203 => "Generic Audio",
        0x1204 => "Generic Telephony",
        0x1800 => "Generic Access Profile",
        0x1801 => "Generic Attribute Profile",
        0x180a => "Device Information",
        0x180d => "Heart Rate",
        0x180f => "Battery Service",
        0x1812 => "Human Interface Device",
        0x1813 => "Scan Parameters",
        0x1814 => "Running Speed and Cadence",
        0x1816 => "Cycling Speed and Cadence",
        0x1818 => "Cycling Power",
        0x181c => "User Data",
        0x1826 => "Fitness Machine",
        0x1843 => "Audio Input Control",
        0x1844 => "Volume Control",
        0x1845 => "Volume Offset Control",
        0x184e => "Audio Stream Control",
        0x184f => "Broadcast Audio Scan",
        0x1850 => "Published Audio Capabilities",
        0x1853 => "Common Audio",
        0x1854 => "Hearing Access",
        0x1855 => "Telephony and Media Audio",
        _ => return format!("0x{:04x}", id),
    };
    name.to_string()
}

/// If `uuid` is `0000xxxx-0000-1000-8000-00805f9b34fb` (the Bluetooth
/// SIG base), return the 16-bit `xxxx`.
fn sig_uuid16(uuid: &str) -> Option<u16> {
    const BASE_SUFFIX: &str = "-0000-1000-8000-00805f9b34fb";
    let rest = uuid.strip_suffix(BASE_SUFFIX)?;
    let hex = rest.strip_prefix("0000")?;
    if hex.len() != 4 {
        return None;
    }
    u16::from_str_radix(hex, 16).ok()
}

//! Bluetooth through BlueZ on the system bus (`org.bluez`): whether there
//! is an adapter and it is on, the paired devices, connected or not, with
//! their batteries; switching the adapter, connecting and disconnecting
//! devices.
//!
//! The thread follows BlueZ's signals and reads everything again once a
//! burst of them settles, but not for the radio chatter of a scan (a
//! device's RSSI, its advertising data), which says nothing that shows.
//! Connecting takes as long as the device does, so each connect or
//! disconnect runs on a connection and a thread of its own. Pairing new
//! devices isn't done here (that takes an agent; the Command Center has
//! the desktop's).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use lntrn_dbus::{BodyReader, Message, Value};

use crate::board::{Board, Notify};
use crate::bus::{self, Bus, CallError, Objects};
use crate::sys::{self, Bell};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
const SETTLE: Duration = Duration::from_millis(150);
/// How often it is read with nothing said, in case a signal went missing.
const REREAD: Duration = Duration::from_secs(60);
const RECONNECT: Duration = Duration::from_secs(3);
/// A device's properties that change all through a scan and show nowhere.
const CHATTER: [&str; 5] = ["RSSI", "TxPower", "ManufacturerData", "ServiceData", "AdvertisingFlags"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// BlueZ's object path: what connect and disconnect take.
    pub path: String,
    pub name: String,
    /// What kind it is, as an icon name: `audio-headset`, `input-mouse`…
    pub icon: String,
    pub connected: bool,
    /// Its battery, while connected and while it says.
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothState {
    /// BlueZ is running and has an adapter.
    pub present: bool,
    pub powered: bool,
    /// Paired devices: connected first, then by name.
    pub devices: Vec<Device>,
    /// Devices being connected or disconnected from here, by path.
    pub busy: Vec<String>,
    /// Why the last change failed, until the next works.
    pub error: Option<String>,
}

impl BluetoothState {
    pub fn connected(&self) -> usize {
        self.devices.iter().filter(|d| d.connected).count()
    }
}

enum Cmd {
    Power(bool),
}

pub struct Bluetooth {
    board: Arc<Board<BluetoothState>>,
    tx: Sender<Cmd>,
    bell: Option<Arc<Bell>>,
}

impl Bluetooth {
    /// Start following BlueZ from a thread of its own.
    pub fn start(notify: Notify) -> Bluetooth {
        let board = Board::new(BluetoothState::default(), notify);
        let (tx, rx) = channel();
        let bell = Bell::new().map(Arc::new).map_err(|e| eprintln!("[lntrn-status] bluetooth: no bell ({e}); Bluetooth isn't followed")).ok();
        if let Some(bell) = bell.clone() {
            let theirs = board.clone();
            let spawned = std::thread::Builder::new().name("bluetooth".into()).spawn(move || run(&theirs, &rx, &bell));
            if let Err(e) = spawned {
                eprintln!("[lntrn-status] bluetooth: no thread: {e}; Bluetooth isn't followed");
            }
        }
        Bluetooth { board, tx, bell }
    }

    pub fn state(&self) -> BluetoothState {
        self.board.get()
    }

    /// Switch the adapter on or off.
    pub fn set_powered(&self, on: bool) {
        self.board.update(|s| s.error = None);
        if self.tx.send(Cmd::Power(on)).is_ok()
            && let Some(bell) = &self.bell
        {
            bell.ring();
        }
    }

    /// Connect the device at `path`, or disconnect it (`on` false).
    pub fn connect(&self, path: &str, on: bool) {
        let path = path.to_owned();
        self.board.update(|s| {
            s.error = None;
            if !s.busy.contains(&path) {
                s.busy.push(path.clone());
            }
        });
        let (board, mine) = (self.board.clone(), path.clone());
        let spawned = std::thread::Builder::new().name("bluetooth-connect".into()).spawn(move || {
            let member = if on { "Connect" } else { "Disconnect" };
            let result = Bus::system().map_err(CallError::Bus).and_then(|mut b| b.call0(BLUEZ, &path, DEVICE, member));
            let error = match result {
                Ok(_) => None,
                // Still at it when the wait ran out: BlueZ's signals say
                // how it ends.
                Err(CallError::Bus(_)) => None,
                Err(e) => Some(short(&e.to_string())),
            };
            board.update(|s| {
                s.busy.retain(|p| *p != path);
                s.error = error;
            });
        });
        if let Err(e) = spawned {
            eprintln!("[lntrn-status] bluetooth: can't connect: no thread: {e}");
            self.board.update(|s| s.busy.retain(|p| *p != mine));
        }
    }
}

fn run(board: &Board<BluetoothState>, rx: &Receiver<Cmd>, bell: &Bell) {
    let mut bus: Option<Bus> = None;
    let mut reconnect_at = Instant::now();
    let mut due = Instant::now();
    let mut settle: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if bus.is_none() && now >= reconnect_at {
            reconnect_at = now + RECONNECT;
            bus = Bus::system().ok().map(|mut b| {
                b.listen(BLUEZ);
                b
            });
            due = now;
        }
        let wake = settle.map_or(due, |s| s.min(due));
        let wake = if bus.is_none() { wake.min(reconnect_at) } else { wake };
        let mut fds = vec![bell.fd()];
        fds.extend(bus.as_ref().map(Bus::fd));
        let ready = sys::wait(&fds, Some(wake.saturating_duration_since(Instant::now())));
        bell.quiet();

        for cmd in rx.try_iter() {
            let Cmd::Power(on) = cmd;
            if let Some(b) = bus.as_mut() {
                power(b, on, board);
            }
            settle = Some(Instant::now() + SETTLE);
        }
        if ready.get(1).copied().unwrap_or(false)
            && let Some(b) = bus.as_mut()
        {
            match b.drain() {
                Ok(messages) => {
                    if messages.iter().any(shows) {
                        settle.get_or_insert(Instant::now() + SETTLE);
                    }
                }
                Err(_) => {
                    bus = None;
                    board.update(|s| *s = BluetoothState { busy: s.busy.clone(), ..BluetoothState::default() });
                }
            }
        }

        let now = Instant::now();
        if now >= due || settle.is_some_and(|s| now >= s) {
            settle = None;
            due = now + REREAD;
            let Some(b) = bus.as_mut() else { continue };
            match b.objects(BLUEZ) {
                Ok(objects) => board.update(|s| {
                    let fresh = read(&objects);
                    (s.present, s.powered, s.devices) = (fresh.present, fresh.powered, fresh.devices);
                }),
                Err(CallError::Refused { .. }) => board.update(|s| *s = BluetoothState { busy: s.busy.clone(), ..BluetoothState::default() }),
                Err(CallError::Bus(_)) => bus = None,
            }
        }
    }
}

/// Switch the first adapter on or off.
fn power(bus: &mut Bus, on: bool, board: &Board<BluetoothState>) {
    let Ok(objects) = bus.objects(BLUEZ) else { return };
    let Some(adapter) = adapter(&objects) else { return };
    if let Err(e) = bus.set_bool(BLUEZ, &adapter, ADAPTER, "Powered", on) {
        let why = short(&e.to_string());
        // Held off by rfkill (a key, or airplane mode).
        let why = if why.contains("Blocked") { "Bluetooth is blocked (airplane mode?)".to_owned() } else { why };
        board.update(|s| s.error = Some(why));
    }
}

/// Whether a signal says something that shows: anything but a scan's
/// radio chatter about a device.
fn shows(message: &Message) -> bool {
    if !message.is_signal() {
        return false;
    }
    if message.member != "PropertiesChanged" {
        return true;
    }
    // (s interface, a{sv} changed, as invalidated)
    let values = BodyReader::new(&message.body, &message.signature).read_all();
    let changed = values.get(1).and_then(Value::as_dict);
    let invalidated = values.get(2).and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    invalidated || changed.is_none_or(|c| c.keys().any(|k| !CHATTER.contains(&k.as_str())))
}

/// The first adapter, by path.
fn adapter(objects: &Objects) -> Option<String> {
    let mut adapters: Vec<&String> = objects.iter().filter(|(_, i)| i.contains_key(ADAPTER)).map(|(p, _)| p).collect();
    adapters.sort();
    adapters.first().map(|p| (*p).clone())
}

/// The state from BlueZ's objects: the first adapter and its paired
/// devices.
fn read(objects: &Objects) -> BluetoothState {
    let Some(path) = adapter(objects) else { return BluetoothState::default() };
    let powered = bus::flag(&objects[&path][ADAPTER], "Powered");
    let ours = format!("{path}/");
    let mut seen = HashSet::new();
    let mut devices: Vec<Device> = objects
        .iter()
        .filter(|(p, _)| p.starts_with(&ours))
        .filter_map(|(p, ifaces)| {
            let d = ifaces.get(DEVICE)?;
            if !bus::flag(d, "Paired") || !seen.insert(p.clone()) {
                return None;
            }
            let connected = bus::flag(d, "Connected");
            let battery = ifaces.get(BATTERY).and_then(|b| bus::number(b, "Percentage")).map(|v| v.clamp(0, 100) as u8).filter(|_| connected);
            let name = bus::text(d, "Alias").or_else(|| bus::text(d, "Name")).or_else(|| bus::text(d, "Address")).unwrap_or_default();
            Some(Device { path: p.clone(), name, icon: bus::text(d, "Icon").unwrap_or_default(), connected, battery })
        })
        .collect();
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    BluetoothState { present: true, powered, devices, ..BluetoothState::default() }
}

/// BlueZ's reasons, made short.
fn short(why: &str) -> String {
    why.rsplit_once(": ").map_or(why, |(_, tail)| tail).lines().next().unwrap_or(why).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, Value)]) -> bus::Props {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    }

    #[test]
    fn paired_devices_read_connected_first_with_their_batteries() {
        let s = |v: &str| Value::String(v.to_owned());
        let mut objects: Objects = HashMap::new();
        objects.insert("/org/bluez/hci0".into(), HashMap::from([(ADAPTER.to_owned(), props(&[("Powered", Value::Bool(true))]))]));
        let device = |alias: &str, paired: bool, connected: bool| props(&[("Alias", s(alias)), ("Icon", s("audio-headset")), ("Paired", Value::Bool(paired)), ("Connected", Value::Bool(connected))]);
        objects.insert("/org/bluez/hci0/dev_AA".into(), HashMap::from([(DEVICE.to_owned(), device("Keyboard", true, false))]));
        objects.insert(
            "/org/bluez/hci0/dev_BB".into(),
            HashMap::from([(DEVICE.to_owned(), device("Buds", true, true)), (BATTERY.to_owned(), props(&[("Percentage", Value::Uint32(64))]))]),
        );
        objects.insert("/org/bluez/hci0/dev_CC".into(), HashMap::from([(DEVICE.to_owned(), device("Stranger's TV", false, false))]));
        let state = read(&objects);
        assert!(state.present && state.powered);
        let names: Vec<&str> = state.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Buds", "Keyboard"], "unpaired ones left out");
        assert_eq!((state.devices[0].battery, state.connected()), (Some(64), 1));
        assert_eq!(read(&HashMap::new()), BluetoothState::default(), "no adapter");
    }

    #[test]
    fn a_scans_chatter_doesnt_count() {
        let signal = |changed: &[(&str, Value)]| {
            let mut body = Vec::new();
            lntrn_dbus::encode_string(&mut body, DEVICE);
            let mut entries = Vec::new();
            for (k, v) in changed {
                lntrn_dbus::encode_dict_entry_sv(&mut entries, k, |b| match v {
                    Value::Bool(on) => lntrn_dbus::encode_variant_bool(b, *on),
                    _ => lntrn_dbus::encode_variant_string(b, "x"),
                });
            }
            lntrn_dbus::align_to(&mut body, 4);
            lntrn_dbus::encode_u32(&mut body, entries.len() as u32);
            lntrn_dbus::align_to(&mut body, 8);
            body.extend_from_slice(&entries);
            lntrn_dbus::align_to(&mut body, 4);
            lntrn_dbus::encode_u32(&mut body, 0);
            Message { msg_type: 4, serial: 1, reply_serial: 0, sender: String::new(), path: String::new(), member: "PropertiesChanged".into(), interface: String::new(), signature: "sa{sv}as".into(), destination: String::new(), error_name: String::new(), body, fds: Vec::new() }
        };
        assert!(!shows(&signal(&[("ManufacturerData", Value::String(String::new()))])));
        assert!(shows(&signal(&[("Connected", Value::Bool(true))])));
        assert!(shows(&signal(&[("ServiceData", Value::String(String::new())), ("Paired", Value::Bool(true))])));
    }
}

//! Bluetooth through BlueZ on the system bus (`org.bluez`): whether there
//! is an adapter and it is on, the paired devices, connected or not, with
//! their batteries, and while someone adds one, the devices in range
//! (`read`); switching the adapter, connecting and disconnecting devices,
//! pairing new ones and forgetting old ones (`pair`); files to and from
//! them, through obexd (`files`, `send`).
//!
//! The thread follows BlueZ's signals and reads everything again once a
//! burst of them settles, but not for the radio chatter of a scan (a
//! device's RSSI, its advertising data), which says nothing that shows;
//! while scanning, it reads every so often for that. It is also the
//! desktop's pairing agent (`agent`), so its questions come to the board
//! and the person's answers go back from here. Connecting and pairing
//! take as long as the device does, so each runs on a connection and a
//! thread of its own.

mod agent;
mod files;
mod pair;
mod read;
mod send;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use lntrn_dbus::{BodyReader, Message, Value, encode_string};

use crate::board::{Board, Notify};
use crate::bus::{Answer, Bus, CallError};
use crate::sys::{self, Bell};
use agent::Agent;
use files::Files;

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
const SETTLE: Duration = Duration::from_millis(150);
/// How often it is read with nothing said, in case a signal went missing;
/// and while scanning, for the devices coming and going.
const REREAD: Duration = Duration::from_secs(60);
const SCAN_READ: Duration = Duration::from_millis(1500);
const RECONNECT: Duration = Duration::from_secs(3);
/// A device's properties that change all through a scan and show nowhere.
const CHATTER: [&str; 5] = ["RSSI", "TxPower", "ManufacturerData", "ServiceData", "AdvertisingFlags"];
const CANCELED: &str = "org.bluez.Error.Canceled";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// BlueZ's object path: what connect, pair and forget take.
    pub path: String,
    /// Its hardware address (`AA:BB:CC:DD:EE:FF`): what sending a file
    /// takes.
    pub address: String,
    pub name: String,
    /// What kind it is, as an icon name: `audio-headset`, `input-mouse`…
    pub icon: String,
    pub connected: bool,
    /// Its battery, while connected and while it says.
    pub battery: Option<u8>,
    /// It takes files sent to it.
    pub takes_files: bool,
    /// Its signal while it's heard, dBm.
    pub rssi: Option<i16>,
}

/// What BlueZ asks the person, about a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// What answers name it by.
    pub id: u64,
    /// The device asking, by path, and its name.
    pub device: String,
    pub name: String,
    pub kind: Asked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Does the device show this code too?
    Confirm(u32),
    /// May it pair? (It shows no code.)
    Allow,
    /// Type this code on the device, then Enter; `typed` keys of it so
    /// far. Nothing is answered here.
    Show(String, u32),
    /// The PIN it wants, typed here (an older device's, often 0000).
    Pin,
    /// The six-digit passkey it shows, typed here.
    Passkey,
}

/// The person's answer to a [`Question`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Yes,
    No,
    Pin(String),
    Passkey(u32),
}

/// A file on its way, in or out.
#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    /// obexd's name for it (a placeholder until it has one).
    pub path: String,
    /// The other device, by address.
    pub device: String,
    pub name: String,
    /// Bytes, all of it and so far.
    pub size: u64,
    pub done: u64,
    pub incoming: bool,
    /// How it ended, and when; none while it goes.
    pub end: Option<(Ended, Instant)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Came in, and is here now.
    Saved(String),
    Sent,
    Failed(String),
    /// Stopped from here.
    Stopped,
}

/// A file a device wants to send here, waiting on a yes or no.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub id: u64,
    /// The device, by address, and what it's called.
    pub device: String,
    pub from: String,
    /// The file's name (as it would be saved), and its size in bytes.
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothState {
    /// BlueZ is running and has an adapter.
    pub present: bool,
    pub powered: bool,
    /// What other devices see this machine as.
    pub name: String,
    /// Paired devices: connected first, then by name.
    pub devices: Vec<Device>,
    /// Looking for devices, and seen by them, while someone adds one.
    pub scanning: bool,
    /// Devices in range that aren't paired, nearest first, while
    /// scanning.
    pub nearby: Vec<Device>,
    /// Devices being connected, disconnected or paired from here, by path.
    pub busy: Vec<String>,
    /// The device being paired from here.
    pub pairing: Option<String>,
    /// What BlueZ asks the person now.
    pub question: Option<Question>,
    /// A file a device wants to send here.
    pub offer: Option<Offer>,
    /// Files on their way, and the last few that ended.
    pub transfers: Vec<Transfer>,
    /// Why the last change failed, until the next works.
    pub error: Option<String>,
    /// The device the error is about, when it's one's.
    pub failed: Option<String>,
}

impl BluetoothState {
    pub fn connected(&self) -> usize {
        self.devices.iter().filter(|d| d.connected).count()
    }
}

enum Cmd {
    Power(bool),
    Scan(bool),
    Answer(u64, Reply),
    Forget(String),
}

pub struct Bluetooth {
    board: Arc<Board<BluetoothState>>,
    tx: Sender<Cmd>,
    bell: Option<Arc<Bell>>,
    files: Option<Files>,
    stops: send::Stops,
}

impl Bluetooth {
    /// Start following BlueZ from a thread of its own, and obexd from
    /// another.
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
        let files = Files::start(board.clone());
        Bluetooth { board, tx, bell, files, stops: send::Stops::default() }
    }

    pub fn state(&self) -> BluetoothState {
        self.board.get()
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok()
            && let Some(bell) = &self.bell
        {
            bell.ring();
        }
    }

    /// Switch the adapter on or off.
    pub fn set_powered(&self, on: bool) {
        self.board.update(|s| (s.error, s.failed) = (None, None));
        self.send(Cmd::Power(on));
    }

    /// Connect the device at `path`, or disconnect it (`on` false).
    pub fn connect(&self, path: &str, on: bool) {
        pair::connect(path.to_owned(), on, self.board.clone());
    }

    /// Look for devices to add, and be seen by them; or stop.
    pub fn scan(&self, on: bool) {
        self.board.update(|s| s.scanning = on && s.powered);
        self.send(Cmd::Scan(on));
    }

    /// Pair the device in range at `path`, then trust and connect it.
    pub fn pair(&self, path: &str) {
        pair::pair(path.to_owned(), self.board.clone());
    }

    /// Answer the question `id` BlueZ asked.
    pub fn answer(&self, id: u64, reply: Reply) {
        self.board.update(|s| {
            if s.question.as_ref().is_some_and(|q| q.id == id) {
                s.question = None;
            }
        });
        self.send(Cmd::Answer(id, reply));
    }

    /// Unpair the device at `path`.
    pub fn forget(&self, path: &str) {
        self.send(Cmd::Forget(path.to_owned()));
    }

    /// Send `file` to the device at `address`.
    pub fn send_file(&self, address: &str, file: PathBuf) {
        send::send(address.to_owned(), file, self.board.clone(), self.stops.clone());
    }

    /// Stop the transfer at `path`, coming in or going out.
    pub fn stop_transfer(&self, path: &str) {
        let incoming = self.board.get().transfers.iter().any(|t| t.path == path && t.incoming);
        match (&self.files, incoming) {
            (Some(files), true) => files.stop(path),
            (_, false) => _ = self.stops.lock().map(|mut s| s.insert(path.to_owned())),
            (None, true) => {}
        }
    }

    /// Take the file offered as `id`, or turn it down.
    pub fn take_file(&self, id: u64, yes: bool) {
        self.board.update(|s| s.offer = None);
        if let Some(files) = &self.files {
            files.take(id, yes);
        }
    }
}

fn run(board: &Board<BluetoothState>, rx: &Receiver<Cmd>, bell: &Bell) {
    let mut bus: Option<Bus> = None;
    let mut agent = Agent::default();
    // The agent is BlueZ's (until BlueZ starts over); someone is adding.
    let (mut registered, mut scanning) = (false, false);
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
            (registered, due) = (false, now);
        }
        let wake = settle.map_or(due, |s| s.min(due));
        let wake = if bus.is_none() { wake.min(reconnect_at) } else { wake };
        let mut fds = vec![bell.fd()];
        fds.extend(bus.as_ref().map(Bus::fd));
        let ready = sys::wait(&fds, Some(wake.saturating_duration_since(Instant::now())));
        bell.quiet();

        for cmd in rx.try_iter() {
            let Some(b) = bus.as_mut() else { continue };
            match cmd {
                Cmd::Power(on) => power(b, on, board),
                Cmd::Scan(on) => {
                    scanning = on;
                    discover(b, on);
                }
                Cmd::Answer(id, reply) => {
                    let yes = reply == Reply::Yes;
                    let asked = board.get().question.filter(|q| q.id == id);
                    if let Some((serial, sender, answer)) = agent.answer(id, reply) {
                        b.answer(serial, &sender, answer);
                    }
                    // Paired from its end with a yes here: trusted, so it
                    // comes back on its own.
                    if let Some(q) = asked.filter(|q| yes && board.get().pairing.as_deref() != Some(q.device.as_str())) {
                        let _ = b.set_bool(BLUEZ, &q.device, DEVICE, "Trusted", true);
                    }
                }
                Cmd::Forget(path) => forget(b, &path, board),
            }
            settle = Some(Instant::now() + SETTLE);
        }
        if ready.get(1).copied().unwrap_or(false)
            && let Some(b) = bus.as_mut()
        {
            match b.drain() {
                Ok(messages) => {
                    for msg in &messages {
                        if msg.is_method_call() {
                            let taken = agent.take(msg, &board.get());
                            if let Some((serial, sender)) = taken.stale {
                                b.answer(serial, &sender, Answer::Refuse(CANCELED, "Asked again"));
                            }
                            if let Some(now) = taken.now {
                                b.answer(msg.serial, &msg.sender, now);
                            }
                            if let Some(question) = taken.question {
                                board.update(|s| s.question = question);
                            }
                        } else if let Some(up) = bluez_owner(msg) {
                            // BlueZ started over (or went): it forgot us.
                            agent.forget();
                            board.update(|s| s.question = None);
                            (registered, due) = (false, if up { Instant::now() } else { due });
                        }
                    }
                    if messages.iter().any(shows) {
                        settle.get_or_insert(Instant::now() + SETTLE);
                    }
                }
                Err(_) => {
                    bus = None;
                    agent.forget();
                    board.update(|s| *s = BluetoothState { busy: s.busy.clone(), transfers: s.transfers.clone(), offer: s.offer.clone(), ..BluetoothState::default() });
                }
            }
        }

        let now = Instant::now();
        if now >= due || settle.is_some_and(|s| now >= s) {
            settle = None;
            due = now + if scanning { SCAN_READ } else { REREAD };
            let Some(b) = bus.as_mut() else { continue };
            if !registered {
                registered = agent.register(b).is_ok();
                if registered && scanning {
                    discover(b, true);
                }
            }
            match b.objects(BLUEZ) {
                Ok(objects) => board.update(|s| {
                    let fresh = read::read(&objects);
                    (s.present, s.powered, s.name, s.devices) = (fresh.present, fresh.powered, fresh.name, fresh.devices);
                    s.scanning = scanning && fresh.powered;
                    s.nearby = if s.scanning { fresh.nearby } else { Vec::new() };
                }),
                Err(CallError::Refused { .. }) => board.update(|s| *s = BluetoothState { busy: s.busy.clone(), transfers: s.transfers.clone(), ..BluetoothState::default() }),
                Err(CallError::Bus(_)) => bus = None,
            }
        }
    }
}

/// Whether `msg` says BlueZ came (`true`) or went (`false`).
fn bluez_owner(msg: &Message) -> Option<bool> {
    if !msg.is_signal() || msg.member != "NameOwnerChanged" {
        return None;
    }
    let args = BodyReader::new(&msg.body, &msg.signature).read_all();
    (args.first().and_then(Value::as_str) == Some(BLUEZ)).then(|| args.get(2).and_then(Value::as_str).is_some_and(|new| !new.is_empty()))
}

/// Switch the first adapter on or off.
fn power(bus: &mut Bus, on: bool, board: &Board<BluetoothState>) {
    let Ok(objects) = bus.objects(BLUEZ) else { return };
    let Some(adapter) = read::adapter(&objects) else { return };
    if let Err(e) = bus.set_bool(BLUEZ, &adapter, ADAPTER, "Powered", on) {
        let why = pair::short(&e.to_string());
        // Held off by rfkill (a key, or airplane mode).
        let why = if why.contains("Blocked") { "Bluetooth is blocked (airplane mode?)".to_owned() } else { why };
        board.update(|s| (s.error, s.failed) = (Some(why), None));
    }
}

/// Look for devices and be seen by them, or stop: BlueZ ends both by
/// itself if we go (the looking with our connection, the being seen
/// after its timeout).
fn discover(bus: &mut Bus, on: bool) {
    let Ok(objects) = bus.objects(BLUEZ) else { return };
    let Some(adapter) = read::adapter(&objects) else { return };
    let _ = bus.call0(BLUEZ, &adapter, ADAPTER, if on { "StartDiscovery" } else { "StopDiscovery" });
    let _ = bus.set_bool(BLUEZ, &adapter, ADAPTER, "Discoverable", on);
}

/// Unpair the device at `path`.
fn forget(bus: &mut Bus, path: &str, board: &Board<BluetoothState>) {
    let Ok(objects) = bus.objects(BLUEZ) else { return };
    let Some(adapter) = read::adapter(&objects) else { return };
    let mut body = Vec::new();
    encode_string(&mut body, path);
    if let Err(e) = bus.call(BLUEZ, &adapter, ADAPTER, "RemoveDevice", "o", &body) {
        board.update(|s| (s.error, s.failed) = (Some(pair::short(&e.to_string())), Some(path.to_owned())));
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn bluez_coming_and_going_is_heard() {
        let owner = |name: &str, new: &str| {
            let mut body = Vec::new();
            for s in [name, ":1.1", new] {
                lntrn_dbus::encode_string(&mut body, s);
            }
            Message { msg_type: 4, serial: 1, reply_serial: 0, sender: "org.freedesktop.DBus".into(), path: String::new(), member: "NameOwnerChanged".into(), interface: String::new(), signature: "sss".into(), destination: String::new(), error_name: String::new(), body, fds: Vec::new() }
        };
        assert_eq!((bluez_owner(&owner(BLUEZ, ":1.8")), bluez_owner(&owner(BLUEZ, ""))), (Some(true), Some(false)));
        assert_eq!(bluez_owner(&owner("net.connman.iwd", ":1.8")), None);
    }
}

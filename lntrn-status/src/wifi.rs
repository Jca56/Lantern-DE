//! WiFi through iwd on the system bus (`net.connman.iwd`), which both
//! Lantern machines run: whether there is a WiFi device and it is on, the
//! network in use and its signal, the networks in range; turning it on and
//! off, scanning, joining a known or open network, leaving one. Whether a
//! cable is plugged into a wired port, too, from sysfs, read again when
//! the kernel says a link changed.
//!
//! The thread follows iwd's own signals (a property changed, a network
//! came or went, iwd itself started or stopped) and reads everything again
//! once a burst of them settles; besides, every so often for the signal
//! strength, which no signal carries. Nothing it calls ever starts iwd
//! (`FLAG_NO_AUTO_START`): one stopped by the init system stays stopped.
//! Joining waits on iwd for as long as connecting takes, so it runs on a
//! connection and a thread of its own.
//!
//! Networks that need a password iwd doesn't know yet can't be joined
//! from here yet (that takes an agent to answer iwd's question).

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use lntrn_dbus::Value;

use crate::board::{Board, Notify};
use crate::bus::{self, Bus, CallError, Objects};
use crate::sys::{self, Bell, Netlink};

const IWD: &str = "net.connman.iwd";
const DEVICE: &str = "net.connman.iwd.Device";
const STATION: &str = "net.connman.iwd.Station";
const NETWORK: &str = "net.connman.iwd.Network";
/// Between a signal and reading everything again: a scan's results come
/// as dozens of them.
const SETTLE: Duration = Duration::from_millis(150);
/// How often it is read with nothing said: connected (for the signal),
/// while someone looks at the list, and otherwise.
const CONNECTED_READ: Duration = Duration::from_secs(15);
const ATTENDED_READ: Duration = Duration::from_secs(5);
const IDLE_READ: Duration = Duration::from_secs(60);
/// Between attempts to open the system bus again.
const RECONNECT: Duration = Duration::from_secs(3);

/// What a network asks to join it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Security {
    #[default]
    Open,
    /// A password (WPA2, WPA3).
    Psk,
    /// A company's login (802.1X).
    Enterprise,
    Wep,
}

impl Security {
    fn from_type(kind: &str) -> Security {
        match kind {
            "psk" => Security::Psk,
            "8021x" => Security::Enterprise,
            "wep" => Security::Wep,
            _ => Security::Open,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Network {
    /// iwd's object path: what [`Wifi::join`] takes.
    pub path: String,
    pub name: String,
    pub security: Security,
    /// iwd has joined it before and keeps its password.
    pub known: bool,
    pub connected: bool,
    /// 0 to 100.
    pub strength: u32,
}

impl Network {
    /// It can be joined from here: iwd knows it, or it asks nothing.
    pub fn joinable(&self) -> bool {
        self.known || self.security == Security::Open
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    /// No WiFi device, or it is switched off.
    #[default]
    Off,
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiState {
    /// iwd is running and has a WiFi device.
    pub present: bool,
    /// The device is switched on.
    pub powered: bool,
    pub link: Link,
    /// The network in use (or being joined).
    pub ssid: Option<String>,
    /// The network in use's signal, 0 to 100.
    pub strength: u32,
    pub scanning: bool,
    /// In range, the one in use first, then by signal.
    pub networks: Vec<Network>,
    /// A cable is plugged in and its link is up.
    pub wired: bool,
    /// The network being joined from here, by path.
    pub joining: Option<String>,
    /// Why the last join or switch failed, until the next works.
    pub error: Option<String>,
}

impl WifiState {
    /// The signal in bars, 0 to 4, for an icon.
    pub fn bars(&self) -> u32 {
        match self.strength {
            0..=19 => 1,
            20..=44 => 2,
            45..=69 => 3,
            _ => 4,
        }
        .min(if self.link == Link::Connected { 4 } else { 0 })
    }
}

enum Cmd {
    Power(bool),
    Scan,
    Join(String),
    Leave,
    Attend(bool),
}

pub struct Wifi {
    board: Arc<Board<WifiState>>,
    tx: Sender<Cmd>,
    bell: Option<Arc<Bell>>,
}

impl Wifi {
    /// Start following iwd from a thread of its own.
    pub fn start(notify: Notify) -> Wifi {
        let board = Board::new(WifiState { wired: wired(Path::new("/sys/class/net")), ..WifiState::default() }, notify);
        let (tx, rx) = channel();
        let bell = match Bell::new() {
            Ok(bell) => Some(Arc::new(bell)),
            Err(e) => {
                eprintln!("[lntrn-status] wifi: no bell ({e}); WiFi isn't followed");
                None
            }
        };
        if let Some(bell) = bell.clone() {
            let theirs = board.clone();
            let spawned = std::thread::Builder::new().name("wifi".into()).spawn(move || run(&theirs, &rx, &bell));
            if let Err(e) = spawned {
                eprintln!("[lntrn-status] wifi: no thread: {e}; WiFi isn't followed");
            }
        }
        Wifi { board, tx, bell }
    }

    pub fn state(&self) -> WifiState {
        self.board.get()
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok()
            && let Some(bell) = &self.bell
        {
            bell.ring();
        }
    }

    /// Switch the WiFi device on or off.
    pub fn set_powered(&self, on: bool) {
        self.send(Cmd::Power(on));
    }

    /// Look for networks now.
    pub fn scan(&self) {
        self.send(Cmd::Scan);
    }

    /// Join the network at `path` (one [`Network::joinable`]).
    pub fn join(&self, path: &str) {
        self.board.update(|s| (s.joining, s.error) = (Some(path.to_owned()), None));
        self.send(Cmd::Join(path.to_owned()));
    }

    /// Leave the network in use.
    pub fn leave(&self) {
        self.send(Cmd::Leave);
    }

    /// Someone looks at the list now (or no longer does): scan, and read
    /// often meanwhile.
    pub fn attend(&self, on: bool) {
        self.send(Cmd::Attend(on));
    }
}

fn run(board: &Arc<Board<WifiState>>, rx: &Receiver<Cmd>, bell: &Bell) {
    let links = Netlink::links().ok();
    let mut bus: Option<Bus> = None;
    let mut reconnect_at = Instant::now();
    let mut attended = false;
    // Read now; read again then; read once the signals settle.
    let mut due = Instant::now();
    let mut settle: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if bus.is_none() && now >= reconnect_at {
            reconnect_at = now + RECONNECT;
            bus = Bus::system().ok().map(|mut b| {
                b.listen(IWD);
                b
            });
            due = now;
        }
        let wake = settle.map_or(due, |s| s.min(due));
        let wake = if bus.is_none() { wake.min(reconnect_at) } else { wake };
        // The bell first, then the bus and the links where there are.
        let mut fds = vec![bell.fd()];
        let bus_at = bus.as_ref().map(|b| {
            fds.push(b.fd());
            fds.len() - 1
        });
        let links_at = links.as_ref().map(|nl| {
            fds.push(nl.fd());
            fds.len() - 1
        });
        let ready = sys::wait(&fds, Some(wake.saturating_duration_since(Instant::now())));
        let is_ready = |at: Option<usize>| at.is_some_and(|i| ready[i]);
        bell.quiet();

        for cmd in rx.try_iter() {
            match cmd {
                Cmd::Attend(on) => {
                    attended = on;
                    if on {
                        settle = Some(Instant::now());
                        if let Some(b) = bus.as_mut() {
                            scan(b, board);
                        }
                    }
                }
                cmd => {
                    if let Some(b) = bus.as_mut() {
                        act(b, cmd, board);
                    }
                    settle = Some(Instant::now() + SETTLE);
                }
            }
        }
        if is_ready(bus_at) && let Some(b) = bus.as_mut() {
            match b.drain() {
                Ok(messages) => {
                    if messages.iter().any(|m| m.is_signal()) {
                        settle.get_or_insert(Instant::now() + SETTLE);
                    }
                }
                Err(_) => {
                    bus = None;
                    board.update(|s| *s = WifiState { wired: s.wired, ..WifiState::default() });
                }
            }
        }
        if let Some(nl) = &links
            && is_ready(links_at)
            && !nl.take().is_empty()
        {
            let wired = wired(Path::new("/sys/class/net"));
            board.update(|s| s.wired = wired);
        }

        let now = Instant::now();
        if now >= due || settle.is_some_and(|s| now >= s) {
            settle = None;
            let Some(b) = bus.as_mut() else {
                due = now + IDLE_READ;
                continue;
            };
            match snapshot(b) {
                Ok(mut fresh) => board.update(|s| {
                    // What only we know: the cable, our join, its failure.
                    (fresh.wired, fresh.joining, fresh.error) = (s.wired, s.joining.clone(), s.error.clone());
                    *s = fresh;
                }),
                Err(CallError::Refused(_)) => board.update(|s| *s = WifiState { wired: s.wired, ..WifiState::default() }),
                Err(CallError::Bus(_)) => bus = None,
            }
            let connected = board.get().link == Link::Connected;
            due = Instant::now() + if attended { ATTENDED_READ } else if connected { CONNECTED_READ } else { IDLE_READ };
        }
    }
}

/// Do what the UI asked.
fn act(bus: &mut Bus, cmd: Cmd, board: &Arc<Board<WifiState>>) {
    if let Cmd::Join(network) = cmd {
        join(network, board.clone());
        return;
    }
    let Ok(objects) = bus.objects(IWD) else { return };
    let Some(path) = device_path(&objects) else { return };
    let result = match cmd {
        Cmd::Power(on) => bus.set_bool(IWD, &path, DEVICE, "Powered", on),
        Cmd::Scan => {
            scan(bus, board);
            Ok(())
        }
        Cmd::Leave => bus.call0(IWD, &path, STATION, "Disconnect").map(|_| ()),
        Cmd::Join(_) | Cmd::Attend(_) => Ok(()),
    };
    if let Err(e) = result {
        board.update(|s| s.error = Some(e.to_string()));
    }
}

/// Ask for a scan; iwd saying it is busy scanning already is as good.
fn scan(bus: &mut Bus, board: &Board<WifiState>) {
    let Ok(objects) = bus.objects(IWD) else { return };
    if let Some(path) = device_path(&objects).filter(|p| objects[p].contains_key(STATION)) {
        let _ = bus.call0(IWD, &path, STATION, "Scan");
        board.update(|s| s.scanning = true);
    }
}

/// Join `network` on a connection and a thread of its own: connecting
/// takes as long as it takes, and the main thread follows iwd meanwhile.
fn join(network: String, board: Arc<Board<WifiState>>) {
    let spawned = std::thread::Builder::new().name("wifi-join".into()).spawn(move || {
        let result = Bus::system().map_err(CallError::Bus).and_then(|mut b| b.call0(IWD, &network, NETWORK, "Connect"));
        let error = match result {
            Ok(_) => None,
            // Still connecting when the reply's wait ran out: iwd's own
            // signals say how it ends.
            Err(CallError::Bus(e)) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => None,
            Err(e) => Some(short(&e.to_string())),
        };
        board.update(|s| {
            if s.joining.as_deref() == Some(network.as_str()) {
                (s.joining, s.error) = (None, error);
            }
        });
    });
    if let Err(e) = spawned {
        eprintln!("[lntrn-status] wifi: can't join: no thread: {e}");
    }
}

/// iwd's reasons, made short: `net.connman.iwd.Failed: Operation failed`
/// reads as "Operation failed".
fn short(why: &str) -> String {
    why.rsplit_once(": ").map_or(why, |(_, tail)| tail).lines().next().unwrap_or(why).to_owned()
}

/// The WiFi device: the first by path.
fn device_path(objects: &Objects) -> Option<String> {
    let mut devices: Vec<&String> = objects.iter().filter(|(_, ifaces)| ifaces.contains_key(DEVICE)).map(|(p, _)| p).collect();
    devices.sort();
    devices.first().map(|p| (*p).clone())
}

/// Everything iwd says now.
fn snapshot(bus: &mut Bus) -> Result<WifiState, CallError> {
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

/// Whether a physical wired port under `root` (`/sys/class/net`) has a
/// cable in and its link up: not WiFi, not a bridge, tunnel or the like.
fn wired(root: &Path) -> bool {
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
        assert!(state.networks[2].joinable(), "open");
    }

    #[test]
    fn iwds_reasons_read_short() {
        assert_eq!(short("net.connman.iwd.Failed: Operation failed"), "Operation failed");
        assert_eq!(short("No agent"), "No agent");
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

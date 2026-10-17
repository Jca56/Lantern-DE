//! WiFi through iwd on the system bus (`net.connman.iwd`), which both
//! Lantern machines run: whether there is a WiFi device and it is on, the
//! network in use and its signal, the networks in range; turning it on and
//! off, scanning, joining a network (telling iwd the password it asks
//! for, or finding a hidden one by its name), leaving one, forgetting a
//! saved one. Whether a cable is plugged into a wired port, too, from
//! sysfs, read again when the kernel says a link changed (`read`).
//!
//! The thread follows iwd's own signals (a property changed, a network
//! came or went, iwd itself started or stopped) and reads everything again
//! once a burst of them settles; besides, every so often for the signal
//! strength, which no signal carries. Nothing it calls ever starts iwd
//! (`FLAG_NO_AUTO_START`): one stopped by the init system stays stopped.
//! Joining waits on iwd for as long as connecting takes, so it runs on a
//! connection and a thread of its own, iwd's agent while it waits when
//! there is a password to tell it (`join`). A company login (802.1X)
//! takes a provisioning file iwd reads, so it isn't joined from here.

mod join;
mod read;

pub use join::fits;

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::board::{Board, Notify};
use crate::bus::{Bus, CallError, Objects};
use crate::secret::Secret;
use crate::sys::{self, Bell, Netlink};

const IWD: &str = "net.connman.iwd";
const DEVICE: &str = "net.connman.iwd.Device";
const STATION: &str = "net.connman.iwd.Station";
const NETWORK: &str = "net.connman.iwd.Network";
const KNOWN: &str = "net.connman.iwd.KnownNetwork";
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
    /// It can be joined with a click: iwd knows it, or it asks nothing.
    pub fn joinable(&self) -> bool {
        self.known || self.security == Security::Open
    }

    /// It needs a password iwd doesn't have yet, and can be told one here
    /// ([`Wifi::join_with_password`]): a WPA network's.
    pub fn asks_password(&self) -> bool {
        !self.known && self.security == Security::Psk
    }
}

/// What is being joined from here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A network in range, by iwd's path.
    Network(String),
    /// A hidden network, by its name.
    Hidden(String),
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
    /// What is being joined from here.
    pub joining: Option<Target>,
    /// Why the last join or switch failed, until the next works.
    pub error: Option<String>,
    /// The join that `error` is about, when it is a join's.
    pub failed: Option<Target>,
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
    Join(Target, Option<Secret>),
    Leave,
    /// The saved network behind the network in range at this path.
    Forget(String),
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
        let board = Board::new(WifiState { wired: read::wired(Path::new("/sys/class/net")), ..WifiState::default() }, notify);
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
        self.start_join(Target::Network(path.to_owned()), None);
    }

    /// Join the network at `path` with `password` (one that [`fits`]),
    /// which iwd keeps once it works. What is held of it here is wiped
    /// once iwd has it.
    pub fn join_with_password(&self, path: &str, password: String) {
        self.start_join(Target::Network(path.to_owned()), Some(Secret::new(password)));
    }

    /// Join the hidden network called `name`, with `password` (empty for
    /// an open one).
    pub fn join_hidden(&self, name: &str, password: String) {
        let secret = (!password.is_empty()).then(|| Secret::new(password));
        self.start_join(Target::Hidden(name.to_owned()), secret);
    }

    fn start_join(&self, target: Target, secret: Option<Secret>) {
        self.board.update(|s| (s.joining, s.error, s.failed) = (Some(target.clone()), None, None));
        self.send(Cmd::Join(target, secret));
    }

    /// Forget the saved network behind the one in range at `path`: iwd
    /// drops its password, and leaves it when it is in use.
    pub fn forget(&self, path: &str) {
        self.send(Cmd::Forget(path.to_owned()));
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
            let wired = read::wired(Path::new("/sys/class/net"));
            board.update(|s| s.wired = wired);
        }

        let now = Instant::now();
        if now >= due || settle.is_some_and(|s| now >= s) {
            settle = None;
            let Some(b) = bus.as_mut() else {
                due = now + IDLE_READ;
                continue;
            };
            match read::snapshot(b) {
                Ok(mut fresh) => board.update(|s| {
                    // What only we know: the cable, our join, its failure.
                    (fresh.wired, fresh.joining, fresh.error, fresh.failed) = (s.wired, s.joining.clone(), s.error.clone(), s.failed.clone());
                    *s = fresh;
                }),
                Err(CallError::Refused { .. }) => board.update(|s| *s = WifiState { wired: s.wired, ..WifiState::default() }),
                Err(CallError::Bus(_)) => bus = None,
            }
            let connected = board.get().link == Link::Connected;
            due = Instant::now() + if attended { ATTENDED_READ } else if connected { CONNECTED_READ } else { IDLE_READ };
        }
    }
}

/// Do what the UI asked.
fn act(bus: &mut Bus, cmd: Cmd, board: &Arc<Board<WifiState>>) {
    if let Cmd::Join(target, secret) = cmd {
        join::start(target, secret, board.clone());
        return;
    }
    let Ok(objects) = bus.objects(IWD) else { return };
    let result = match (cmd, read::device_path(&objects)) {
        (Cmd::Forget(network), _) => forget(bus, &objects, &network),
        (_, None) => return,
        (Cmd::Power(on), Some(path)) => bus.set_bool(IWD, &path, DEVICE, "Powered", on),
        (Cmd::Scan, Some(_)) => {
            scan(bus, board);
            Ok(())
        }
        (Cmd::Leave, Some(path)) => bus.call0(IWD, &path, STATION, "Disconnect").map(|_| ()),
        (Cmd::Join(..) | Cmd::Attend(_), Some(_)) => Ok(()),
    };
    if let Err(e) = result {
        board.update(|s| (s.error, s.failed) = (Some(e.to_string()), None));
    }
}

/// Forget the saved network behind the one in range at `network`.
fn forget(bus: &mut Bus, objects: &Objects, network: &str) -> Result<(), CallError> {
    match read::known(objects, network) {
        Some(known) => bus.call0(IWD, &known, KNOWN, "Forget").map(|_| ()),
        None => Ok(()),
    }
}

/// Ask for a scan; iwd saying it is busy scanning already is as good.
fn scan(bus: &mut Bus, board: &Board<WifiState>) {
    let Ok(objects) = bus.objects(IWD) else { return };
    if let Some(path) = read::device_path(&objects).filter(|p| objects[p].contains_key(STATION)) {
        let _ = bus.call0(IWD, &path, STATION, "Scan");
        board.update(|s| s.scanning = true);
    }
}

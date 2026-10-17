//! Apps' tray icons (StatusNotifierItem): Steam, Discord, Telegram and the
//! like, which keep running with no window and put an icon in the tray
//! instead. This is the desktop's tray: it takes the watcher's name on
//! the session bus, so their items register here (`host`), reads what
//! each says of itself (`item`), and fetches their menus (`menu`). The
//! UI shows [`AppsState`] and asks back through [`Apps`]: a click, the
//! wheel, a menu and a row of it chosen.
//!
//! The thread waits on its connection and is told the rest; every so
//! often it reads each item again, so one whose app died without the bus
//! saying so goes. With another tray holding the name (the Command
//! Center, running as the panel), it tries again now and then.

mod host;
mod item;
mod menu;

pub use menu::{MenuEntry, Toggle};

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::board::{Board, Notify};
use crate::bus::Bus;
use crate::sys::{self, Bell};
use host::Host;

/// Between readings of every item, and tries at the watcher's name while
/// another tray has it.
const LIVENESS: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_secs(10);

/// What an app says it's doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Active,
    /// Idle: nothing to show (its icon stays out of the tray).
    Passive,
    NeedsAttention,
}

/// An item's picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Icon {
    #[default]
    None,
    /// A name to look up: in `theme` (a folder of the app's own) first,
    /// then the icon themes.
    Named { name: String, theme: String },
    /// Pixels it sends.
    Pixmap(Pixmap),
}

/// An icon's pixels, RGBA.
#[derive(Clone, Debug, Eq)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// What it shows, in brief: one changing is a new picture.
    pub stamp: u64,
}

impl PartialEq for Pixmap {
    fn eq(&self, other: &Pixmap) -> bool {
        (self.width, self.height, self.stamp) == (other.width, other.height, other.stamp)
    }
}

/// One app's tray item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppItem {
    /// What it's asked by: its bus name and object path.
    pub key: String,
    /// Its name, as it gives it.
    pub title: String,
    /// What it says under the pointer.
    pub tooltip: String,
    pub status: Status,
    /// Its picture for its status: the attention one, needing attention.
    pub icon: Icon,
    /// A click on it shows its menu rather than doing something.
    pub item_is_menu: bool,
    pub has_menu: bool,
}

/// An item's menu, as it came for an ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    /// Whose it is, and which ask it answers ([`Apps::menu`]).
    pub key: String,
    pub ask: u64,
    pub entries: Vec<MenuEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppsState {
    /// This is the desktop's tray (it has the watcher's name).
    pub hosting: bool,
    /// The items registered, in the order they came.
    pub items: Vec<AppItem>,
    /// The last menu that came.
    pub menu: Option<Menu>,
}

enum Cmd {
    Activate(String),
    Secondary(String),
    Scroll(String, i32),
    Menu(String, u64),
    Click(String, i32),
}

pub struct Apps {
    board: Arc<Board<AppsState>>,
    tx: Sender<Cmd>,
    bell: Option<Arc<Bell>>,
    asks: AtomicU64,
}

impl Apps {
    /// Be the desktop's tray from a thread of its own.
    pub fn start(notify: Notify) -> Apps {
        let board = Board::new(AppsState::default(), notify);
        let (tx, rx) = channel();
        let bell = Bell::new().map(Arc::new).map_err(|e| eprintln!("[lntrn-status] apps: no bell ({e}); no app icons")).ok();
        if let Some(bell) = bell.clone() {
            let theirs = board.clone();
            let spawned = std::thread::Builder::new().name("tray-apps".into()).spawn(move || run(&theirs, &rx, &bell));
            if let Err(e) = spawned {
                eprintln!("[lntrn-status] apps: no thread: {e}; no app icons");
            }
        }
        Apps { board, tx, bell, asks: AtomicU64::new(1) }
    }

    pub fn state(&self) -> AppsState {
        self.board.get()
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok()
            && let Some(bell) = &self.bell
        {
            bell.ring();
        }
    }

    /// A click on the item `key`: it does its thing (shows its window),
    /// or its menu comes when it's only a menu.
    pub fn activate(&self, key: &str) {
        self.send(Cmd::Activate(key.to_owned()));
    }

    /// A middle click on it.
    pub fn secondary(&self, key: &str) {
        self.send(Cmd::Secondary(key.to_owned()));
    }

    /// The wheel over it: `notches`, up positive.
    pub fn scroll(&self, key: &str, notches: i32) {
        self.send(Cmd::Scroll(key.to_owned(), notches));
    }

    /// Ask for its menu. Returns the ask, which [`Menu::ask`] answers.
    pub fn menu(&self, key: &str) -> u64 {
        let ask = self.asks.fetch_add(1, Ordering::Relaxed);
        self.send(Cmd::Menu(key.to_owned(), ask));
        ask
    }

    /// The row `id` of its menu chosen.
    pub fn click(&self, key: &str, id: i32) {
        self.send(Cmd::Click(key.to_owned(), id));
    }
}

fn run(board: &Board<AppsState>, rx: &Receiver<Cmd>, bell: &Bell) {
    let mut bus: Option<Bus> = None;
    let mut host = Host::default();
    let mut retry = Instant::now();
    let mut liveness = Instant::now() + LIVENESS;
    loop {
        if bus.is_none() && Instant::now() >= retry {
            retry = Instant::now() + RETRY;
            bus = Bus::session().ok().and_then(|mut b| Host::start(&mut b).then_some(b));
            host = Host::default();
            board.update(|s| *s = AppsState { hosting: bus.is_some(), ..AppsState::default() });
        }
        let wake = if bus.is_some() { liveness } else { retry };
        let mut fds = vec![bell.fd()];
        fds.extend(bus.as_ref().map(Bus::fd));
        let ready = sys::wait(&fds, Some(wake.saturating_duration_since(Instant::now())));
        bell.quiet();
        let Some(b) = bus.as_mut() else {
            for _ in rx.try_iter() {}
            continue;
        };
        let mut changed = false;
        for cmd in rx.try_iter() {
            match cmd {
                Cmd::Activate(key) => {
                    // One that's only a menu shows it on a click.
                    if board.get().items.iter().any(|i| i.key == key && i.item_is_menu) {
                        host.menu(b, &key, 0);
                    } else {
                        host.activate(b, &key, "Activate");
                    }
                }
                Cmd::Secondary(key) => host.activate(b, &key, "SecondaryActivate"),
                Cmd::Scroll(key, notches) => host.scroll(b, &key, notches),
                Cmd::Menu(key, ask) => host.menu(b, &key, ask),
                Cmd::Click(key, id) => host.click(b, &key, id),
            }
        }
        if ready.get(1).copied().unwrap_or(false) {
            match b.drain() {
                Ok(messages) => {
                    for msg in &messages {
                        let (items, menu) = host.take(b, msg);
                        changed |= items;
                        if let Some(menu) = menu {
                            board.update(|s| s.menu = Some(menu));
                        }
                    }
                }
                Err(_) => {
                    bus = None;
                    board.update(|s| *s = AppsState::default());
                    continue;
                }
            }
        }
        if Instant::now() >= liveness {
            liveness = Instant::now() + LIVENESS;
            host.refresh_all(b);
        }
        if changed {
            let items = host.shown();
            board.update(|s| s.items = items);
        }
    }
}

//! The system tray: the bar's right end before the clock, left to right
//! in the order System Settings puts them (`[control_center]`'s
//! `tray_order`), at first the battery (with its percent), WiFi,
//! Bluetooth, sound, brightness, the workspace's number and Game Mode
//! (`bar`). Each shows only where it means something (no battery on a
//! desktop) and where System Settings leaves it on (`tray_*`). A click on the first
//! five opens their popup above the bar, in the overlay (`popup`); the
//! workspace's number turns to the next workspace and Game Mode flips.
//! The wheel turns the sound and the brightness up and down, as their
//! keys do, and goes through the workspaces; a middle click mutes.
//!
//! What they show comes from `lntrn-status`, a thread each, started the
//! first time its item shows, waking the loop only when it changed. A
//! device asking something of the person (to pair from its end, to send
//! a file) opens the Bluetooth popup by itself, once for each ask; a file
//! to send is picked in the file manager, from a thread.

mod bar;
pub mod glyphs;
pub mod popup;

pub use bar::{TrayPress, draw};

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_math::Rect;
use lntrn_status::{Audio, Backlight, Battery, Bluetooth, Notify, Wifi};

use crate::game_mode::GameMode;
use glyphs::Glyphs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Battery,
    Wifi,
    Bluetooth,
    Audio,
    Brightness,
    Workspace,
    GameMode,
}

impl Item {
    /// Left to right, until System Settings says otherwise.
    pub const ALL: [Item; 7] = [Item::Battery, Item::Wifi, Item::Bluetooth, Item::Audio, Item::Brightness, Item::Workspace, Item::GameMode];

    /// What `tray_order` calls it.
    pub fn word(self) -> &'static str {
        match self {
            Item::Battery => "battery",
            Item::Wifi => "wifi",
            Item::Bluetooth => "bluetooth",
            Item::Audio => "audio",
            Item::Brightness => "brightness",
            Item::Workspace => "workspace",
            Item::GameMode => "game_mode",
        }
    }

    /// Every item once, in the order `words` name them: a word that isn't
    /// one is passed over, and the ones left out follow in their own order.
    pub fn ordered<'a>(words: impl IntoIterator<Item = &'a str>) -> [Item; 7] {
        let mut order = Vec::with_capacity(Item::ALL.len());
        let named = words.into_iter().filter_map(|w| Item::ALL.into_iter().find(|i| i.word() == w));
        for item in named.chain(Item::ALL) {
            if !order.contains(&item) {
                order.push(item);
            }
        }
        order.try_into().unwrap_or(Item::ALL)
    }
}

/// Which items show, and in what order: `[control_center]`'s `tray_*`
/// keys, all on unless System Settings turned them off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayOptions {
    pub battery: bool,
    /// The percent beside the battery.
    pub battery_percent: bool,
    pub wifi: bool,
    pub bluetooth: bool,
    pub audio: bool,
    pub brightness: bool,
    pub workspace: bool,
    pub game_mode: bool,
    /// Left to right (`tray_order`).
    pub order: [Item; 7],
}

impl Default for TrayOptions {
    fn default() -> Self {
        TrayOptions { battery: true, battery_percent: true, wifi: true, bluetooth: true, audio: true, brightness: true, workspace: true, game_mode: true, order: Item::ALL }
    }
}

impl TrayOptions {
    pub fn shows(&self, item: Item) -> bool {
        match item {
            Item::Battery => self.battery,
            Item::Wifi => self.wifi,
            Item::Bluetooth => self.bluetooth,
            Item::Audio => self.audio,
            Item::Brightness => self.brightness,
            Item::Workspace => self.workspace,
            Item::GameMode => self.game_mode,
        }
    }
}

#[derive(Default)]
pub struct Tray {
    pub battery: Option<Battery>,
    pub backlight: Option<Backlight>,
    pub audio: Option<Audio>,
    pub wifi: Option<Wifi>,
    pub bluetooth: Option<Bluetooth>,
    pub game: GameMode,
    game_started: bool,
    pub glyphs: Glyphs,
    /// The item whose popup is up.
    pub popup: Option<Item>,
    /// Each item's face as last drawn, in the bar's pixels.
    faces: Vec<(Item, Rect)>,
    /// The wheel turned over an item, not yet a whole notch.
    wheel: Option<(Item, f64)>,
    /// How far the WiFi and Bluetooth lists are scrolled, pixels.
    scroll: f64,
    /// What is typed in the WiFi popup: a password, a hidden network.
    pub entry: popup::Entry,
    /// What is typed and open in the Bluetooth popup.
    pub bt: popup::BtEntry,
    /// The last of a device's asks that opened its popup by itself: a
    /// file offer's (`true`) or a question's, by id.
    called: Option<(bool, u64)>,
    /// Files picked to send, by the address they go to.
    picks: Option<(Sender<(String, PathBuf)>, Receiver<(String, PathBuf)>)>,
    waker: Option<Waker>,
}

impl Tray {
    /// The loop's waker is known: start the watchers of what shows.
    pub fn start(&mut self, waker: Waker, options: TrayOptions) {
        self.waker = Some(waker);
        self.follow(options);
    }

    /// Start the watcher of each item that shows and isn't watched yet;
    /// put away the popup of one that no longer shows.
    pub fn follow(&mut self, options: TrayOptions) {
        if self.popup.is_some_and(|item| !options.shows(item)) {
            self.close();
        }
        let Some(waker) = self.waker.clone() else { return };
        let wake = waker.clone();
        let notify: Notify = Arc::new(move || wake.wake());
        let start = |on: bool, slot: bool| on && !slot;
        if start(options.battery, self.battery.is_some()) {
            self.battery = Some(Battery::start(notify.clone()));
        }
        if start(options.wifi, self.wifi.is_some()) {
            self.wifi = Some(Wifi::start(notify.clone()));
        }
        if start(options.bluetooth, self.bluetooth.is_some()) {
            self.bluetooth = Some(Bluetooth::start(notify.clone()));
        }
        if start(options.audio, self.audio.is_some()) {
            self.audio = Some(Audio::start(notify.clone()));
        }
        if start(options.brightness, self.backlight.is_some()) {
            self.backlight = Some(Backlight::start(notify));
        }
        if start(options.game_mode, self.game_started) {
            self.game.start(waker);
            self.game_started = true;
        }
    }

    /// Where `item`'s face was last drawn on the bar.
    pub fn face(&self, item: Item) -> Option<Rect> {
        self.faces.iter().find(|(i, _)| *i == item).map(|(_, r)| *r)
    }

    /// Open `item`'s popup (closing any other), or close it when it is up.
    pub fn toggle(&mut self, item: Item) {
        if self.popup == Some(item) {
            self.close();
            return;
        }
        self.close();
        self.popup = Some(item);
        self.scroll = 0.0;
        self.attend(item, true);
    }

    pub fn close(&mut self) {
        self.entry.close();
        if self.bt.adding
            && let Some(bt) = &self.bluetooth
        {
            bt.scan(false);
        }
        self.bt.close();
        if let Some(item) = self.popup.take() {
            self.attend(item, false);
        }
    }

    /// A device asking something of the person that hasn't opened the
    /// Bluetooth popup yet: it opens now. Returns whether it did.
    pub fn calling(&mut self) -> bool {
        let Some(s) = self.bluetooth.as_ref().map(Bluetooth::state) else { return false };
        let call = s.question.as_ref().map(|q| (false, q.id)).or(s.offer.as_ref().map(|o| (true, o.id)));
        let Some(call) = call.filter(|c| self.called != Some(*c)) else { return false };
        self.called = Some(call);
        if self.popup == Some(Item::Bluetooth) {
            return false;
        }
        self.toggle(Item::Bluetooth);
        true
    }

    /// Ask the file manager for a file to send to the device at
    /// `address`, called `name`. It comes back from a thread, and goes
    /// ([`Self::follow_picks`]).
    pub fn pick_file(&mut self, address: &str, name: &str) {
        let tx = self.picks.get_or_insert_with(channel).0.clone();
        let (address, title, waker) = (address.to_owned(), format!("Send to {name}"), self.waker.clone());
        let spawned = std::thread::Builder::new().name("pick-file".into()).spawn(move || {
            let out = std::process::Command::new("lntrn-file-manager").args(["--pick", "--pick-print0", "--title", &title]).output();
            let Some(path) = out.ok().and_then(|o| first_pick(&o.stdout)) else { return };
            if tx.send((address, path)).is_ok()
                && let Some(waker) = waker
            {
                waker.wake();
            }
        });
        if let Err(e) = spawned {
            log_warn!("tray: no thread to pick a file in: {e}");
        }
    }

    /// Send the files picked since last time.
    pub fn follow_picks(&self) {
        let (Some((_, rx)), Some(bt)) = (&self.picks, &self.bluetooth) else { return };
        for (address, path) in rx.try_iter() {
            bt.send_file(&address, path);
        }
    }

    /// Tell a watcher someone looks at its popup now, or no longer does:
    /// WiFi scans meanwhile, the backlight follows the keys closely.
    fn attend(&self, item: Item, on: bool) {
        match (item, &self.wifi, &self.backlight) {
            (Item::Wifi, Some(wifi), _) => wifi.attend(on),
            (Item::Brightness, _, Some(backlight)) => backlight.attend(on),
            _ => {}
        }
    }
}

/// The first path the file manager printed: each one its bytes then a
/// NUL (one a line, from one older than that).
fn first_pick(stdout: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let end = if stdout.contains(&0) { 0 } else { b'\n' };
    stdout.split(|b| *b == end).find(|p| !p.is_empty()).map(|p| PathBuf::from(std::ffi::OsStr::from_bytes(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picked_file_is_the_first_path_printed() {
        assert_eq!(first_pick(b"/home/a/My Photo.jpg\0/home/a/b\0"), Some(PathBuf::from("/home/a/My Photo.jpg")));
        assert_eq!(first_pick(b"/home/a/old.txt\n"), Some(PathBuf::from("/home/a/old.txt")));
        assert_eq!(first_pick(b""), None, "nothing picked");
    }
}

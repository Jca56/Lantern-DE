//! The system tray: the bar's right end before the clock, left to right
//! the battery (with its percent), WiFi, Bluetooth, sound, brightness,
//! the workspace's number and Game Mode (`bar`). Each shows only where it
//! means something (no battery on a desktop) and where System Settings
//! leaves it on (`[control_center]`'s `tray_*`). A click on the first
//! five opens their popup above the bar, in the overlay (`popup`); the
//! workspace's number turns to the next workspace and Game Mode flips.
//! The wheel turns the sound and the brightness up and down, as their
//! keys do, and goes through the workspaces; a middle click mutes.
//!
//! What they show comes from `lntrn-status`, a thread each, started the
//! first time its item shows, waking the loop only when it changed.

mod bar;
pub mod glyphs;
pub mod popup;

pub use bar::{TrayPress, draw};

use std::sync::Arc;

use lntrn_app::Waker;
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
    /// Left to right.
    pub const ALL: [Item; 7] = [Item::Battery, Item::Wifi, Item::Bluetooth, Item::Audio, Item::Brightness, Item::Workspace, Item::GameMode];
}

/// Which items show: `[control_center]`'s `tray_*` keys, all on unless
/// System Settings turned them off.
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
}

impl Default for TrayOptions {
    fn default() -> Self {
        TrayOptions { battery: true, battery_percent: true, wifi: true, bluetooth: true, audio: true, brightness: true, workspace: true, game_mode: true }
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
        if let Some(item) = self.popup.take() {
            self.attend(item, false);
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

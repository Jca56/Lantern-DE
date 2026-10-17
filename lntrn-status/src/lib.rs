//! The machine's status, and the quick controls a bar or a panel offers
//! for it, each kept current by a thread of its own:
//!
//! - [`battery`]: the laptop's charge, read from sysfs again whenever the
//!   kernel says a power supply changed (a charger in or out, a percent
//!   gone), and its charge limit.
//! - [`backlight`]: the screen's brightness, and setting it.
//! - [`audio`]: the default output's volume and mute, the outputs there
//!   are, and changing them, through WirePlumber's `wpctl`; `pactl
//!   subscribe` says when anything changed, so nothing is polled.
//! - [`wifi`]: iwd over the system bus: on or off, the network in use and
//!   its signal, the networks in range; joining one (telling iwd the
//!   password it asks for, or a hidden one by name), leaving, forgetting
//!   a saved one. Whether a cable is plugged in, too.
//! - [`apps`]: apps' tray icons (StatusNotifierItem) on the session bus:
//!   this is the desktop's tray, so they register here; their names,
//!   pictures and menus, and clicks, the wheel and menu rows back to them.
//! - [`bluetooth`]: BlueZ over the system bus: on or off, the paired
//!   devices, their batteries; connecting and disconnecting them, finding
//!   and pairing new ones (the desktop's pairing agent asking the person
//!   what pairing needs), forgetting old ones; files to and from them
//!   through obexd on the session bus.
//! - [`load`]: how hard the machine works: the processors and how hot
//!   they run, the memory, the graphics cards, the busiest programs.
//!   Samplers the System Monitor drives, and [`Load`], a thread looking
//!   on a beat for a bar's mini monitors.
//!
//! Each `start` takes a [`Notify`], called from its thread whenever what
//! it shows changed (never when it didn't), so a UI sleeps until then;
//! `state` reads what it shows now. The D-Bus ones follow the daemons'
//! own signals rather than asking over and over. No crate but ours: the
//! bus is `lntrn-dbus`, NVIDIA's library is opened with `lntrn-sys`, and
//! the few libc calls std doesn't wrap are declared in `sys`.

pub mod apps;
pub mod audio;
pub mod backlight;
pub mod battery;
pub mod bluetooth;
mod board;
mod bus;
mod level;
pub mod load;
pub mod secret;
mod sys;
pub mod wifi;

pub use apps::{Apps, AppsState};
pub use audio::{Audio, AudioState};
pub use backlight::{Backlight, BacklightState};
pub use battery::{Battery, BatteryState};
pub use bluetooth::{Bluetooth, BluetoothState};
pub use board::Notify;
pub use load::{Load, LoadOptions, LoadState};
pub use wifi::{Wifi, WifiState};

#[cfg(test)]
mod here {
    use std::sync::Arc;

    /// Everything as this machine has it, read only (nothing scanned, set
    /// or played): `cargo test -p lntrn-status -- --ignored --nocapture
    /// everything_here`.
    #[test]
    #[ignore = "reads this machine's battery, backlight, sound, WiFi and Bluetooth"]
    fn everything_here_reads() {
        let notify: crate::Notify = Arc::new(|| {});
        let (battery, backlight, audio) = (crate::Battery::start(notify.clone()), crate::Backlight::start(notify.clone()), crate::Audio::start(notify.clone()));
        let (wifi, bluetooth) = (crate::Wifi::start(notify.clone()), crate::Bluetooth::start(notify));
        std::thread::sleep(std::time::Duration::from_secs(2));
        println!("{:#?}\n{:#?}\n{:#?}", battery.state(), backlight.state(), audio.state());
        println!("{:#?}\n{:#?}", wifi.state(), bluetooth.state());
    }
}

//! What the bar takes from `lantern.toml`: the desktop's look (through
//! `lntrn-kit`), its own shape, clock, auto hide and tray
//! (`[control_center]`),
//! the primary monitor
//! it lives on, and how windows glide (which docking keeps step with). A
//! thread watches the file and wakes the loop when it changes, so a bar
//! sitting still draws nothing.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_data::{Doc, toml};
use lntrn_kit::desktop::{self, Desktop};
use lntrn_sys::watch::{Event, EventKind, Watcher};

use crate::bar::Shape;
use crate::clock::{ClockOptions, DateStyle};
use crate::hide::{HideOptions, Visibility};
use crate::morph::Glide;
use crate::tray::TrayOptions;
/// How long a burst of writes to the file is let settle before it is read.
const SETTLE: Duration = Duration::from_millis(100);

/// Everything the bar follows from the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub desktop: Desktop,
    /// `[control_center]`: how tall the bar is, how round, and how far it
    /// floats from the edges.
    pub shape: Shape,
    /// `[control_center]`'s `clock_*` keys: what the clock shows.
    pub clock: ClockOptions,
    /// `[control_center]`'s `visibility` and `auto_hide_*` keys: whether
    /// it hides, and how.
    pub hide: HideOptions,
    /// `[control_center]`'s `tray_*` keys: which of the tray's items show.
    pub tray: TrayOptions,
    /// The monitor `[[monitors]]` marks `primary`, by its connector name
    /// (`DP-1`), unless it is switched off.
    pub primary: Option<String>,
    /// The compositor's maximize glide from `[animations]`: docking takes
    /// as long, eased alike, so the plate and the window land together.
    pub dock_glide: Glide,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_doc(&Doc::map())
    }
}

impl Settings {
    /// What the file says now; the defaults when it isn't there. `None`
    /// when it is there but won't read or parse (caught mid-write, say):
    /// keep what was.
    pub fn read() -> Option<Settings> {
        match std::fs::read_to_string(desktop::config_path()) {
            Ok(text) => toml::parse(&text).ok().map(|doc| Self::from_doc(&doc)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Settings::default()),
            Err(_) => None,
        }
    }

    pub fn from_doc(doc: &Doc) -> Settings {
        Settings { desktop: Desktop::from_doc(doc), shape: shape(doc), clock: clock(doc), hide: hide(doc), tray: tray(doc), primary: primary(doc), dock_glide: dock_glide(doc) }
    }
}

/// `[control_center]`: each key that is there and a number, held to its
/// range; the default for the rest.
fn shape(doc: &Doc) -> Shape {
    let d = Shape::default();
    let key = |k: &str, or: u32| doc.path(&format!("control_center.{k}")).and_then(Doc::as_i64).map_or(or, |v| v.clamp(0, i64::from(u32::MAX)) as u32);
    Shape { height: key("height", d.height), radius: key("corner_radius", d.radius), below: key("gap_below", d.below), sides: key("gap_sides", d.sides) }.clamped()
}

/// `[control_center]`'s `clock_*` keys; the default for any missing.
fn clock(doc: &Doc) -> ClockOptions {
    let d = ClockOptions::default();
    let key = |k: &str| doc.path(&format!("control_center.clock_{k}"));
    let flag = |k: &str, or: bool| key(k).and_then(Doc::as_bool).unwrap_or(or);
    ClockOptions { hour24: flag("24h", d.hour24), seconds: flag("seconds", d.seconds), ampm: flag("ampm", d.ampm), date: flag("date", d.date), style: key("date_style").and_then(Doc::as_str).map_or(d.style, DateStyle::from_word) }
}

/// `[control_center]`'s `visibility`, and the auto hide timings in
/// milliseconds held to their ranges; the default for any missing.
fn hide(doc: &Doc) -> HideOptions {
    let d = HideOptions::default();
    let key = |k: &str| doc.path(&format!("control_center.{k}"));
    let seconds = |k: &str, or: f64, (lo, hi): (i64, i64)| key(k).and_then(Doc::as_i64).map_or(or, |ms| ms.clamp(lo, hi) as f64 / 1000.0);
    HideOptions {
        visibility: key("visibility").and_then(Doc::as_str).map_or(Visibility::Always, Visibility::from_word),
        delay: seconds("auto_hide_delay_ms", d.delay, HideOptions::DELAYS_MS),
        slide: seconds("auto_hide_slide_ms", d.slide, HideOptions::SLIDES_MS),
    }
}

/// `[control_center]`'s `tray_*` switches; on unless set off.
fn tray(doc: &Doc) -> TrayOptions {
    let d = TrayOptions::default();
    let on = |k: &str, or: bool| doc.path(&format!("control_center.tray_{k}")).and_then(Doc::as_bool).unwrap_or(or);
    TrayOptions {
        battery: on("battery", d.battery),
        battery_percent: on("battery_percent", d.battery_percent),
        wifi: on("wifi", d.wifi),
        bluetooth: on("bluetooth", d.bluetooth),
        audio: on("audio", d.audio),
        brightness: on("brightness", d.brightness),
        workspace: on("workspace", d.workspace),
        game_mode: on("game_mode", d.game_mode),
    }
}

/// `[animations]` as the compositor reads it for a window-state change:
/// on unless `enabled` or `state` says otherwise, `speed` 1, `cinematic`.
fn dock_glide(doc: &Doc) -> Glide {
    let key = |k: &str| doc.path(&format!("animations.{k}"));
    let on = ["enabled", "state"].iter().all(|k| key(k).and_then(Doc::as_bool) != Some(false));
    let speed = key("speed").and_then(Doc::as_f64).unwrap_or(1.0);
    Glide::windows(on, speed, key("preset").and_then(Doc::as_str).unwrap_or("cinematic"))
}

fn primary(doc: &Doc) -> Option<String> {
    let on = |m: &&Doc| m.get("primary").and_then(Doc::as_bool) == Some(true) && m.get("enabled").and_then(Doc::as_bool) != Some(false);
    let monitor = doc.get("monitors")?.as_list()?.iter().find(on)?;
    monitor.get("name")?.as_str().filter(|n| !n.is_empty()).map(str::to_owned)
}

/// Watch `lantern.toml` from a thread of its own: raise `changed` and
/// wake the loop when it changes. System Settings writes the file whole
/// and renames it into place, so its directory is what's watched.
pub fn watch(changed: Arc<AtomicBool>, waker: Waker) {
    let path = desktop::config_path();
    let Some(dir) = path.parent().map(Path::to_path_buf) else { return };
    let mut watcher = match Watcher::new().and_then(|mut w| w.watch(&dir, false).map(|()| w)) {
        Ok(w) => w,
        Err(e) => {
            log_warn!("settings: can't watch {}: {e}; changes show after a restart", dir.display());
            return;
        }
    };
    let spawned = std::thread::Builder::new().name("lantern.toml".into()).spawn(move || {
        loop {
            if !touches(&watcher.wait(None), &path) {
                continue;
            }
            // An editor that writes in place writes in pieces: what comes
            // in meanwhile is the same change.
            std::thread::sleep(SETTLE);
            watcher.poll();
            changed.store(true, Ordering::Release);
            waker.wake();
        }
    });
    if let Err(e) = spawned {
        log_warn!("settings: no watcher thread: {e}; changes show after a restart");
    }
}

/// Whether any of `events` is about the file at `path`.
fn touches(events: &[Event], path: &Path) -> bool {
    events.iter().any(|e| e.path == path || e.kind == EventKind::Overflow || matches!(&e.kind, EventKind::Renamed { from } if from == path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse(text: &str) -> Settings {
        Settings::from_doc(&toml::parse(text).unwrap())
    }

    #[test]
    fn the_primary_monitor_is_the_one_marked_and_switched_on() {
        let two = "[[monitors]]\nname = \"HDMI-A-1\"\nprimary = false\n\n[[monitors]]\nname = \"DP-1\"\nprimary = true\nenabled = true\n";
        assert_eq!(parse(two).primary.as_deref(), Some("DP-1"));
        // Marked but switched off, marked but nameless, or none marked.
        assert_eq!(parse("[[monitors]]\nname = \"DP-1\"\nprimary = true\nenabled = false\n").primary, None);
        assert_eq!(parse("[[monitors]]\nname = \"\"\nprimary = true\n").primary, None);
        assert_eq!(parse("[[monitors]]\nname = \"eDP-1\"\n").primary, None);
        assert_eq!(Settings::default().primary, None);
    }

    #[test]
    fn the_shape_is_what_system_settings_wrote_held_to_its_ranges() {
        assert_eq!(Settings::default().shape, Shape::default());
        let set = parse("[control_center]\nheight = 90\ncorner_radius = 0\ngap_below = 12\ngap_sides = 40\n").shape;
        assert_eq!(set, Shape { height: 90, radius: 0, below: 12, sides: 40 });
        // Out of range, below zero, or not a number.
        let odd = parse("[control_center]\nheight = 500\ngap_below = -3\ngap_sides = \"wide\"\n").shape;
        assert_eq!(odd, Shape { height: 120, radius: 16, below: 0, sides: 16 });
        // The window gap no longer moves it.
        assert_eq!(parse("[window_manager]\ngap = 20\n").shape, Shape::default());
    }

    #[test]
    fn the_clock_shows_what_system_settings_chose() {
        assert_eq!(Settings::default().clock, ClockOptions::default());
        let set = parse("[control_center]\nclock_24h = true\nclock_seconds = true\nclock_ampm = false\nclock_date = false\nclock_date_style = \"long\"\n").clock;
        assert_eq!(set, ClockOptions { hour24: true, seconds: true, ampm: false, date: false, style: DateStyle::Long });
        assert_eq!(parse("[control_center]\nclock_date_style = \"???\"\n").clock.style, DateStyle::Short);
    }

    #[test]
    fn auto_hide_is_what_system_settings_chose_held_to_its_ranges() {
        assert_eq!(Settings::default().hide, HideOptions::default());
        let set = parse("[control_center]\nvisibility = \"auto_hide\"\nauto_hide_delay_ms = 1200\nauto_hide_slide_ms = 400\n").hide;
        assert_eq!(set, HideOptions { visibility: Visibility::AutoHide, delay: 1.2, slide: 0.4 });
        let odd = parse("[control_center]\nvisibility = \"?\"\nauto_hide_delay_ms = -5\nauto_hide_slide_ms = 99999\n").hide;
        assert_eq!(odd, HideOptions { visibility: Visibility::Always, delay: 0.0, slide: 1.0 });
    }

    #[test]
    fn the_tray_shows_what_system_settings_left_on() {
        assert_eq!(Settings::default().tray, TrayOptions::default());
        let set = parse("[control_center]\ntray_wifi = false\ntray_battery_percent = false\ntray_game_mode = \"no\"\n").tray;
        assert_eq!(set, TrayOptions { wifi: false, battery_percent: false, ..TrayOptions::default() }, "a word isn't a switch");
    }

    #[test]
    fn docking_glides_as_the_windows_do() {
        use crate::morph::Curve;
        assert_eq!(Settings::default().dock_glide, Glide { seconds: 1.0, curve: Curve::EaseInOutQuint });
        assert_eq!(parse("[animations]\nspeed = 2.0\npreset = \"snappy\"\n").dock_glide, Glide { seconds: 0.5, curve: Curve::EaseOutCubic });
        assert_eq!(parse("[animations]\nstate = false\n").dock_glide, Glide::NONE);
        assert_eq!(parse("[animations]\nenabled = false\nstate = true\n").dock_glide, Glide::NONE);
    }

    #[test]
    fn only_the_files_own_events_count() {
        let path = PathBuf::from("/c/lantern.toml");
        let event = |path: &str, kind| Event { path: PathBuf::from(path), kind, is_dir: false };
        assert!(touches(&[event("/c/lantern.toml", EventKind::Modified)], &path));
        // Renamed into place, or away; the kernel losing count.
        assert!(touches(&[event("/c/lantern.toml", EventKind::Renamed { from: PathBuf::from("/c/lantern.toml.part") })], &path));
        assert!(touches(&[event("/c/old.toml", EventKind::Renamed { from: path.clone() })], &path));
        assert!(touches(&[event("/c", EventKind::Overflow)], &path));
        assert!(!touches(&[event("/c/lantern.toml.part", EventKind::Modified), event("/c/other.toml", EventKind::Created)], &path));
    }
}

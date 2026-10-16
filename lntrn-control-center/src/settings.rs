//! What the bar takes from `lantern.toml`: the desktop's look (through
//! `lntrn-kit`), the window gap it floats by, the primary monitor it lives
//! on, and how windows glide (which docking keeps step with). A thread watches the file and wakes the loop when it
//! changes, so a bar sitting still draws nothing.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::log_warn;
use lntrn_data::{Doc, toml};
use lntrn_kit::desktop::{self, Desktop};
use lntrn_sys::watch::{Event, EventKind, Watcher};

use crate::morph::Glide;

/// The window gap when `lantern.toml` doesn't say: the compositor's own.
const DEFAULT_GAP: i32 = 8;
/// The most the gap may be: the compositor holds it to the same.
const MAX_GAP: i32 = 32;
/// How long a burst of writes to the file is let settle before it is read.
const SETTLE: Duration = Duration::from_millis(100);

/// Everything the bar follows from the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub desktop: Desktop,
    /// `[window_manager] gap`: what snapped windows keep from the
    /// screen's edges, and so what the bar floats by.
    pub gap: i32,
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
        let gap = doc.path("window_manager.gap").and_then(Doc::as_i64).map_or(DEFAULT_GAP, |g| g.clamp(0, MAX_GAP as i64) as i32);
        Settings { desktop: Desktop::from_doc(doc), gap, primary: primary(doc), dock_glide: dock_glide(doc) }
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
    fn the_gap_follows_the_window_manager_held_to_what_the_compositor_allows() {
        assert_eq!(Settings::default().gap, 8);
        assert_eq!(parse("[window_manager]\ngap = 12\n").gap, 12);
        assert_eq!(parse("[window_manager]\ngap = 100\n").gap, 32);
        assert_eq!(parse("[window_manager]\ngap = -3\n").gap, 0);
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

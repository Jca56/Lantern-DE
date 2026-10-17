//! The screen's backlight, from `/sys/class/backlight`: the firmware's
//! device before the platform's before a raw one (as systemd picks), and
//! setting it, through the file (Lantern's machines let the user write
//! it; the brightness keys do) or else logind's `SetBrightness`, which
//! the session may always use.
//!
//! The kernel says nothing when someone else changes it (the keys), so
//! the thread reads it: every few seconds, and often while something on
//! screen shows the level ([`Backlight::attend`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use lntrn_dbus::{encode_string, encode_u32};

use crate::board::{Board, Notify};
use crate::bus::Bus;
use crate::{level, sys};

const ROOT: &str = "/sys/class/backlight";
/// Never set below this: a screen turned all the way down looks dead.
pub const FLOOR: f64 = 0.05;
const IDLE_READ: Duration = Duration::from_secs(5);
const ATTENDED_READ: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BacklightState {
    /// The machine has a backlight to set.
    pub present: bool,
    /// 0 to 1.
    pub level: f64,
}

enum Cmd {
    Set(f64),
    Step { steps: i32, osd: bool },
    Attend(bool),
}

pub struct Backlight {
    board: Arc<Board<BacklightState>>,
    tx: Sender<Cmd>,
}

impl Backlight {
    /// The backlight as it is now, kept current from a thread of its own;
    /// a machine without one says so (`present` false).
    pub fn start(notify: Notify) -> Backlight {
        let device = pick(Path::new(ROOT));
        let board = Board::new(device.as_ref().map(Device::state).unwrap_or_default(), notify);
        let (tx, rx) = channel();
        if let Some(device) = device {
            let theirs = board.clone();
            let spawned = std::thread::Builder::new().name("backlight".into()).spawn(move || run(device, &theirs, &rx));
            if let Err(e) = spawned {
                eprintln!("[lntrn-status] backlight: no thread: {e}; it can't be set from here");
            }
        }
        Backlight { board, tx }
    }

    pub fn state(&self) -> BacklightState {
        self.board.get()
    }

    /// Set it to `level` (0 to 1), never below [`FLOOR`].
    pub fn set(&self, level: f64) {
        self.board.update(|s| s.level = level.clamp(FLOOR, 1.0));
        let _ = self.tx.send(Cmd::Set(level));
    }

    /// Turn it up (or down) by `steps` 5% steps, as the keys do; `osd`
    /// shows where it landed on Lantern's on-screen display, as they do.
    pub fn step(&self, steps: i32, osd: bool) {
        let _ = self.tx.send(Cmd::Step { steps, osd });
    }

    /// Something on screen shows the level now (or no longer does): read
    /// it often meanwhile, so it follows the keys.
    pub fn attend(&self, on: bool) {
        let _ = self.tx.send(Cmd::Attend(on));
    }
}

struct Device {
    dir: PathBuf,
    max: u64,
}

impl Device {
    fn raw(&self) -> Option<u64> {
        fs::read_to_string(self.dir.join("brightness")).ok()?.trim().parse().ok()
    }

    fn state(&self) -> BacklightState {
        let level = self.raw().map_or(0.0, |raw| (raw as f64 / self.max as f64).clamp(0.0, 1.0));
        BacklightState { present: true, level }
    }
}

/// The device to drive: firmware, then platform, then raw; by name
/// within each.
fn pick(root: &Path) -> Option<Device> {
    let rank = |dir: &Path| match fs::read_to_string(dir.join("type")).unwrap_or_default().trim() {
        "firmware" => 0,
        "platform" => 1,
        _ => 2,
    };
    let mut found: Vec<(u8, PathBuf, u64)> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter_map(|dir| {
            let max: u64 = fs::read_to_string(dir.join("max_brightness")).ok()?.trim().parse().ok()?;
            (max > 0 && dir.join("brightness").exists()).then(|| (rank(&dir), dir, max))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, dir, max)| Device { dir, max })
}

fn run(device: Device, board: &Board<BacklightState>, rx: &Receiver<Cmd>) {
    let mut attended = false;
    let mut logind: Option<Bus> = None;
    loop {
        let wait = if attended { ATTENDED_READ } else { IDLE_READ };
        let first = match rx.recv_timeout(wait) {
            Ok(cmd) => Some(cmd),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // A slider dragged sends one a pixel: only the last matters.
        let (mut target, mut steps, mut osd) = (None, 0, false);
        for cmd in first.into_iter().chain(rx.try_iter()) {
            match cmd {
                Cmd::Set(level) => (target, steps) = (Some(level), 0),
                Cmd::Step { steps: n, osd: show } => (steps, osd) = (steps + n, osd || show),
                Cmd::Attend(on) => attended = on,
            }
        }
        if steps != 0 {
            let from = target.unwrap_or_else(|| device.state().level);
            target = Some(level::step5(from, steps));
        }
        if let Some(level) = target {
            write(&device, level.clamp(FLOOR, 1.0), &mut logind);
        }
        let now = device.state();
        if osd {
            sys::osd(&["brightness", &((now.level * 100.0).round() as u32).to_string()]);
        }
        board.put(now);
    }
}

/// Set the device to `level`: by its file, else through logind.
fn write(device: &Device, level: f64, logind: &mut Option<Bus>) {
    let raw = ((level * device.max as f64).round() as u64).clamp(1, device.max);
    if fs::write(device.dir.join("brightness"), format!("{raw}\n")).is_ok() {
        return;
    }
    let name = device.dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
    if logind.is_none() {
        *logind = Bus::system().ok();
    }
    let Some(bus) = logind.as_mut() else { return };
    let mut body = Vec::new();
    encode_string(&mut body, "backlight");
    encode_string(&mut body, &name);
    encode_u32(&mut body, u32::try_from(raw).unwrap_or(u32::MAX));
    if let Err(e) = bus.call("org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session", "SetBrightness", "ssu", &body) {
        eprintln!("[lntrn-status] backlight: can't set {name} (not by its file, not through logind: {e})");
        *logind = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(root: &Path, name: &str, kind: &str, max: u64, now: u64) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        fs::write(dir.join("brightness"), format!("{now}\n")).unwrap();
    }

    #[test]
    fn the_firmwares_device_wins_and_reads_as_a_fraction() {
        let root = std::env::temp_dir().join(format!("lntrn-status-backlight-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(pick(&root).is_none(), "a desktop has none");
        device(&root, "acpi_video0", "firmware", 100, 40);
        device(&root, "amdgpu_bl1", "raw", 255, 255);
        device(&root, "dead", "firmware", 0, 0);
        let picked = pick(&root).unwrap();
        assert_eq!(picked.dir.file_name().unwrap(), "acpi_video0");
        assert_eq!(picked.state(), BacklightState { present: true, level: 0.4 });
        // Its file is ours here, so setting goes straight to it.
        write(&picked, 0.75, &mut None);
        assert_eq!(picked.raw(), Some(75));
    }
}

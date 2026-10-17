//! The laptop's battery, from `/sys/class/power_supply`: its charge, and
//! whether it is charging and how long it has left. Every battery of the
//! system's own counts (two make one by their energy); a mouse's or a
//! headset's (`scope` = `Device`) doesn't. A charger is any mains or
//! USB-C supply that is `online`.
//!
//! The thread reads it all again whenever the kernel announces a power
//! supply event (a charger plugged in or out, most batteries' percent
//! moving), and every half minute besides, for the time left, which no
//! event marks.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::board::{Board, Notify};
use crate::sys::{self, Netlink};

const ROOT: &str = "/sys/class/power_supply";
/// How often it is read with no event to say so.
const REREAD: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    /// Plugged in and full.
    Full,
    /// Plugged in, held where it is (a charge limit, or the firmware).
    NotCharging,
    #[default]
    Unknown,
}

impl Charge {
    fn from_word(word: &str) -> Charge {
        match word {
            "Charging" => Charge::Charging,
            "Discharging" => Charge::Discharging,
            "Full" => Charge::Full,
            "Not charging" => Charge::NotCharging,
            _ => Charge::Unknown,
        }
    }

    /// As a person would say it.
    pub fn label(self) -> &'static str {
        match self {
            Charge::Charging => "Charging",
            Charge::Discharging => "On battery",
            Charge::Full => "Fully charged",
            Charge::NotCharging => "Plugged in, not charging",
            Charge::Unknown => "Battery",
        }
    }
}

/// A cap the firmware holds the charge to: `charge_control_end_threshold`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChargeLimit {
    /// The percent it stops at; 100 is no cap.
    pub percent: u32,
    /// This user may change it.
    pub writable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatteryState {
    /// The machine has a battery of its own.
    pub present: bool,
    /// 0 to 100.
    pub percent: u32,
    pub charge: Charge,
    /// A charger is plugged in.
    pub plugged: bool,
    /// Until empty while discharging, until full while charging.
    pub seconds_left: Option<u64>,
    /// What it is drawing or taking now, watts.
    pub watts: Option<f64>,
    /// The first battery's charge cap, where it has one.
    pub limit: Option<ChargeLimit>,
}

impl BatteryState {
    /// Charging, or plugged in and held: a bolt shows.
    pub fn on_power(&self) -> bool {
        self.plugged || matches!(self.charge, Charge::Charging | Charge::Full | Charge::NotCharging)
    }
}

pub struct Battery {
    board: Arc<Board<BatteryState>>,
}

impl Battery {
    /// Read the battery now, then keep it current from a thread of its
    /// own. A machine without one says so (`present` false) and stops.
    pub fn start(notify: Notify) -> Battery {
        let board = Board::new(read(Path::new(ROOT)), notify);
        if board.get().present {
            let theirs = board.clone();
            let spawned = std::thread::Builder::new().name("battery".into()).spawn(move || follow(&theirs));
            if let Err(e) = spawned {
                eprintln!("[lntrn-status] battery: no thread: {e}; the charge shown stays as it was at start");
            }
        }
        Battery { board }
    }

    pub fn state(&self) -> BatteryState {
        self.board.get()
    }

    /// Cap the charge at `percent` (100 lifts the cap), where the firmware
    /// can and this user may.
    pub fn set_limit(&self, percent: u32) -> io::Result<()> {
        let Some(bat) = batteries(Path::new(ROOT)).into_iter().next() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no battery"));
        };
        let percent = percent.clamp(50, 100);
        fs::write(bat.join("charge_control_end_threshold"), format!("{percent}\n"))?;
        self.board.update(|s| {
            if let Some(limit) = &mut s.limit {
                limit.percent = percent;
            }
        });
        Ok(())
    }
}

/// Read again on each power supply event, and every [`REREAD`].
fn follow(board: &Board<BatteryState>) {
    let events = Netlink::uevents();
    if let Err(e) = &events {
        eprintln!("[lntrn-status] battery: no kernel events ({e}); reading every {}s", REREAD.as_secs());
    }
    let mut due = Instant::now() + REREAD;
    loop {
        // Every event waiting at once, and one read for them all, when
        // any is a supply's.
        let ours = match &events {
            Ok(nl) => {
                sys::wait(&[nl.fd()], Some(due.saturating_duration_since(Instant::now())));
                nl.take().iter().any(|m| is_power_supply(m))
            }
            Err(_) => {
                std::thread::sleep(REREAD);
                false
            }
        };
        if ours || Instant::now() >= due {
            board.put(read(Path::new(ROOT)));
            due = Instant::now() + REREAD;
        }
    }
}

/// Whether a kernel event (`change@/devices/…\0ACTION=change\0…`) is
/// about a power supply.
fn is_power_supply(event: &[u8]) -> bool {
    event.split(|&b| b == 0).any(|field| field == b"SUBSYSTEM=power_supply")
}

/// The system's own batteries under `root`, by name.
fn batteries(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| word(p, "type").as_deref() == Some("Battery") && word(p, "scope").as_deref() != Some("Device") && p.join("capacity").exists()).collect();
    found.sort();
    found
}

/// One sysfs attribute, trimmed.
fn word(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_owned())
}

fn number(dir: &Path, name: &str) -> Option<u64> {
    // Some drivers write a current or power below zero while discharging.
    word(dir, name).and_then(|s| s.parse::<i64>().ok()).map(i64::unsigned_abs)
}

/// One battery's energy, in its own units (µWh, or µAh when it counts
/// charge), and the rate it moves at (µW or µA).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Store {
    now: u64,
    full: u64,
    rate: u64,
}

fn store(bat: &Path) -> Option<Store> {
    let pick = |a: &str, b: &str| number(bat, a).or_else(|| number(bat, b));
    if let (Some(now), Some(full)) = (number(bat, "energy_now"), number(bat, "energy_full")) {
        return Some(Store { now, full, rate: number(bat, "power_now").unwrap_or(0) });
    }
    Some(Store { now: number(bat, "charge_now")?, full: number(bat, "charge_full")?, rate: pick("current_now", "current_avg").unwrap_or(0) })
}

/// Everything under `root` as one battery.
fn read(root: &Path) -> BatteryState {
    let bats = batteries(root);
    if bats.is_empty() {
        return BatteryState::default();
    }
    let charges: Vec<Charge> = bats.iter().map(|b| Charge::from_word(&word(b, "status").unwrap_or_default())).collect();
    let charge = if charges.contains(&Charge::Charging) {
        Charge::Charging
    } else if charges.contains(&Charge::Discharging) {
        Charge::Discharging
    } else if charges.iter().all(|c| *c == Charge::Full) {
        Charge::Full
    } else if charges.contains(&Charge::NotCharging) {
        Charge::NotCharging
    } else {
        Charge::Unknown
    };
    let stores: Vec<Store> = bats.iter().filter_map(|b| store(b)).collect();
    let (now, full, rate) = stores.iter().fold((0, 0, 0), |(n, f, r), s| (n + s.now, f + s.full, r + s.rate));
    // The kernel's own percent for one battery; two weigh by energy.
    let percent = match bats.as_slice() {
        [one] => number(one, "capacity").unwrap_or(0) as u32,
        _ if full > 0 => ((now as f64 / full as f64) * 100.0).round() as u32,
        _ => (bats.iter().filter_map(|b| number(b, "capacity")).sum::<u64>() / bats.len() as u64) as u32,
    }
    .min(100);
    let seconds_left = (rate > 0 && stores.len() == bats.len())
        .then(|| match charge {
            Charge::Discharging => Some(now.saturating_mul(3600) / rate),
            Charge::Charging => Some(full.saturating_sub(now).saturating_mul(3600) / rate),
            _ => None,
        })
        .flatten();
    let watts = bats.iter().filter_map(|b| number(b, "power_now")).reduce(|a, b| a + b).filter(|w| *w > 0).map(|uw| uw as f64 / 1_000_000.0);
    let limit_file = bats[0].join("charge_control_end_threshold");
    let limit = limit_file.exists().then(|| ChargeLimit { percent: number(&bats[0], "charge_control_end_threshold").unwrap_or(100) as u32, writable: sys::writable(&limit_file) });
    BatteryState { present: true, percent, charge, plugged: plugged(root), seconds_left, watts, limit }
}

/// Whether a charger (mains, or a USB-C supply) is plugged in.
fn plugged(root: &Path) -> bool {
    fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).any(|p| matches!(word(&p, "type").as_deref(), Some("Mains" | "USB")) && word(&p, "online").as_deref() == Some("1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A power_supply folder of our own, files and all.
    fn supply(root: &Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            fs::write(dir.join(file), format!("{text}\n")).unwrap();
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lntrn-status-battery-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_desktop_has_none_and_a_mouses_battery_isnt_the_laptops() {
        let root = scratch("none");
        assert!(!read(&root).present);
        supply(&root, "hidpp_battery_0", &[("type", "Battery"), ("scope", "Device"), ("capacity", "40"), ("status", "Discharging")]);
        assert!(!read(&root).present);
    }

    #[test]
    fn one_battery_reads_with_its_time_left_and_charger() {
        let root = scratch("one");
        supply(&root, "BAT0", &[("type", "Battery"), ("capacity", "87"), ("status", "Discharging"), ("energy_now", "40000000"), ("energy_full", "50000000"), ("power_now", "10000000")]);
        supply(&root, "AC", &[("type", "Mains"), ("online", "0")]);
        let s = read(&root);
        assert_eq!((s.present, s.percent, s.charge, s.plugged), (true, 87, Charge::Discharging, false));
        assert_eq!(s.seconds_left, Some(4 * 3600), "40 Wh at 10 W");
        assert_eq!(s.watts, Some(10.0));
        assert_eq!(s.limit, None);
        supply(&root, "BAT0", &[("status", "Charging"), ("charge_control_end_threshold", "80")]);
        supply(&root, "AC", &[("online", "1")]);
        let s = read(&root);
        assert_eq!((s.charge, s.plugged, s.seconds_left), (Charge::Charging, true, Some(3600)));
        assert_eq!(s.limit.map(|l| l.percent), Some(80));
        assert!(s.on_power());
    }

    #[test]
    fn two_batteries_weigh_by_energy_and_charge_counts_too() {
        let root = scratch("two");
        supply(&root, "BAT0", &[("type", "Battery"), ("capacity", "100"), ("status", "Full"), ("charge_now", "3000000"), ("charge_full", "3000000"), ("current_now", "0")]);
        supply(&root, "BAT1", &[("type", "Battery"), ("capacity", "0"), ("status", "Discharging"), ("charge_now", "0"), ("charge_full", "1000000"), ("current_now", "-500000")]);
        let s = read(&root);
        assert_eq!((s.percent, s.charge), (75, Charge::Discharging));
        // Discharging at 0.5 A from 3 Ah left: six hours.
        assert_eq!(s.seconds_left, Some(6 * 3600));
    }

    #[test]
    fn only_power_supply_events_count() {
        assert!(is_power_supply(b"change@/devices/LNXSYSTM:00/BAT0\0ACTION=change\0SUBSYSTEM=power_supply\0"));
        assert!(!is_power_supply(b"add@/devices/usb1\0ACTION=add\0SUBSYSTEM=usb\0"));
    }
}

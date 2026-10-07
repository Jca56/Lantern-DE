//! What the machine's own sensors say: temperatures and fans from
//! `/sys/class/hwmon`, and the battery when there is one.

use std::fs;
use std::path::{Path, PathBuf};

use super::{read, read_number};

/// How many looks pass between searches for chips that came or went.
const RESCAN_EVERY: u64 = 30;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Temp {
    pub label: String,
    pub celsius: f32,
    /// Where the maker says it is too hot, when the chip knows.
    pub limit: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fan {
    pub label: String,
    pub rpm: u32,
}

/// One sensor chip and what it measures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chip {
    /// What it is, in words: "Processor".
    pub name: String,
    /// The driver's name for it: "coretemp".
    pub driver: String,
    pub temps: Vec<Temp>,
    pub fans: Vec<Fan>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Battery {
    pub name: String,
    pub percent: f32,
    /// "Charging", "Discharging", "Full", as the kernel says it.
    pub status: String,
    /// Power going in or out, when it is measured.
    pub watts: Option<f32>,
    /// Hours until it is empty, or until it is full when charging.
    pub hours: Option<f32>,
    /// How much of the charge it was built to hold it still holds,
    /// 0 to 1.
    pub health: Option<f32>,
    pub cycles: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sensors {
    pub chips: Vec<Chip>,
    pub batteries: Vec<Battery>,
}

impl Sensors {
    /// The hottest thing any chip measures.
    pub fn hottest(&self) -> Option<f32> {
        self.chips.iter().flat_map(|c| &c.temps).map(|t| t.celsius).fold(None, |hot: Option<f32>, c| Some(hot.map_or(c, |h| h.max(c))))
    }
}

/// What a driver's chip is, in words. One nobody has named here keeps
/// the driver's name.
pub fn chip_name(driver: &str) -> String {
    let known = match driver {
        "coretemp" | "k10temp" | "zenpower" | "cpu_thermal" => "Processor",
        "nvme" => "NVMe drive",
        "drivetemp" => "Drive",
        "acpitz" => "Mainboard",
        "spd5118" | "jc42" => "Memory module",
        "amdgpu" | "radeon" | "nouveau" | "i915" | "xe" => "Graphics",
        "pch_cannonlake" | "pch_cometlake" | "pch_skylake" => "Chipset",
        "BAT0" | "BAT1" => "Battery",
        "thinkpad" | "asus" | "dell_smm" | "applesmc" | "nct6775" | "it87" => "Mainboard",
        d if d.starts_with("iwlwifi") || d.starts_with("mt79") || d.starts_with("ath1") => "Wi-Fi",
        d if d.starts_with("nct") || d.starts_with("it8") || d.starts_with("f71") => "Mainboard",
        _ => return driver.to_owned(),
    };
    known.to_owned()
}

/// One reading a chip offers: where it is read, what it is called and
/// where it is too hot. Found once, read every look.
struct Input {
    path: PathBuf,
    label: String,
    limit: Option<f32>,
}

struct Found {
    driver: String,
    temps: Vec<Input>,
    fans: Vec<Input>,
}

/// The number after `prefix` in a sensor file's name: 12 in `temp12_input`.
fn input_number(file: &str, prefix: &str) -> Option<u32> {
    file.strip_prefix(prefix)?.strip_suffix("_input")?.parse().ok()
}

fn milli(path: &Path) -> Option<f32> {
    read_number::<f32>(&path.to_string_lossy()).map(|v| v / 1000.0)
}

/// Everything `dir` (one hwmon chip) can be asked.
fn find(dir: &Path) -> Option<Found> {
    let driver = read(&dir.join("name").to_string_lossy())?;
    let mut numbered: Vec<(bool, u32)> = fs::read_dir(dir).ok()?.flatten().filter_map(|e| e.file_name().into_string().ok()).filter_map(|f| input_number(&f, "temp").map(|n| (true, n)).or(input_number(&f, "fan").map(|n| (false, n)))).collect();
    numbered.sort_unstable();
    let (mut temps, mut fans) = (Vec::new(), Vec::new());
    for (is_temp, n) in numbered {
        let kind = if is_temp { "temp" } else { "fan" };
        let file = |what: &str| dir.join(format!("{kind}{n}_{what}"));
        let label = read(&file("label").to_string_lossy()).filter(|l| !l.is_empty()).unwrap_or_else(|| format!("{} {n}", if is_temp { "Sensor" } else { "Fan" }));
        if is_temp {
            // "Too hot" is the critical point; a chip without one may
            // still say where it should stay under.
            let limit = milli(&file("crit")).or(milli(&file("max"))).filter(|l| *l > 0.0 && *l < 200.0);
            temps.push(Input { path: file("input"), label, limit });
        } else {
            fans.push(Input { path: file("input"), label, limit: None });
        }
    }
    (!temps.is_empty() || !fans.is_empty()).then_some(Found { driver, temps, fans })
}

fn find_all() -> Vec<Found> {
    let Ok(dir) = fs::read_dir("/sys/class/hwmon") else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.iter().filter_map(|d| find(d)).collect()
}

/// A battery's numbers, which the kernel gives as energy (µWh) on some
/// machines and as charge (µAh) on others: the same sums work on both.
fn battery(dir: &Path) -> Option<Battery> {
    let file = |name: &str| dir.join(name).to_string_lossy().into_owned();
    if read(&file("type"))? != "Battery" || read(&file("scope")).is_some_and(|s| s == "Device") {
        // Not a battery, or a mouse's or a keyboard's own.
        return None;
    }
    let number = |name: &str| read_number::<f64>(&file(name));
    let (now, full, design) = (number("energy_now").or(number("charge_now")), number("energy_full").or(number("charge_full")), number("energy_full_design").or(number("charge_full_design")));
    let status = read(&file("status")).unwrap_or_default();
    // Power, or current times voltage where only those are measured.
    let volts = number("voltage_now").map(|v| v / 1e6);
    let watts = number("power_now").map(|p| p / 1e6).or(number("current_now").zip(volts).map(|(c, v)| c / 1e6 * v)).filter(|w| *w > 0.05);
    // The same unit as `now`, a second: charge needs no voltage for this.
    let flow = number("power_now").or(number("current_now")).filter(|f| *f > 0.0);
    let left = match status.as_str() {
        "Charging" => full.zip(now).map(|(f, n)| f - n),
        "Discharging" => now,
        _ => None,
    };
    let percent = number("capacity").or(now.zip(full).filter(|(_, f)| *f > 0.0).map(|(n, f)| n / f * 100.0))?;
    Some(Battery {
        name: dir.file_name()?.to_string_lossy().into_owned(),
        percent: percent.clamp(0.0, 100.0) as f32,
        status,
        watts: watts.map(|w| w as f32),
        hours: left.zip(flow).map(|(l, f)| (l / f) as f32).filter(|h| h.is_finite() && *h > 0.0 && *h < 100.0),
        health: full.zip(design).filter(|(_, d)| *d > 0.0).map(|(f, d)| (f / d).clamp(0.0, 1.0) as f32),
        cycles: read_number::<u32>(&file("cycle_count")).filter(|c| *c > 0),
    })
}

#[derive(Default)]
pub struct SensorSampler {
    found: Vec<Found>,
    looks: u64,
}

impl SensorSampler {
    pub fn sample(&mut self) -> Sensors {
        if self.looks.is_multiple_of(RESCAN_EVERY) {
            self.found = find_all();
        }
        self.looks += 1;
        let chips = self
            .found
            .iter()
            .map(|f| Chip {
                name: chip_name(&f.driver),
                driver: f.driver.clone(),
                temps: f.temps.iter().filter_map(|i| milli(&i.path).map(|celsius| Temp { label: i.label.clone(), celsius, limit: i.limit })).collect(),
                fans: f.fans.iter().filter_map(|i| read_number::<u32>(&i.path.to_string_lossy()).map(|rpm| Fan { label: i.label.clone(), rpm })).collect(),
            })
            .filter(|c| !c.temps.is_empty() || !c.fans.is_empty())
            .collect();
        let mut batteries: Vec<Battery> = fs::read_dir("/sys/class/power_supply").map(|d| d.flatten().filter_map(|e| battery(&e.path())).collect()).unwrap_or_default();
        batteries.sort_by(|a, b| a.name.cmp(&b.name));
        Sensors { chips, batteries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder of files, standing in for one of sysfs's.
    fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lntrn-sysmon-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    #[test]
    fn a_chip_is_found_with_its_labels_and_limits_in_number_order() {
        let dir = scratch("hwmon", &[("name", "coretemp\n"), ("temp10_input", "61000\n"), ("temp10_label", "Core 8\n"), ("temp1_input", "54000\n"), ("temp1_label", "Package id 0\n"), ("temp1_crit", "100000\n"), ("temp1_max", "80000\n"), ("temp2_input", "40000\n"), ("temp2_max", "84000\n"), ("fan1_input", "1200\n"), ("in0_input", "900\n")]);
        let f = find(&dir).unwrap();
        assert_eq!(f.driver, "coretemp");
        assert_eq!(f.temps.iter().map(|t| (t.label.as_str(), t.limit)).collect::<Vec<_>>(), [("Package id 0", Some(100.0)), ("Sensor 2", Some(84.0)), ("Core 8", None)]);
        assert_eq!(f.fans.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Fan 1"]);
        assert_eq!(milli(&f.temps[0].path), Some(54.0));
        // A chip with nothing to read (voltages only) is not listed.
        assert!(find(&scratch("hwmon-empty", &[("name", "asus\n"), ("in0_input", "1\n")])).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_battery_is_read_whether_it_counts_energy_or_charge() {
        let energy = scratch("bat-energy", &[("type", "Battery\n"), ("status", "Discharging\n"), ("capacity", "50\n"), ("energy_now", "30000000\n"), ("energy_full", "60000000\n"), ("energy_full_design", "75000000\n"), ("power_now", "15000000\n"), ("cycle_count", "212\n")]);
        let b = battery(&energy).unwrap();
        assert_eq!((b.percent, b.watts, b.hours, b.health, b.cycles), (50.0, Some(15.0), Some(2.0), Some(0.8), Some(212)));

        let charge = scratch("bat-charge", &[("type", "Battery\n"), ("status", "Charging\n"), ("charge_now", "2000000\n"), ("charge_full", "4000000\n"), ("current_now", "1000000\n"), ("voltage_now", "12000000\n")]);
        let b = battery(&charge).unwrap();
        // No `capacity`: worked out. Two amp-hours to go at one amp.
        assert_eq!((b.percent, b.watts, b.hours, b.health, b.cycles), (50.0, Some(12.0), Some(2.0), None, None));

        // A mouse's battery, and the mains, are not the machine's.
        assert!(battery(&scratch("bat-mouse", &[("type", "Battery\n"), ("scope", "Device\n"), ("capacity", "80\n")])).is_none());
        assert!(battery(&scratch("bat-mains", &[("type", "Mains\n")])).is_none());
        for dir in [energy, charge] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn chips_are_named_and_the_hottest_is_found() {
        assert_eq!(chip_name("coretemp"), "Processor");
        assert_eq!(chip_name("iwlwifi_1"), "Wi-Fi");
        assert_eq!(chip_name("nct6798"), "Mainboard");
        assert_eq!(chip_name("mystery9000"), "mystery9000");
        let temp = |c: f32| Temp { label: String::new(), celsius: c, limit: None };
        let s = Sensors { chips: vec![Chip { temps: vec![temp(40.0), temp(71.5)], ..Chip::default() }, Chip { temps: vec![temp(55.0)], ..Chip::default() }], batteries: Vec::new() };
        assert_eq!(s.hottest(), Some(71.5));
        assert_eq!(Sensors::default().hottest(), None);
        // This machine: whatever it has is read without a fuss.
        let live = SensorSampler::default().sample();
        assert!(live.chips.iter().all(|c| !c.name.is_empty()));
    }
}

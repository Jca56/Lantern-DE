//! How hot the processor runs: the sensor chip its driver makes (AMD's
//! `k10temp` or `zenpower`, Intel's `coretemp`, an ARM board's
//! `cpu_thermal`), its whole-package reading where it has one; failing
//! a chip, the thermal zone the kernel keeps for the package. Found once
//! and read each look; looked for again, now and then, while it can't be
//! read (a driver loaded late, or gone).

use std::fs;
use std::path::{Path, PathBuf};

use super::read;

/// How many looks pass between searches while there is nothing to read.
const RESCAN_EVERY: u32 = 30;

/// The processors' own drivers, the likeliest first.
const DRIVERS: [&str; 5] = ["k10temp", "zenpower", "coretemp", "cpu_thermal", "cpu-thermal"];
/// Readings that are the whole processor's, the best first: AMD's die
/// (true) temperature, its control one (which may run a little high),
/// Intel's package.
const LABELS: [&str; 4] = ["Tdie", "Tctl", "Package id 0", "CPU"];

/// The reading to take of a chip's `(number, label)`s: the best named
/// one, else its first.
pub fn pick(inputs: &[(u32, String)]) -> Option<u32> {
    LABELS.iter().find_map(|want| inputs.iter().find(|(_, label)| label == want).map(|(n, _)| *n)).or_else(|| inputs.iter().map(|(n, _)| *n).min())
}

/// The `tempN_input` files of hwmon chip `dir`, with their labels.
fn inputs(dir: &Path) -> Vec<(u32, String)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|f| f.strip_prefix("temp")?.strip_suffix("_input")?.parse::<u32>().ok())
        .map(|n| (n, read(&dir.join(format!("temp{n}_label")).to_string_lossy()).unwrap_or_default()))
        .collect()
}

/// The file the processor's temperature is read from, in millidegrees.
fn find() -> Option<PathBuf> {
    let mut chips: Vec<(usize, PathBuf)> = fs::read_dir("/sys/class/hwmon")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = read(&e.path().join("name").to_string_lossy())?;
            DRIVERS.iter().position(|d| *d == name).map(|rank| (rank, e.path()))
        })
        .collect();
    // By driver, then by folder: the first package of a two-socket board.
    chips.sort();
    let chip = chips.into_iter().find_map(|(_, dir)| pick(&inputs(&dir)).map(|n| dir.join(format!("temp{n}_input"))));
    chip.or_else(|| {
        let mut zones: Vec<PathBuf> = fs::read_dir("/sys/class/thermal").into_iter().flatten().flatten().map(|e| e.path()).filter(|p| read(&p.join("type").to_string_lossy()).is_some_and(|t| t == "x86_pkg_temp")).collect();
        zones.sort();
        zones.into_iter().next().map(|z| z.join("temp"))
    })
}

/// Degrees Celsius from a file in millidegrees, when it reads as a
/// temperature at all.
fn celsius(path: &Path) -> Option<f32> {
    read(&path.to_string_lossy())?.parse::<f32>().ok().map(|m| m / 1000.0).filter(|c| (-40.0..200.0).contains(c))
}

pub struct CpuTemp {
    input: Option<PathBuf>,
    /// Looks since the last search, while there is nothing to read.
    since: u32,
}

impl Default for CpuTemp {
    fn default() -> Self {
        CpuTemp { input: find(), since: 0 }
    }
}

impl CpuTemp {
    /// The processor's temperature now, °C. `None` on a machine that
    /// doesn't say.
    pub fn read(&mut self) -> Option<f32> {
        if let Some(c) = self.input.as_deref().and_then(celsius) {
            self.since = 0;
            return Some(c);
        }
        self.since += 1;
        if self.since >= RESCAN_EVERY {
            self.since = 0;
            self.input = find();
            return self.input.as_deref().and_then(celsius);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labelled(list: &[(u32, &str)]) -> Vec<(u32, String)> {
        list.iter().map(|(n, l)| (*n, (*l).to_owned())).collect()
    }

    #[test]
    fn the_whole_processors_reading_is_picked_over_a_cores() {
        // Ryzen: the die over control, over each chiplet.
        assert_eq!(pick(&labelled(&[(1, "Tctl"), (3, "Tccd1"), (4, "Tccd2")])), Some(1));
        assert_eq!(pick(&labelled(&[(1, "Tctl"), (2, "Tdie"), (3, "Tccd1")])), Some(2));
        // Intel: the package over its cores.
        assert_eq!(pick(&labelled(&[(2, "Core 0"), (3, "Core 1"), (1, "Package id 0")])), Some(1));
        // Nothing named: the first there is.
        assert_eq!(pick(&labelled(&[(3, ""), (1, "")])), Some(1));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn this_machine_says_a_sensible_temperature_if_any() {
        let mut t = CpuTemp::default();
        assert!(t.read().is_none_or(|c| (0.0..150.0).contains(&c)));
    }
}

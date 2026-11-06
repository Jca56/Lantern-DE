//! The processor, through its frequency driver: each thread's clock and
//! load and the package's heat for watching, and for tuning the knobs
//! Linux gives every core at once: the governor, the energy preference,
//! turbo, and a ceiling on speed.
//!
//! On Intel's `intel_pstate` the ceiling is the driver's own
//! `max_perf_pct`; elsewhere each thread's `scaling_max_freq`, as a share
//! of its own fastest (a hybrid part's two kinds of core top out apart).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::sysfs;
use crate::tune::CpuLimits;

const BASE: &str = "/sys/devices/system/cpu";

/// One logical CPU.
struct Thread {
    id: usize,
    freq: PathBuf,
    max_khz: u64,
    /// A performance core's (or a part with only one kind).
    pcore: bool,
}

/// How busy one thread has been: (busy, all) ticks of `/proc/stat`.
type Ticks = (u64, u64);

/// One thread's reading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreadReading {
    pub id: usize,
    pub mhz: u32,
    pub usage: f32,
    pub pcore: bool,
}

/// The processor's reading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    /// How busy it all is, percent.
    pub usage: f32,
    /// The package's heat, °C.
    pub celsius: Option<f32>,
    pub threads: Vec<ThreadReading>,
}

/// The knobs as they stand.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub governor: String,
    pub epp: String,
    pub turbo: bool,
    pub max_percent: u32,
}

pub struct Cpu {
    pub model: String,
    threads: Vec<Thread>,
    /// The package temperature's file, from the `coretemp` sensor.
    package: Option<PathBuf>,
    last: Option<(Ticks, HashMap<usize, Ticks>)>,
}

/// The CPUs a hybrid part's performance cores are, as the kernel's perf
/// devices list them; `None` on a part with one kind of core.
fn hybrid_pcores() -> Option<Vec<usize>> {
    let list = sysfs::read(Path::new("/sys/devices/cpu_core/cpus"))?;
    sysfs::read(Path::new("/sys/devices/cpu_atom/cpus"))?;
    let mut ids = Vec::new();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => ids.extend(a.parse::<usize>().ok()?..=b.parse::<usize>().ok()?),
            None => ids.push(part.parse().ok()?),
        }
    }
    Some(ids)
}

fn package_sensor() -> Option<PathBuf> {
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let dir = entry.path();
        if sysfs::read(&dir.join("name")).as_deref() != Some("coretemp") {
            continue;
        }
        let mut first = None;
        for i in 1..64 {
            let input = dir.join(format!("temp{i}_input"));
            if !input.exists() {
                continue;
            }
            if sysfs::read(&dir.join(format!("temp{i}_label"))).is_some_and(|l| l.starts_with("Package")) {
                return Some(input);
            }
            first.get_or_insert(input);
        }
        return first;
    }
    None
}

/// Busy and all ticks for the whole machine and each CPU.
fn ticks() -> Option<(Ticks, HashMap<usize, Ticks>)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let mut all = None;
    let mut each = HashMap::new();
    for line in text.lines().take_while(|l| l.starts_with("cpu")) {
        let mut words = line.split_whitespace();
        let tag = words.next()?;
        let n: Vec<u64> = words.filter_map(|w| w.parse().ok()).collect();
        if n.len() < 5 {
            continue;
        }
        // user nice system idle iowait irq softirq steal: idle and iowait rest.
        let total: u64 = n.iter().take(8).sum();
        let t = (total - n[3] - n[4], total);
        match tag[3..].parse::<usize>() {
            Ok(id) => {
                each.insert(id, t);
            }
            Err(_) => all = Some(t),
        }
    }
    Some((all?, each))
}

fn usage(was: Ticks, now: Ticks) -> f32 {
    let (busy, all) = (now.0.saturating_sub(was.0), now.1.saturating_sub(was.1));
    if all == 0 { 0.0 } else { (busy as f32 / all as f32 * 100.0).clamp(0.0, 100.0) }
}

impl Cpu {
    pub fn discover() -> Cpu {
        let model = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim().to_owned()))
            .unwrap_or_else(|| "Processor".to_owned());
        let pcores = hybrid_pcores();
        let mut threads = Vec::new();
        for entry in std::fs::read_dir(BASE).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_prefix("cpu").and_then(|n| n.parse::<usize>().ok()) else { continue };
            let freq = entry.path().join("cpufreq");
            if !freq.is_dir() {
                continue;
            }
            let max_khz = sysfs::read_u64(&freq.join("cpuinfo_max_freq")).unwrap_or(0);
            let pcore = pcores.as_ref().is_none_or(|p| p.contains(&id));
            threads.push(Thread { id, freq, max_khz, pcore });
        }
        threads.sort_by_key(|t| t.id);
        Cpu { model, threads, package: package_sensor(), last: None }
    }

    /// What it reads now. Loads are since the last reading, so the
    /// first says nothing about them.
    pub fn read(&mut self) -> Reading {
        let now = ticks();
        let (mut total, mut each) = (0.0, HashMap::new());
        if let (Some((was_all, was_each)), Some((all, now_each))) = (&self.last, &now) {
            total = usage(*was_all, *all);
            for (id, t) in now_each {
                if let Some(was) = was_each.get(id) {
                    each.insert(*id, usage(*was, *t));
                }
            }
        }
        if now.is_some() {
            self.last = now;
        }
        let threads = self.threads.iter().map(|t| ThreadReading { id: t.id, mhz: (sysfs::read_u64(&t.freq.join("scaling_cur_freq")).unwrap_or(0) / 1000) as u32, usage: each.get(&t.id).copied().unwrap_or(0.0), pcore: t.pcore }).collect();
        let celsius = self.package.as_deref().and_then(sysfs::read_u64).map(|milli| milli as f32 / 1000.0);
        Reading { usage: total, celsius, threads }
    }

    /// The first thread's frequency directory: the driver keeps every
    /// thread alike, so it speaks for all.
    fn first(&self) -> Option<&Path> {
        self.threads.first().map(|t| t.freq.as_path())
    }

    fn turbo_file() -> Option<(PathBuf, bool)> {
        // intel_pstate says the opposite of what it means: 1 is off.
        let intel = Path::new(BASE).join("intel_pstate/no_turbo");
        if intel.exists() {
            return Some((intel, true));
        }
        let boost = Path::new(BASE).join("cpufreq/boost");
        boost.exists().then_some((boost, false))
    }

    fn pstate_ceiling() -> Option<PathBuf> {
        let path = Path::new(BASE).join("intel_pstate/max_perf_pct");
        path.exists().then_some(path)
    }

    /// How far each knob goes. The power limits are RAPL's to say.
    pub fn limits(&self) -> CpuLimits {
        let top = |pcore: bool| self.threads.iter().filter(|t| t.pcore == pcore).map(|t| t.max_khz).max().map_or(0, |khz| (khz / 1000) as u32);
        CpuLimits {
            model: self.model.clone(),
            governors: self.first().map(|d| sysfs::words(&d.join("scaling_available_governors"))).unwrap_or_default(),
            epps: self.first().map(|d| sysfs::words(&d.join("energy_performance_available_preferences"))).unwrap_or_default(),
            turbo: Self::turbo_file().is_some(),
            pcore_mhz: top(true),
            ecore_mhz: top(false),
            power: None,
        }
    }

    /// The knobs as they stand now.
    pub fn settings(&self) -> Settings {
        let first = self.first();
        let read = |file: &str| first.and_then(|d| sysfs::read(&d.join(file))).unwrap_or_default();
        let turbo = Self::turbo_file().and_then(|(path, inverted)| sysfs::read(&path).map(|v| (v == "1") != inverted)).unwrap_or(true);
        let max_percent = match Self::pstate_ceiling() {
            Some(path) => sysfs::read_u64(&path).unwrap_or(100) as u32,
            None => match (first.and_then(|d| sysfs::read_u64(&d.join("scaling_max_freq"))), self.threads.first().map(|t| t.max_khz)) {
                (Some(now), Some(max)) if max > 0 => ((now * 100 + max / 2) / max) as u32,
                _ => 100,
            },
        };
        Settings { governor: read("scaling_governor"), epp: read("energy_performance_preference"), turbo, max_percent: max_percent.clamp(1, 100) }
    }

    fn write_each(&self, file: &str, value: &str) -> Result<(), String> {
        if self.threads.is_empty() {
            return Err("this processor has no frequency driver".to_owned());
        }
        self.threads.iter().try_for_each(|t| sysfs::write(&t.freq.join(file), value))
    }

    pub fn set_governor(&self, governor: &str) -> Result<(), String> {
        self.write_each("scaling_governor", governor)
    }

    pub fn set_epp(&self, epp: &str) -> Result<(), String> {
        self.write_each("energy_performance_preference", epp)
    }

    pub fn set_turbo(&self, on: bool) -> Result<(), String> {
        let (path, inverted) = Self::turbo_file().ok_or("this processor's turbo can't be switched")?;
        sysfs::write(&path, if on != inverted { "1" } else { "0" })
    }

    pub fn set_max_percent(&self, percent: u32) -> Result<(), String> {
        let percent = percent.clamp(10, 100);
        if let Some(path) = Self::pstate_ceiling() {
            return sysfs::write(&path, &percent.to_string());
        }
        self.threads.iter().try_for_each(|t| sysfs::write(&t.freq.join("scaling_max_freq"), &(t.max_khz * u64::from(percent) / 100).to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_is_the_share_of_ticks_spent_busy() {
        assert_eq!(usage((10, 100), (60, 200)), 50.0);
        assert_eq!(usage((10, 100), (10, 100)), 0.0, "no time passed");
        assert_eq!(usage((50, 100), (40, 90)), 0.0, "a counter that went back");
    }

    #[test]
    fn this_machine_reads_without_trouble() {
        let mut cpu = Cpu::discover();
        let _ = cpu.read();
        let r = cpu.read();
        assert!((0.0..=100.0).contains(&r.usage));
        let l = cpu.limits();
        if !l.governors.is_empty() {
            assert!(l.governors.contains(&cpu.settings().governor));
        }
        assert!((1..=100).contains(&cpu.settings().max_percent));
    }
}

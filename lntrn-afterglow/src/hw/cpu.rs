//! CPU monitoring (per-core clocks/util via cpufreq + /proc/stat, temps via
//! coretemp hwmon) and the tuning knobs Linux actually exposes: governor,
//! EPP, turbo, and a max-frequency cap. Knob writes need root.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{read_trim, read_u64, write_value};

const CPU_BASE: &str = "/sys/devices/system/cpu";

struct Core {
    id: usize,
    freq_dir: PathBuf,
    max_khz: u64,
    /// True for performance-core threads. On Intel hybrid parts P-cores carry
    /// an HT sibling (2 logical CPUs share a physical core) while E-cores run
    /// solo — a far more reliable split than comparing max turbo bins, which
    /// Turbo Boost Max 3.0 perturbs per favoured core.
    is_pcore: bool,
}

#[derive(Debug, Clone)]
pub struct CoreStat {
    pub id: usize,
    pub mhz: u32,
    pub util_pct: f32,
    pub is_pcore: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CpuSnapshot {
    pub total_util_pct: f32,
    pub package_temp_c: Option<f32>,
    pub cores: Vec<CoreStat>,
}

#[derive(Debug, Clone, Default)]
pub struct CpuTuning {
    pub governor: String,
    pub governors: Vec<String>,
    pub epp: Option<String>,
    pub epps: Vec<String>,
    pub no_turbo: Option<bool>,
    pub max_freq_pct: u32,
}

pub struct Cpu {
    pub model: String,
    cores: Vec<Core>,
    /// (label, temp_input path) pairs from the coretemp hwmon.
    temps: Vec<(String, PathBuf)>,
    /// Previous (busy, total) jiffies per core id, plus the aggregate line.
    prev_stat: Option<(HashMap<usize, (u64, u64)>, (u64, u64))>,
}

impl Cpu {
    pub fn discover() -> Cpu {
        let model = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|v| v.trim().to_string())
            })
            .unwrap_or_else(|| "unknown CPU".to_string());

        let mut cores = Vec::new();
        if let Ok(dir) = std::fs::read_dir(CPU_BASE) {
            for entry in dir.flatten() {
                let fname = entry.file_name().to_string_lossy().into_owned();
                let Some(id) = fname
                    .strip_prefix("cpu")
                    .and_then(|n| n.parse::<usize>().ok())
                else {
                    continue;
                };
                let freq_dir = entry.path().join("cpufreq");
                if !freq_dir.is_dir() {
                    continue;
                }
                let max_khz = read_u64(&freq_dir.join("cpuinfo_max_freq")).unwrap_or(0);
                let siblings = read_trim(&entry.path().join("topology/thread_siblings_list"))
                    .map(|s| count_cpu_list(&s))
                    .unwrap_or(1);
                cores.push(Core {
                    id,
                    freq_dir,
                    max_khz,
                    is_pcore: siblings > 1,
                });
            }
        }
        cores.sort_by_key(|c| c.id);

        Cpu {
            model,
            cores,
            temps: find_coretemp(),
            prev_stat: None,
        }
    }

    pub fn snapshot(&mut self) -> CpuSnapshot {
        let stat = read_proc_stat();
        let mut per_core_util: HashMap<usize, f32> = HashMap::new();
        let mut total_util = 0.0f32;
        if let (Some((prev_cores, prev_total)), Some((cur_cores, cur_total))) =
            (&self.prev_stat, &stat)
        {
            for (id, cur) in cur_cores {
                if let Some(prev) = prev_cores.get(id) {
                    per_core_util.insert(*id, util_pct(*prev, *cur));
                }
            }
            total_util = util_pct(*prev_total, *cur_total);
        }
        if let Some(s) = stat {
            self.prev_stat = Some(s);
        }

        let cores = self
            .cores
            .iter()
            .map(|c| CoreStat {
                id: c.id,
                mhz: (read_u64(&c.freq_dir.join("scaling_cur_freq")).unwrap_or(0) / 1000)
                    as u32,
                util_pct: per_core_util.get(&c.id).copied().unwrap_or(0.0),
                is_pcore: c.is_pcore,
            })
            .collect();

        let package_temp_c = self
            .temps
            .iter()
            .find(|(label, _)| label.starts_with("Package"))
            .or_else(|| self.temps.first())
            .and_then(|(_, path)| read_u64(path))
            .map(|milli| milli as f32 / 1000.0);

        CpuSnapshot {
            total_util_pct: total_util,
            package_temp_c,
            cores,
        }
    }

    pub fn tuning(&self) -> CpuTuning {
        let Some(c0) = self.cores.first() else {
            return CpuTuning::default();
        };
        let read_list = |file: &str| {
            read_trim(&c0.freq_dir.join(file))
                .map(|s| s.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default()
        };
        let max_freq_pct = match (
            read_u64(&c0.freq_dir.join("scaling_max_freq")),
            c0.max_khz,
        ) {
            (Some(cur), max) if max > 0 => ((cur * 100 + max / 2) / max) as u32,
            _ => 100,
        };
        CpuTuning {
            governor: read_trim(&c0.freq_dir.join("scaling_governor")).unwrap_or_default(),
            governors: read_list("scaling_available_governors"),
            epp: read_trim(&c0.freq_dir.join("energy_performance_preference")),
            epps: read_list("energy_performance_available_preferences"),
            no_turbo: read_trim(Path::new(&format!("{CPU_BASE}/intel_pstate/no_turbo")))
                .map(|v| v == "1"),
            max_freq_pct,
        }
    }

    pub fn set_governor(&self, governor: &str) -> Result<(), String> {
        self.write_all_cores("scaling_governor", governor)
    }

    pub fn set_epp(&self, epp: &str) -> Result<(), String> {
        self.write_all_cores("energy_performance_preference", epp)
    }

    pub fn set_no_turbo(&self, no_turbo: bool) -> Result<(), String> {
        write_value(
            Path::new(&format!("{CPU_BASE}/intel_pstate/no_turbo")),
            if no_turbo { "1" } else { "0" },
        )
        .map_err(|e| format!("no_turbo write failed (root needed): {e}"))
    }

    /// Caps every core at `pct` of its own hardware max (P and E cores have
    /// different ceilings, so the cap is proportional, not absolute).
    pub fn set_max_freq_pct(&self, pct: u32) -> Result<(), String> {
        let pct = pct.clamp(10, 100) as u64;
        for core in &self.cores {
            let khz = core.max_khz * pct / 100;
            write_value(&core.freq_dir.join("scaling_max_freq"), &khz.to_string())
                .map_err(|e| format!("cpu{} max_freq write failed (root needed): {e}", core.id))?;
        }
        Ok(())
    }

    fn write_all_cores(&self, file: &str, value: &str) -> Result<(), String> {
        for core in &self.cores {
            write_value(&core.freq_dir.join(file), value)
                .map_err(|e| format!("cpu{} {file} write failed (root needed): {e}", core.id))?;
        }
        Ok(())
    }
}

/// Counts the CPUs named by a sysfs cpu-list like "0,1" or "0-1,4". Used to
/// tell how many logical threads share a physical core.
fn count_cpu_list(list: &str) -> usize {
    let mut count = 0;
    for part in list.split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    count += b.saturating_sub(a) + 1;
                }
            }
            None if !part.trim().is_empty() => count += 1,
            None => {}
        }
    }
    count.max(1)
}

fn util_pct(prev: (u64, u64), cur: (u64, u64)) -> f32 {
    let busy = cur.0.saturating_sub(prev.0) as f32;
    let total = cur.1.saturating_sub(prev.1) as f32;
    if total > 0.0 {
        (busy / total * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// Parses /proc/stat into per-core and aggregate (busy, total) jiffies.
fn read_proc_stat() -> Option<(HashMap<usize, (u64, u64)>, (u64, u64))> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let mut cores = HashMap::new();
    let mut total = (0, 0);
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let tag = parts.next()?;
        if !tag.starts_with("cpu") {
            break;
        }
        let fields: Vec<u64> = parts.filter_map(|f| f.parse().ok()).collect();
        if fields.len() < 5 {
            continue;
        }
        // user nice system idle iowait irq softirq steal ...
        let idle = fields[3] + fields[4];
        let all: u64 = fields.iter().sum();
        let entry = (all - idle, all);
        match tag[3..].parse::<usize>() {
            Ok(id) => {
                cores.insert(id, entry);
            }
            Err(_) => total = entry, // the aggregate "cpu" line
        }
    }
    Some((cores, total))
}

fn find_coretemp() -> Vec<(String, PathBuf)> {
    let mut temps = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") else {
        return temps;
    };
    for entry in dir.flatten() {
        let hwmon = entry.path();
        if read_trim(&hwmon.join("name")).as_deref() != Some("coretemp") {
            continue;
        }
        for i in 1..128 {
            let input = hwmon.join(format!("temp{i}_input"));
            if !input.exists() {
                break;
            }
            let label = read_trim(&hwmon.join(format!("temp{i}_label")))
                .unwrap_or_else(|| format!("temp{i}"));
            temps.push((label, input));
        }
        break;
    }
    temps
}

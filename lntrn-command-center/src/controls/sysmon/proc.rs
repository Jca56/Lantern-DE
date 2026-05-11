//! Direct `/proc` parsers for the system monitor. No external crates —
//! each function reads a single proc file and parses the fields it
//! needs. Errors are folded into `None` / empty results because a
//! transient read failure should just produce a missing data point, not
//! a panic in the panel.

use std::fs;

/// Aggregate CPU jiffy counters from `/proc/stat`'s first `cpu` line.
/// `idle` counts truly idle time; `total` is the sum of all fields so
/// `% busy = 1 - Δidle / Δtotal`.
#[derive(Debug, Clone, Copy)]
pub struct CpuTotals {
    pub idle: u64,
    pub total: u64,
}

pub fn read_cpu_totals() -> Option<CpuTotals> {
    let s = fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_totals(&s)
}

fn parse_cpu_totals(s: &str) -> Option<CpuTotals> {
    let line = s.lines().find(|l| l.starts_with("cpu "))?;
    let mut parts = line.split_whitespace();
    parts.next()?; // "cpu"
    // Fields: user nice system idle iowait irq softirq steal guest guest_nice
    let nums: Vec<u64> = parts.filter_map(|p| p.parse().ok()).collect();
    if nums.len() < 4 {
        return None;
    }
    let idle = nums[3].saturating_add(*nums.get(4).unwrap_or(&0)); // idle + iowait
    let total: u64 = nums.iter().sum();
    Some(CpuTotals { idle, total })
}

/// Memory snapshot in **kilobytes** as exposed by /proc/meminfo.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemInfo {
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

impl MemInfo {
    pub fn used_kb(&self) -> u64 {
        self.mem_total_kb.saturating_sub(self.mem_available_kb)
    }
    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }
    /// 0..1 fraction of RAM in use.
    pub fn used_fraction(&self) -> f32 {
        if self.mem_total_kb == 0 {
            return 0.0;
        }
        (self.used_kb() as f32 / self.mem_total_kb as f32).clamp(0.0, 1.0)
    }
}

pub fn read_meminfo() -> MemInfo {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .map(|s| parse_meminfo(&s))
        .unwrap_or_default()
}

fn parse_meminfo(s: &str) -> MemInfo {
    let mut out = MemInfo::default();
    for line in s.lines() {
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let val: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match key {
            "MemTotal:" => out.mem_total_kb = val,
            "MemAvailable:" => out.mem_available_kb = val,
            "SwapTotal:" => out.swap_total_kb = val,
            "SwapFree:" => out.swap_free_kb = val,
            _ => {}
        }
    }
    out
}

/// Aggregate RX/TX byte counters across non-loopback interfaces.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

pub fn read_net_total() -> NetCounters {
    fs::read_to_string("/proc/net/dev")
        .ok()
        .map(|s| parse_net_total(&s))
        .unwrap_or_default()
}

fn parse_net_total(s: &str) -> NetCounters {
    let mut out = NetCounters::default();
    // Skip the two header lines, then each line is:
    //   iface: rx_bytes rx_packets rx_errs rx_drop ... tx_bytes tx_packets ...
    for line in s.lines().skip(2) {
        let Some((iface_part, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface_part.trim();
        if iface == "lo" || iface.starts_with("docker") || iface.starts_with("veth") {
            continue;
        }
        let nums: Vec<u64> = rest.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        // rx_bytes is column 0, tx_bytes is column 8 (after 8 rx fields).
        if nums.len() >= 9 {
            out.rx_bytes = out.rx_bytes.saturating_add(nums[0]);
            out.tx_bytes = out.tx_bytes.saturating_add(nums[8]);
        }
    }
    out
}

/// Single process snapshot from /proc/<pid>/stat + /proc/<pid>/statm.
#[derive(Debug, Clone)]
pub struct ProcRaw {
    pub pid: i32,
    pub comm: String,
    /// utime + stime in jiffies — used to compute Δ-CPU between samples.
    pub cpu_jiffies: u64,
    /// Resident set size in **bytes**.
    pub rss_bytes: u64,
}

/// Page size in bytes (cached on first call).
fn page_size() -> u64 {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 }
}

/// Read CPU package temperature in degrees Celsius. Walks
/// `/sys/class/thermal/thermal_zone*` and picks the hottest reading
/// from a known CPU-related type (`x86_pkg_temp`, `coretemp*`, `TCPU*`,
/// `Package_id_*`), falling back to `acpitz` if none of those exist.
/// Returns `None` if we can't find any usable zone.
pub fn read_cpu_temp_c() -> Option<f32> {
    let read_dir = fs::read_dir("/sys/class/thermal").ok()?;
    let mut best: Option<(u8, f32)> = None; // (priority, °C) — lower priority wins
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("thermal_zone") {
            continue;
        }
        let path = entry.path();
        let Ok(ty) = fs::read_to_string(path.join("type")) else {
            continue;
        };
        let ty = ty.trim();
        // Priority: lower = preferred. CPU-package readings win; acpitz
        // is the motherboard fallback.
        let prio = if ty == "x86_pkg_temp" {
            0
        } else if ty.starts_with("coretemp") || ty.starts_with("Package_id") {
            1
        } else if ty.starts_with("TCPU") {
            2
        } else if ty == "acpitz" {
            3
        } else {
            continue;
        };
        let Ok(raw) = fs::read_to_string(path.join("temp")) else {
            continue;
        };
        let Ok(millic) = raw.trim().parse::<i32>() else {
            continue;
        };
        // Filter clearly-bogus readings (some zones report 50 m°C as a
        // "sensor disabled" sentinel).
        if millic < 1000 || millic > 150_000 {
            continue;
        }
        let c = millic as f32 / 1000.0;
        // Among same-priority zones (e.g. multiple coretemps), keep the
        // hottest reading — that's the CPU's hottest core.
        match best {
            Some((p, _)) if p < prio => {}
            Some((p, t)) if p == prio && t >= c => {}
            _ => best = Some((prio, c)),
        }
    }
    best.map(|(_, c)| c)
}

/// Iterate /proc reading every numeric subdirectory as a PID. Skips
/// processes that vanish mid-read (race), and our own `kthreadd`-style
/// noise via best-effort filtering at the call site.
pub fn list_processes() -> Vec<ProcRaw> {
    let Ok(read_dir) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let pgsz = page_size();
    let mut out = Vec::with_capacity(256);
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let Ok(pid) = name_str.parse::<i32>() else {
            continue;
        };
        if let Some(p) = read_one(pid, pgsz) {
            out.push(p);
        }
    }
    out
}

fn read_one(pid: i32, pgsz: u64) -> Option<ProcRaw> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // /proc/<pid>/stat: pid (comm) state ppid pgrp session tty_nr tpgid ...
    // The comm field is parenthesised and may contain spaces, so locate
    // the *last* ')' and parse around it.
    let close = stat.rfind(')')?;
    let open = stat.find('(')?;
    let comm = stat[open + 1..close].to_string();
    let after = &stat[close + 1..];
    let parts: Vec<&str> = after.split_whitespace().collect();
    // After comm, fields are 1-indexed in the man page starting at
    // state=2. So utime is field 14 → parts[14 - 3] = parts[11].
    let utime: u64 = parts.get(11)?.parse().ok()?;
    let stime: u64 = parts.get(12)?.parse().ok()?;
    let cpu_jiffies = utime.saturating_add(stime);

    // statm: size resident shared text lib data dt — pages.
    let statm = fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let rss_bytes = resident_pages.saturating_mul(pgsz);

    Some(ProcRaw { pid, comm, cpu_jiffies, rss_bytes })
}

/// Format a byte count as a human-friendly string (e.g. "1.2 GB").
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Format a kilobyte count from /proc/meminfo as human-friendly bytes.
pub fn format_kb(kb: u64) -> String {
    format_bytes(kb.saturating_mul(1024))
}

/// Per-second rate string (e.g. "240 KB/s", "3.1 MB/s").
pub fn format_rate(bytes_per_sec: f32) -> String {
    const KB: f32 = 1024.0;
    const MB: f32 = 1024.0 * 1024.0;
    const GB: f32 = 1024.0 * 1024.0 * 1024.0;
    if bytes_per_sec >= GB {
        format!("{:.1} GB/s", bytes_per_sec / GB)
    } else if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec / MB)
    } else if bytes_per_sec >= KB {
        format!("{:.0} KB/s", bytes_per_sec / KB)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

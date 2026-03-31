//! System monitor — CPU, RAM, disk, network metrics + process list.
//! Drawing code lives in sysmon_draw.rs.

use std::path::PathBuf;
use std::time::Instant;

use lntrn_render::Rect;
use lntrn_ui::gpu::InteractionContext;

pub(crate) const POLL_FAST_MS: u64 = 2_000;
const POLL_DISK_MS: u64 = 10_000;
pub(crate) const MAX_PROCS: usize = 30;

pub(crate) const ZONE_CORES_TOGGLE: u32 = 0xBF_F000;
pub(crate) const ZONE_PROC_BASE: u32 = 0xBF_E000;
pub(crate) const ZONE_PINNED_BASE: u32 = 0xBF_D000;
pub(crate) const ZONE_KILL_BASE: u32 = 0xBF_C000;
pub(crate) const ZONE_SORT_NAME: u32 = 0xBF_B000;
pub(crate) const ZONE_SORT_CPU: u32 = 0xBF_B001;
pub(crate) const ZONE_SORT_MEM: u32 = 0xBF_B002;
pub(crate) const ZONE_SORT_PID: u32 = 0xBF_B003;

#[derive(Clone, Copy, PartialEq)]
pub enum SortColumn { Cpu, Mem, Name, Pid }

pub(crate) const SPARKLINE_LEN: usize = 15;

/// A process pinned to the bar for live monitoring.
pub struct PinnedProcess {
    pub name: String,
    pub pid: u32,
    pub cpu_pct: f32,
    pub mem_kb: u64,
    /// Ring buffer of CPU% samples (~30s at 2s poll).
    pub cpu_history: [f32; SPARKLINE_LEN],
    pub history_idx: usize,
    /// Animation phase for high-CPU warning pulse (0.0–1.0).
    pub warning_phase: f32,
}

pub struct SystemMonitor {
    pub(crate) prev_total: u64, pub(crate) prev_idle: u64, pub(crate) cpu_pct: f32,
    pub(crate) per_core: Vec<f32>, pub(crate) prev_cores: Vec<(u64, u64)>,
    pub(crate) cpu_freqs: Vec<u32>,
    pub(crate) mem_total_kb: u64, pub(crate) mem_avail_kb: u64,
    pub(crate) swap_total_kb: u64, pub(crate) swap_used_kb: u64,
    pub(crate) disks: Vec<DiskInfo>,
    pub(crate) prev_rx: u64, pub(crate) prev_tx: u64,
    pub(crate) rx_rate: f64, pub(crate) tx_rate: f64,
    pub(crate) cpu_temp: u32,
    thermal_zone: Option<PathBuf>,
    pub(crate) procs: Vec<ProcInfo>,
    prev_proc_times: std::collections::HashMap<u32, (u64, u64)>,
    last_fast: Instant, last_disk: Instant, first_tick: bool,
    pub scroll_offset: f32,
    pub cores_expanded: bool,
    pub pinned: Vec<PinnedProcess>,
    pub right_clicked_proc: Option<(String, u32)>,
    pub sort_column: SortColumn,
    pub sort_ascending: bool,
    pub proc_filter: String,
    pub filter_focused: bool,
    pub filter_cursor: usize,
}

pub(crate) struct DiskInfo {
    pub(crate) name: String, pub(crate) mount: String,
    pub(crate) total: u64, pub(crate) used: u64,
}
pub(crate) struct ProcInfo {
    pub(crate) pid: u32, pub(crate) name: String,
    pub(crate) cpu_pct: f32, pub(crate) mem_kb: u64,
}

impl SystemMonitor {
    pub fn new() -> Self {
        Self {
            prev_total: 0, prev_idle: 0, cpu_pct: 0.0,
            per_core: Vec::new(), prev_cores: Vec::new(), cpu_freqs: Vec::new(),
            mem_total_kb: 0, mem_avail_kb: 0, swap_total_kb: 0, swap_used_kb: 0,
            disks: Vec::new(),
            prev_rx: 0, prev_tx: 0, rx_rate: 0.0, tx_rate: 0.0,
            cpu_temp: 0, thermal_zone: find_thermal_zone(),
            procs: Vec::new(), prev_proc_times: std::collections::HashMap::new(),
            last_fast: Instant::now() - std::time::Duration::from_secs(10),
            last_disk: Instant::now() - std::time::Duration::from_secs(60),
            first_tick: true, scroll_offset: 0.0, cores_expanded: false,
            pinned: Vec::new(), right_clicked_proc: None,
            sort_column: SortColumn::Cpu, sort_ascending: false,
            proc_filter: String::new(), filter_focused: false, filter_cursor: 0,
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_fast).as_millis() >= POLL_FAST_MS as u128 || self.first_tick {
            self.poll_cpu();
            self.poll_memory();
            self.poll_network(now);
            self.poll_processes();
            self.poll_temp();
            self.last_fast = now;
        }
        if now.duration_since(self.last_disk).as_millis() >= POLL_DISK_MS as u128 || self.first_tick {
            self.poll_disks();
            self.last_disk = now;
        }
        self.first_tick = false;
        self.update_pinned();
    }

    fn poll_cpu(&mut self) {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else { return };
        for line in stat.lines() {
            if line.starts_with("cpu ") {
                let (total, idle) = parse_cpu_line(line);
                if self.prev_total > 0 {
                    let dt = total.saturating_sub(self.prev_total) as f64;
                    let di = idle.saturating_sub(self.prev_idle) as f64;
                    self.cpu_pct = if dt > 0.0 { ((1.0 - di / dt) * 100.0) as f32 } else { 0.0 };
                }
                self.prev_total = total; self.prev_idle = idle;
            } else if let Some(rest) = line.strip_prefix("cpu") {
                if rest.starts_with(|c: char| c.is_ascii_digit()) {
                    let idx_end = rest.find(' ').unwrap_or(rest.len());
                    let idx: usize = rest[..idx_end].parse().unwrap_or(0);
                    let (total, idle) = parse_cpu_line(line);
                    while self.prev_cores.len() <= idx {
                        self.prev_cores.push((0, 0)); self.per_core.push(0.0);
                    }
                    let (pt, pi) = self.prev_cores[idx];
                    if pt > 0 {
                        let dt = total.saturating_sub(pt) as f64;
                        let di = idle.saturating_sub(pi) as f64;
                        self.per_core[idx] = if dt > 0.0 { ((1.0 - di / dt) * 100.0) as f32 } else { 0.0 };
                    }
                    self.prev_cores[idx] = (total, idle);
                }
            }
        }
        // Read per-core frequencies
        while self.cpu_freqs.len() < self.per_core.len() {
            self.cpu_freqs.push(0);
        }
        for i in 0..self.per_core.len() {
            let path = format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq");
            if let Ok(s) = std::fs::read_to_string(&path) {
                self.cpu_freqs[i] = s.trim().parse::<u32>().unwrap_or(0);
            }
        }
    }

    fn poll_memory(&mut self) {
        let Ok(info) = std::fs::read_to_string("/proc/meminfo") else { return };
        for line in info.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") { self.mem_total_kb = parse_kb(v); }
            else if let Some(v) = line.strip_prefix("MemAvailable:") { self.mem_avail_kb = parse_kb(v); }
            else if let Some(v) = line.strip_prefix("SwapTotal:") { self.swap_total_kb = parse_kb(v); }
            else if let Some(v) = line.strip_prefix("SwapFree:") { self.swap_used_kb = self.swap_total_kb.saturating_sub(parse_kb(v)); }
        }
    }

    fn poll_temp(&mut self) {
        if let Some(path) = &self.thermal_zone {
            let p = path.join("temp");
            if let Ok(s) = std::fs::read_to_string(&p) {
                self.cpu_temp = s.trim().parse::<u32>().unwrap_or(0) / 1000;
            }
        }
    }

    fn poll_disks(&mut self) {
        self.disks.clear();
        let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else { return };
        let mut seen = std::collections::HashSet::new();
        for line in mounts.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() < 3 { continue; }
            let (dev, mount) = (p[0], p[1]);
            if !dev.starts_with("/dev/") || dev.contains("loop") || !seen.insert(dev.to_string()) { continue; }
            if let Some(st) = statvfs(mount) {
                let total = st.blocks * st.block_size;
                let free = st.blocks_free * st.block_size;
                let name = match mount {
                    "/" => "System", "/boot" => "Boot", "/home" => "Home",
                    m if m.starts_with("/media/") || m.starts_with("/mnt/") || m.starts_with("/run/media/") =>
                        m.rsplit('/').next().unwrap_or("Drive"),
                    _ => continue,
                };
                self.disks.push(DiskInfo { name: name.into(), mount: mount.into(), total, used: total.saturating_sub(free) });
            }
        }
        self.disks.sort_by_key(|d| match d.mount.as_str() { "/" => 0, "/home" => 1, "/boot" => 2, _ => 3 });
    }

    fn poll_network(&mut self, now: Instant) {
        let Ok(dev) = std::fs::read_to_string("/proc/net/dev") else { return };
        let (mut rx, mut tx) = (0u64, 0u64);
        for line in dev.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else { continue };
            if iface.trim() == "lo" { continue; }
            let n: Vec<u64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if n.len() >= 9 { rx += n[0]; tx += n[8]; }
        }
        if self.prev_rx > 0 {
            let dt = now.duration_since(self.last_fast).as_secs_f64().max(0.1);
            self.rx_rate = rx.saturating_sub(self.prev_rx) as f64 / dt;
            self.tx_rate = tx.saturating_sub(self.prev_tx) as f64 / dt;
        }
        self.prev_rx = rx; self.prev_tx = tx;
    }

    fn poll_processes(&mut self) {
        let cpu_total = self.prev_total;
        let Ok(proc_dir) = std::fs::read_dir("/proc") else { return };
        let mut new_times = std::collections::HashMap::new();
        let mut raw: Vec<ProcInfo> = Vec::new();
        for entry in proc_dir.flatten() {
            let name = entry.file_name();
            let Some(s) = name.to_str() else { continue };
            let Ok(pid) = s.parse::<u32>() else { continue };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
            let Some(c0) = stat.find('(') else { continue };
            let Some(c1) = stat.rfind(')') else { continue };
            let comm = stat[c0 + 1..c1].to_string();
            let fields: Vec<&str> = stat[c1 + 2..].split_whitespace().collect();
            if fields.len() < 22 { continue; }
            let utime: u64 = fields[11].parse().unwrap_or(0);
            let stime: u64 = fields[12].parse().unwrap_or(0);
            let proc_time = utime + stime;
            let mem_kb = fields[21].parse::<u64>().unwrap_or(0) * 4;
            let cpu_pct = self.prev_proc_times.get(&pid).map_or(0.0, |&(pt, pct)| {
                if cpu_total > pct { (proc_time.saturating_sub(pt) as f64 / (cpu_total - pct) as f64 * 100.0) as f32 } else { 0.0 }
            });
            new_times.insert(pid, (proc_time, cpu_total));
            raw.push(ProcInfo { pid, name: comm, cpu_pct, mem_kb });
        }
        self.prev_proc_times = new_times;
        // Always collect top 30 by CPU first
        raw.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal));
        raw.truncate(MAX_PROCS);
        // Then re-sort for display
        match self.sort_column {
            SortColumn::Cpu => raw.sort_by(|a, b| a.cpu_pct.partial_cmp(&b.cpu_pct).unwrap_or(std::cmp::Ordering::Equal)),
            SortColumn::Mem => raw.sort_by(|a, b| a.mem_kb.cmp(&b.mem_kb)),
            SortColumn::Name => raw.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortColumn::Pid => raw.sort_by_key(|p| p.pid),
        }
        if !self.sort_ascending { raw.reverse(); }
        self.procs = raw;
    }

    // ── Right-click on process row ──────────────────────────────────

    pub fn on_right_click(&mut self, ix: &InteractionContext, phys_x: f32, phys_y: f32) -> bool {
        if let Some(zone) = ix.zone_at(phys_x, phys_y) {
            if zone >= ZONE_PROC_BASE && zone < ZONE_PROC_BASE + MAX_PROCS as u32 {
                let idx = (zone - ZONE_PROC_BASE) as usize;
                if let Some(proc) = self.procs.get(idx) {
                    self.right_clicked_proc = Some((proc.name.clone(), proc.pid));
                    return true;
                }
            }
        }
        false
    }

    /// Check if a left-click hit a kill button on a process row.
    pub fn on_kill_click(&mut self, ix: &InteractionContext, phys_x: f32, phys_y: f32) -> bool {
        if let Some(zone) = ix.zone_at(phys_x, phys_y) {
            if zone >= ZONE_KILL_BASE && zone < ZONE_KILL_BASE + MAX_PROCS as u32 {
                let idx = (zone - ZONE_KILL_BASE) as usize;
                if let Some(proc) = self.procs.get(idx) {
                    kill_pid(proc.pid);
                    return true;
                }
            }
        }
        false
    }

    /// Handle click on a sort column header.
    pub fn on_sort_click(&mut self, ix: &InteractionContext, phys_x: f32, phys_y: f32) -> bool {
        if let Some(zone) = ix.zone_at(phys_x, phys_y) {
            let col = match zone {
                ZONE_SORT_NAME => Some(SortColumn::Name),
                ZONE_SORT_CPU => Some(SortColumn::Cpu),
                ZONE_SORT_MEM => Some(SortColumn::Mem),
                ZONE_SORT_PID => Some(SortColumn::Pid),
                _ => None,
            };
            if let Some(col) = col {
                if self.sort_column == col {
                    self.sort_ascending = !self.sort_ascending;
                } else {
                    self.sort_column = col;
                    // Default descending for CPU/Mem, ascending for Name/Pid
                    self.sort_ascending = matches!(col, SortColumn::Name | SortColumn::Pid);
                }
                return true;
            }
        }
        false
    }

    // ── Pinning ─────────────────────────────────────────────────────

    pub fn pin_right_clicked(&mut self) {
        if let Some((name, pid)) = self.right_clicked_proc.take() {
            if self.pinned.iter().any(|p| p.name == name) { return; }
            self.pinned.push(PinnedProcess {
                name, pid, cpu_pct: 0.0, mem_kb: 0,
                cpu_history: [0.0; SPARKLINE_LEN], history_idx: 0,
                warning_phase: 0.0,
            });
        }
    }

    pub fn unpin(&mut self, name: &str) {
        self.pinned.retain(|p| p.name != name);
    }

    /// Kill the right-clicked process and unpin if pinned.
    pub fn kill_right_clicked(&mut self) {
        if let Some((ref name, pid)) = self.right_clicked_proc {
            kill_pid(pid);
            let name = name.clone();
            self.unpin(&name);
        }
        self.right_clicked_proc = None;
    }

    fn update_pinned(&mut self) {
        for pinned in &mut self.pinned {
            if let Some(proc) = self.procs.iter().find(|p| p.name == pinned.name) {
                pinned.cpu_pct = proc.cpu_pct;
                pinned.mem_kb = proc.mem_kb;
                pinned.pid = proc.pid;
            } else {
                pinned.cpu_pct = 0.0;
                pinned.mem_kb = 0;
            }
            // Record sparkline sample
            pinned.cpu_history[pinned.history_idx] = pinned.cpu_pct;
            pinned.history_idx = (pinned.history_idx + 1) % SPARKLINE_LEN;
            // Warning pulse: ramp up when hot, decay when cool
            if pinned.cpu_pct > 80.0 {
                pinned.warning_phase = (pinned.warning_phase + 0.15).min(1.0);
            } else {
                pinned.warning_phase *= 0.85;
                if pinned.warning_phase < 0.01 { pinned.warning_phase = 0.0; }
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

pub(crate) fn parse_cpu_line(line: &str) -> (u64, u64) {
    let n: Vec<u64> = line.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
    (n.iter().sum(), n.get(3).copied().unwrap_or(0) + n.get(4).copied().unwrap_or(0))
}

pub(crate) fn parse_kb(s: &str) -> u64 { s.trim().trim_end_matches("kB").trim().parse().unwrap_or(0) }

pub(crate) fn format_bytes_kb(kb: u64) -> String {
    let gb = kb as f64 / (1024.0 * 1024.0);
    if gb >= 1.0 { format!("{:.1} GB", gb) } else { format!("{} MB", kb / 1024) }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 1.0 { format!("{:.0} GB", gb) } else { format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0)) }
}

pub(crate) fn format_rate(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 { format!("{:.1} MB/s", bps / (1024.0 * 1024.0)) }
    else if bps >= 1024.0 { format!("{:.0} KB/s", bps / 1024.0) }
    else { format!("{:.0} B/s", bps) }
}

fn find_thermal_zone() -> Option<PathBuf> {
    let targets = ["x86_pkg_temp", "TCPU", "coretemp", "acpitz"];
    for i in 0..20 {
        let base = PathBuf::from(format!("/sys/class/thermal/thermal_zone{i}"));
        if let Ok(t) = std::fs::read_to_string(base.join("type")) {
            let t = t.trim();
            if targets.contains(&t) { return Some(base); }
        }
    }
    None
}

fn kill_pid(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGTERM); }
}

pub(crate) struct StatVfs { pub(crate) block_size: u64, pub(crate) blocks: u64, pub(crate) blocks_free: u64 }

pub(crate) fn statvfs(path: &str) -> Option<StatVfs> {
    use std::ffi::CString; use std::mem::MaybeUninit;
    extern "C" { fn statvfs(path: *const i8, buf: *mut LibcStatvfs) -> i32; }
    #[repr(C)]
    struct LibcStatvfs {
        f_bsize: u64, f_frsize: u64, f_blocks: u64, f_bfree: u64, f_bavail: u64,
        f_files: u64, f_ffree: u64, f_favail: u64, f_fsid: u64, f_flag: u64, f_namemax: u64,
        __spare: [i32; 6],
    }
    let c_path = CString::new(path).ok()?;
    let mut buf = MaybeUninit::<LibcStatvfs>::uninit();
    let ret = unsafe { statvfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if ret != 0 { return None; }
    let buf = unsafe { buf.assume_init() };
    Some(StatVfs { block_size: buf.f_frsize, blocks: buf.f_blocks, blocks_free: buf.f_bavail })
}

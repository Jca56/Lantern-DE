//! System monitor tab — CPU, RAM, disk, network metrics + process list.

use std::path::PathBuf;
use std::time::Instant;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::scroll::{ScrollArea, Scrollbar};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

const POLL_FAST_MS: u64 = 2_000;
const POLL_DISK_MS: u64 = 10_000;
const MAX_PROCS: usize = 30;

const SECTION_FONT: f32 = 20.0;
const VALUE_FONT: f32 = 18.0;
const CORE_FONT: f32 = 16.0;
const PROC_FONT: f32 = 18.0;
const BAR_H: f32 = 14.0;
const CORE_BAR_H: f32 = 10.0;
const SECTION_GAP: f32 = 18.0;
const LINE_GAP: f32 = 6.0;
const BAR_RADIUS: f32 = 5.0;
const PROC_LINE_H: f32 = 26.0;
const GAUGE_R: f32 = 60.0;
const GAUGE_THICK: f32 = 10.0;

pub(crate) const ZONE_CORES_TOGGLE: u32 = 0xBF_F000;

pub struct SystemMonitor {
    prev_total: u64, prev_idle: u64, cpu_pct: f32,
    per_core: Vec<f32>, prev_cores: Vec<(u64, u64)>,
    mem_total_kb: u64, mem_avail_kb: u64, swap_total_kb: u64, swap_used_kb: u64,
    disks: Vec<DiskInfo>,
    prev_rx: u64, prev_tx: u64, rx_rate: f64, tx_rate: f64,
    cpu_temp: u32,
    thermal_zone: Option<PathBuf>,
    procs: Vec<ProcInfo>,
    prev_proc_times: std::collections::HashMap<u32, (u64, u64)>,
    last_fast: Instant, last_disk: Instant, first_tick: bool,
    pub scroll_offset: f32,
    pub cores_expanded: bool,
}

struct DiskInfo { name: String, mount: String, total: u64, used: u64 }
struct ProcInfo { pid: u32, name: String, cpu_pct: f32, mem_kb: u64 }

impl SystemMonitor {
    pub fn new() -> Self {
        Self {
            prev_total: 0, prev_idle: 0, cpu_pct: 0.0,
            per_core: Vec::new(), prev_cores: Vec::new(),
            mem_total_kb: 0, mem_avail_kb: 0, swap_total_kb: 0, swap_used_kb: 0,
            disks: Vec::new(),
            prev_rx: 0, prev_tx: 0, rx_rate: 0.0, tx_rate: 0.0,
            cpu_temp: 0, thermal_zone: find_thermal_zone(),
            procs: Vec::new(), prev_proc_times: std::collections::HashMap::new(),
            last_fast: Instant::now() - std::time::Duration::from_secs(10),
            last_disk: Instant::now() - std::time::Duration::from_secs(60),
            first_tick: true, scroll_offset: 0.0, cores_expanded: false,
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
        raw.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal));
        raw.truncate(MAX_PROCS);
        self.procs = raw;
    }

    // ── Drawing ──────────────────────────────────────────────────────

    fn content_height(&self, scale: f32) -> f32 {
        let s = scale;
        let sg = SECTION_GAP * s; let lg = LINE_GAP * s;
        let sf = SECTION_FONT * s; let vf = VALUE_FONT * s; let cf = CORE_FONT * s;
        let bh = BAR_H * s; let cbh = CORE_BAR_H * s;
        let mut h = 16.0 * s;
        // Gauge row (CPU+Temp + RAM+Swap circles) + label + detail + padding
        h += (GAUGE_R * 2.0) * s + 10.0 * s + sf + cf + 24.0 * s;
        // Cores toggle (uses section font + padding)
        h += sf + 16.0 * s;
        if self.cores_expanded {
            h += ((self.per_core.len() + 1) / 2) as f32 * (cf + cbh + lg + 2.0 * s);
        }
        h += sg;
        for _ in &self.disks { h += vf + lg + bh + lg; }
        h += sg;
        h += sf + lg + cbh + sg; // net
        h += sf + lg + PROC_LINE_H * s; // proc header + column header
        h += self.procs.len() as f32 * PROC_LINE_H * s;
        h + 16.0 * s
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        area: Rect, scale: f32, screen_w: u32, screen_h: u32,
    ) {
        let pad = 16.0 * scale;
        let sf = SECTION_FONT * scale; let vf = VALUE_FONT * scale;
        let cf = CORE_FONT * scale; let pf = PROC_FONT * scale;
        let bh = BAR_H * scale; let cbh = CORE_BAR_H * scale;
        let sg = SECTION_GAP * scale; let lg = LINE_GAP * scale;
        let br = BAR_RADIUS * scale; let plh = PROC_LINE_H * scale;

        let content_h = self.content_height(scale);
        let gr = Rect::new(area.x + pad, area.y + pad, area.w - pad * 2.0, area.h - pad * 2.0);
        let scroll = ScrollArea::new(gr, content_h, &mut self.scroll_offset);
        scroll.begin(painter);

        let x = gr.x; let w = gr.w;
        let mut y = scroll.content_y();
        let clip = [gr.x, gr.y, gr.w, gr.h];

        let accent = palette.accent;
        let ram_c = Color::from_rgb8(59, 130, 246);
        let swap_c = Color::from_rgb8(168, 85, 247);
        let disk_c = Color::from_rgb8(34, 197, 94);
        let net_c = Color::from_rgb8(236, 72, 153);
        let proc_c = Color::from_rgb8(239, 68, 68);
        let track = palette.muted.with_alpha(0.15);

        // ── Dual-ring gauges: CPU+Temp (left), RAM+Swap (right) ──
        let gauge_r = GAUGE_R * scale;
        let thick = GAUGE_THICK * scale;
        let gauge_cy = y + gauge_r + 4.0 * scale;
        let quarter_w = w * 0.25;

        // CPU temp color
        let temp_color = match self.cpu_temp {
            0..=59 => Color::from_rgb8(34, 197, 94),   // green
            60..=79 => Color::from_rgb8(234, 179, 8),   // yellow
            _ => Color::from_rgb8(239, 68, 68),          // red
        };
        let temp_pct = (self.cpu_temp as f32 / 100.0).clamp(0.0, 1.0);
        let temp_sub = if self.cpu_temp > 0 { format!("{}C", self.cpu_temp) } else { String::new() };

        // CPU (outer) + Temp (inner)
        let cpu_cx = x + quarter_w;
        self.draw_dual_gauge(painter, text, cpu_cx, gauge_cy, gauge_r, thick,
            self.cpu_pct / 100.0, accent,
            temp_pct, temp_color,
            track, &format!("{:.0}%", self.cpu_pct), accent,
            "CPU", &temp_sub, scale, clip);

        // RAM (outer) + Swap (inner)
        let mem_used = self.mem_total_kb.saturating_sub(self.mem_avail_kb);
        let mem_pct = if self.mem_total_kb > 0 { mem_used as f32 / self.mem_total_kb as f32 } else { 0.0 };
        let swap_pct = if self.swap_total_kb > 0 { self.swap_used_kb as f32 / self.swap_total_kb as f32 } else { 0.0 };
        let ram_cx = x + w - quarter_w;
        let ram_detail = format!("{} / {}", format_bytes_kb(mem_used), format_bytes_kb(self.mem_total_kb));

        self.draw_dual_gauge(painter, text, ram_cx, gauge_cy, gauge_r, thick,
            mem_pct, ram_c,
            swap_pct, swap_c,
            track, &format!("{:.0}%", mem_pct * 100.0), ram_c,
            "RAM", "Swap", scale, clip);

        // RAM detail below "RAM" label
        let label_bottom = gauge_cy + gauge_r + 10.0 * scale + sf;
        let rw = text.measure_width(&ram_detail, cf);
        text.queue_clipped(&ram_detail, cf, ram_cx - rw * 0.5, label_bottom + 4.0 * scale, palette.text_secondary, w, clip);

        // Advance past gauges + label + detail + padding
        y += gauge_r * 2.0 + 10.0 * scale + sf + cf + 24.0 * scale;

        // Cores dropdown toggle
        let arrow = if self.cores_expanded { "v" } else { ">" };
        let cores_label = format!("{arrow}  Cores ({})", self.per_core.len());
        let toggle_h = sf + pad;
        let toggle_rect = Rect::new(x, y, w, toggle_h);
        let ts = ix.add_zone(ZONE_CORES_TOGGLE, toggle_rect);
        if ts.is_hovered() {
            painter.rect_filled(toggle_rect, 6.0 * scale, palette.surface_2);
        }
        let toggle_color = if ts.is_hovered() { palette.text } else { palette.text_secondary };
        text.queue_clipped(&cores_label, sf, x + 8.0 * scale, y + (toggle_h - sf) * 0.5, toggle_color, w, clip);
        y += toggle_h;

        if self.cores_expanded {
            let cw = (w - pad) * 0.5;
            let cores = self.per_core.len();
            for row in 0..(cores + 1) / 2 {
                for col in 0..2 {
                    let idx = row * 2 + col;
                    if idx >= cores { break; }
                    let cx = x + col as f32 * (cw + pad);
                    text.queue_clipped(&format!("Core {} {:.0}%", idx, self.per_core[idx]), cf, cx, y, palette.text_secondary, cw, clip);
                    self.draw_bar(painter, cx, y + cf + 2.0 * scale, cw, cbh, br, self.per_core[idx] / 100.0, accent.with_alpha(0.7), track);
                }
                y += cf + cbh + lg + 2.0 * scale;
            }
        }
        y += sg;

        // Disks
        for disk in &self.disks {
            let pct = if disk.total > 0 { disk.used as f32 / disk.total as f32 } else { 0.0 };
            text.queue_clipped(&format!("{}  {} / {}", disk.name, format_bytes(disk.used), format_bytes(disk.total)),
                vf, x, y, palette.text, w, clip);
            y += vf + lg;
            self.draw_bar(painter, x, y, w, bh, br, pct, disk_c, track);
            y += bh + lg;
        }
        y += sg;

        // Network
        text.queue_clipped(&format!("Net  v {}  ^ {}", format_rate(self.rx_rate), format_rate(self.tx_rate)),
            sf, x, y, palette.text, w, clip);
        y += sf + lg;
        let max_rate = 100.0 * 1024.0 * 1024.0;
        let hw = (w - pad) * 0.5;
        self.draw_bar(painter, x, y, hw, cbh, br, (self.rx_rate as f32 / max_rate).min(1.0), net_c, track);
        self.draw_bar(painter, x + hw + pad, y, hw, cbh, br, (self.tx_rate as f32 / max_rate).min(1.0), net_c.with_alpha(0.7), track);
        y += cbh + sg;

        // Processes
        text.queue_clipped("Processes", sf, x, y, palette.text, w, clip);
        y += sf + lg;

        let name_w = w * 0.45; let cpu_x = x + name_w; let cpu_w = w * 0.2;
        let mem_x = cpu_x + cpu_w; let mem_w = w * 0.2;
        let pid_x = mem_x + mem_w; let pid_w = w - name_w - cpu_w - mem_w;

        text.queue_clipped("Name", pf, x, y, palette.text_secondary, name_w, clip);
        text.queue_clipped("CPU%", pf, cpu_x, y, palette.text_secondary, cpu_w, clip);
        text.queue_clipped("Mem", pf, mem_x, y, palette.text_secondary, mem_w, clip);
        text.queue_clipped("PID", pf, pid_x, y, palette.text_secondary, pid_w, clip);
        y += plh;

        for p in &self.procs {
            let cc = if p.cpu_pct > 50.0 { proc_c } else if p.cpu_pct > 10.0 { palette.warning } else { palette.text_secondary };
            let nm = if p.name.len() > 22 { format!("{}...", &p.name[..20]) } else { p.name.clone() };
            text.queue_clipped(&nm, pf, x, y, palette.text, name_w, clip);
            text.queue_clipped(&format!("{:.1}", p.cpu_pct), pf, cpu_x, y, cc, cpu_w, clip);
            text.queue_clipped(&format_bytes_kb(p.mem_kb), pf, mem_x, y, palette.text_secondary, mem_w, clip);
            text.queue_clipped(&p.pid.to_string(), pf, pid_x, y, palette.muted, pid_w, clip);
            y += plh;
        }
        let _ = y;

        scroll.end(painter);
        if scroll.is_scrollable() {
            let sb = Scrollbar::new(&gr, content_h, self.scroll_offset);
            sb.draw(painter, InteractionState::Idle, palette);
        }
    }

    /// Draw a dual-ring gauge: outer ring + inner ring with labels.
    #[allow(clippy::too_many_arguments)]
    fn draw_dual_gauge(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        cx: f32, cy: f32, r: f32, thick: f32,
        outer_pct: f32, outer_color: Color,
        inner_pct: f32, inner_color: Color,
        track_color: Color,
        center_text: &str, center_color: Color,
        label: &str, sub_label: &str,
        scale: f32, clip: [f32; 4],
    ) {
        let pi = std::f32::consts::PI;
        let gap = 4.0 * scale;
        let inner_r = r - thick - gap;
        let inner_thick = thick * 0.7;
        let start = -pi * 0.5;
        let full = pi * 2.0 - 0.001; // slight less than full to avoid wrap glitch

        // Outer ring: track + fill
        painter.arc(cx, cy, r, start, full, thick, r - thick, track_color);
        if outer_pct > 0.001 {
            painter.arc(cx, cy, r, start, full * outer_pct.clamp(0.0, 1.0), thick, r - thick, outer_color);
        }

        // Inner ring: track + fill
        painter.arc(cx, cy, inner_r, start, full, inner_thick, inner_r - inner_thick, track_color);
        if inner_pct > 0.001 {
            painter.arc(cx, cy, inner_r, start, full * inner_pct.clamp(0.0, 1.0), inner_thick, inner_r - inner_thick, inner_color);
        }

        // Center text (percentage) — nudge up a bit more
        let vf = 24.0 * scale;
        let vw = text.measure_width(center_text, vf);
        text.queue_clipped(center_text, vf, cx - vw * 0.5, cy - vf * 0.8, center_color, vw + 4.0, clip);

        // Sub text inside circle (below percentage, with padding)
        if !sub_label.is_empty() {
            let sf = CORE_FONT * scale;
            let sw = text.measure_width(sub_label, sf);
            text.queue_clipped(sub_label, sf, cx - sw * 0.5, cy + vf * 0.1, inner_color.with_alpha(0.7), sw + 4.0, clip);
        }

        // Label below circle (more padding below ring)
        let lf = SECTION_FONT * scale;
        let lw = text.measure_width(label, lf);
        text.queue_clipped(label, lf, cx - lw * 0.5, cy + r + 10.0 * scale, outer_color, lw + 4.0, clip);
    }

    fn draw_bar(&self, painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, r: f32, pct: f32, fill: Color, track: Color) {
        painter.rect_filled(Rect::new(x, y, w, h), r, track);
        if pct > 0.001 {
            painter.rect_filled(Rect::new(x, y, (w * pct.clamp(0.0, 1.0)).max(h), h), r, fill);
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse_cpu_line(line: &str) -> (u64, u64) {
    let n: Vec<u64> = line.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
    (n.iter().sum(), n.get(3).copied().unwrap_or(0) + n.get(4).copied().unwrap_or(0))
}

fn parse_kb(s: &str) -> u64 { s.trim().trim_end_matches("kB").trim().parse().unwrap_or(0) }

fn format_bytes_kb(kb: u64) -> String {
    let gb = kb as f64 / (1024.0 * 1024.0);
    if gb >= 1.0 { format!("{:.1} GB", gb) } else { format!("{} MB", kb / 1024) }
}

fn format_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 1.0 { format!("{:.0} GB", gb) } else { format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0)) }
}

fn format_rate(bps: f64) -> String {
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

struct StatVfs { block_size: u64, blocks: u64, blocks_free: u64 }

fn statvfs(path: &str) -> Option<StatVfs> {
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

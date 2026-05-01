use std::collections::HashMap;
use std::fs;
use lntrn_render::{Color, Painter, Rect, TextRenderer};

const TEXT_PRIMARY: Color   = Color::rgb(0.85, 0.85, 0.85);
const TEXT_MUTED: Color     = Color::rgb(0.55, 0.55, 0.55);
const ACCENT: Color         = Color::rgb(0.78, 0.45, 0.06); // sRGB ~232,180,75 (gold)
const HEADER_BG: Color      = Color::rgba(1.0, 1.0, 1.0, 0.05);
const ROW_HOVER: Color      = Color::rgba(1.0, 1.0, 1.0, 0.05);
const BORDER: Color         = Color::rgba(1.0, 1.0, 1.0, 0.06);
const HIGH_CPU: Color       = Color::rgb(0.90, 0.35, 0.35);
const MED_CPU: Color        = Color::rgb(0.85, 0.72, 0.25);
const ROW_SELECTED: Color   = Color::rgba(0.78, 0.45, 0.06, 0.22); // gold tint
const KILL_BG: Color        = Color::rgb(0.56, 0.013, 0.013);
const KILL_HOVER: Color     = Color::rgba(0.75, 0.10, 0.10, 1.0);
const KILL_TEXT: Color      = Color::rgb(0.95, 0.92, 0.92);

pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,    // % of total CPU power (all cores sum to 100), matches the bar
    pub mem_pct: f32,
    pub mem_mb: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey { Name, Cpu, MemPct, Mem, Pid }

#[derive(Clone, Copy)]
pub struct Sort {
    pub key: SortKey,
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self { Self { key: SortKey::Cpu, desc: true } }
}

impl Sort {
    /// Click on a header: same column toggles direction; different column resets to desc.
    pub fn toggle(&mut self, key: SortKey) {
        if self.key == key { self.desc = !self.desc; }
        else { self.key = key; self.desc = true; }
    }
}

pub fn resort(procs: &mut [ProcessEntry], sort: Sort) {
    apply_sort(procs, sort);
}

fn apply_sort(procs: &mut [ProcessEntry], sort: Sort) {
    use std::cmp::Ordering::Equal;
    procs.sort_by(|a, b| {
        let cmp = match sort.key {
            SortKey::Name => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
            SortKey::Cpu => a.cpu.partial_cmp(&b.cpu).unwrap_or(Equal),
            SortKey::MemPct => a.mem_pct.partial_cmp(&b.mem_pct).unwrap_or(Equal),
            SortKey::Mem => a.mem_mb.partial_cmp(&b.mem_mb).unwrap_or(Equal),
            SortKey::Pid => a.pid.cmp(&b.pid),
        };
        if sort.desc { cmp.reverse() } else { cmp }
    });
}

/// Tracks previous /proc/PID/stat jiffies so we can compute real CPU% deltas.
pub struct ProcessSampler {
    prev_proc: HashMap<u32, u64>,
    prev_total: u64,
}

impl ProcessSampler {
    pub fn new() -> Self {
        Self {
            prev_proc: HashMap::new(),
            prev_total: read_total_cpu_jiffies(),
        }
    }

    pub fn sample(&mut self, sort: Sort) -> Vec<ProcessEntry> {
        let mut procs = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else { return procs };

        let total_mem_kb = read_total_mem_kb();
        let total_now = read_total_cpu_jiffies();
        let total_delta = total_now.saturating_sub(self.prev_total).max(1);
        let mut next_proc: HashMap<u32, u64> = HashMap::new();

        for entry in entries.flatten() {
            let name = entry.file_name();
            let pid_str = name.to_string_lossy();
            let Ok(pid) = pid_str.parse::<u32>() else { continue };

            let stat = match fs::read_to_string(format!("/proc/{}/stat", pid)) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let Some((comm, post)) = parse_stat_comm(&stat) else { continue };
            if comm.is_empty() { continue; }

            // Fields after `) `: state(0) ppid(1) ... utime(11) stime(12)
            let mut tokens = post.split_whitespace();
            let utime: u64 = tokens.nth(11).and_then(|s| s.parse().ok()).unwrap_or(0);
            let stime: u64 = tokens.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let proc_jiffies = utime + stime;

            let proc_delta = match self.prev_proc.get(&pid) {
                Some(prev) => proc_jiffies.saturating_sub(*prev),
                None => 0,
            };
            // % of total CPU power across all cores (matches the bar's reading).
            let cpu = (proc_delta as f64 / total_delta as f64) as f32 * 100.0;
            next_proc.insert(pid, proc_jiffies);

            // Memory from /proc/PID/statm (pages)
            let statm = fs::read_to_string(format!("/proc/{}/statm", pid)).unwrap_or_default();
            let rss_pages: f64 = statm.split_whitespace().nth(1)
                .and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let mem_kb = rss_pages * 4.0;
            let mem_mb = (mem_kb / 1024.0) as f32;
            let mem_pct = (mem_kb / total_mem_kb * 100.0) as f32;

            procs.push(ProcessEntry { pid, name: comm, cpu, mem_pct, mem_mb });
        }

        self.prev_proc = next_proc;
        self.prev_total = total_now;

        apply_sort(&mut procs, sort);
        procs.truncate(50);
        procs
    }
}

/// Parse comm out of /proc/PID/stat — comm is in parens and may contain spaces.
/// Returns (comm, rest_after_closing_paren).
fn parse_stat_comm(stat: &str) -> Option<(String, &str)> {
    let lparen = stat.find('(')?;
    let rparen = stat.rfind(')')?;
    if rparen <= lparen { return None; }
    let comm = stat[lparen + 1..rparen].to_string();
    let rest = stat.get(rparen + 1..)?.trim_start();
    Some((comm, rest))
}

fn read_total_cpu_jiffies() -> u64 {
    let stat = fs::read_to_string("/proc/stat").unwrap_or_default();
    let Some(line) = stat.lines().next() else { return 0 };
    if !line.starts_with("cpu ") && !line.starts_with("cpu\t") { return 0; }
    line.split_whitespace().skip(1)
        .filter_map(|s| s.parse::<u64>().ok())
        .sum()
}

fn read_total_mem_kb() -> f64 {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    info.lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0)
}


fn in_rect(cx: f32, cy: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    cx >= x && cx <= x + w && cy >= y && cy <= y + h
}

pub enum ProcessAction {
    None,
    Select(u32),
    Kill(u32),
    SortBy(SortKey),
}

const TOOLBAR_H: f32 = 50.0;
const KILL_BTN_W: f32 = 180.0;
const KILL_BTN_H: f32 = 38.0;

fn kill_button_rect(s: f32, top_y: f32, wf: f32) -> (f32, f32, f32, f32) {
    let pad = 32.0 * s;
    let bw = KILL_BTN_W * s;
    let bh = KILL_BTN_H * s;
    let bx = wf - pad - bw;
    let by = top_y + 10.0 * s + (TOOLBAR_H * s - bh) * 0.5;
    (bx, by, bw, bh)
}

fn column_x(wf: f32, pad: f32) -> [(SortKey, f32); 5] {
    [
        (SortKey::Name,   pad),
        (SortKey::Cpu,    wf * 0.46),
        (SortKey::MemPct, wf * 0.60),
        (SortKey::Mem,    wf * 0.72),
        (SortKey::Pid,    wf * 0.86),
    ]
}

/// Hit-test a click in the processes view. Selects rows, fires the kill button,
/// or changes sort order when a header column is clicked.
pub fn handle_click(
    cx: f32, cy: f32, s: f32, top_y: f32, wf: f32,
    procs: &[ProcessEntry], selected_pid: Option<u32>,
) -> ProcessAction {
    // Kill button (only active when something is selected)
    if let Some(pid) = selected_pid {
        let (bx, by, bw, bh) = kill_button_rect(s, top_y, wf);
        if in_rect(cx, cy, bx, by, bw, bh) {
            return ProcessAction::Kill(pid);
        }
    }

    let pad = 32.0 * s;
    let row_h = 40.0 * s;
    let header_h = 42.0 * s;
    let toolbar_h = TOOLBAR_H * s;
    let hy = top_y + 10.0 * s + toolbar_h;

    // Header row hit-test → SortBy
    if cy >= hy && cy < hy + header_h && cx >= pad && cx < wf - pad {
        let cols = column_x(wf, pad);
        // Pick the column whose left edge is the largest one <= cx
        let mut hit = cols[0].0;
        for (key, x) in cols.iter() {
            if cx >= *x { hit = *key; }
        }
        return ProcessAction::SortBy(hit);
    }

    let list_y = hy + header_h + 4.0 * s;
    for (i, proc) in procs.iter().enumerate() {
        let y = list_y + i as f32 * row_h;
        if in_rect(cx, cy, pad, y, wf - pad * 2.0, row_h) {
            return ProcessAction::Select(proc.pid);
        }
    }
    ProcessAction::None
}

/// Send SIGTERM to the given pid. Returns true on success.
pub fn kill_pid(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) == 0 }
}

pub fn draw(
    p: &mut Painter, t: &mut TextRenderer,
    cx: f32, cy: f32, s: f32, top_y: f32,
    procs: &[ProcessEntry], selected_pid: Option<u32>, sort: Sort,
    wf: f32, _hf: f32, sw: u32, sh: u32,
) {
    let pad = 32.0 * s;
    let row_h = 40.0 * s;
    let header_h = 42.0 * s;
    let font = 22.0 * s;
    let header_font = 20.0 * s;
    let toolbar_h = TOOLBAR_H * s;

    // ── Toolbar (top): selected info + kill button ─────────────────────
    let tb_y = top_y + 10.0 * s;
    if let Some(pid) = selected_pid {
        let label = procs.iter().find(|p| p.pid == pid)
            .map(|p| format!("Selected: {} ({})", p.name, p.pid))
            .unwrap_or_else(|| format!("Selected PID {}", pid));
        t.queue(&label, font, pad, tb_y + (toolbar_h - font) * 0.5,
            TEXT_PRIMARY, wf, sw, sh);

        // Kill button
        let (bx, by, bw, bh) = kill_button_rect(s, top_y, wf);
        let hov = in_rect(cx, cy, bx, by, bw, bh);
        let bg = if hov { KILL_HOVER } else { KILL_BG };
        p.rect_filled(Rect::new(bx, by, bw, bh), 8.0 * s, bg);
        let txt = "Kill Process";
        let txt_sz = 20.0 * s;
        let tw = txt.len() as f32 * txt_sz * 0.55;
        t.queue(txt, txt_sz, bx + (bw - tw) * 0.5,
            by + (bh - txt_sz) * 0.5, KILL_TEXT, wf, sw, sh);
    } else {
        t.queue("Click a process to select it", font, pad,
            tb_y + (toolbar_h - font) * 0.5, TEXT_MUTED, wf, sw, sh);
    }

    // Column positions (must match column_x())
    let col_name = pad;
    let col_cpu = wf * 0.46;
    let col_mempct = wf * 0.60;
    let col_mem = wf * 0.72;
    let col_pid = wf * 0.86;

    // Header row (below toolbar) — clickable, with sort indicator
    let hy = tb_y + toolbar_h;
    p.rect_filled(Rect::new(pad, hy, wf - pad * 2.0, header_h), 8.0 * s, HEADER_BG);

    let arrow = if sort.desc { " v" } else { " ^" };
    let label = |key: SortKey, base: &'static str| -> String {
        if sort.key == key { format!("{}{}", base, arrow) } else { base.to_string() }
    };
    t.queue(&label(SortKey::Name, "Name"), header_font, col_name + 12.0 * s, hy + 11.0 * s, ACCENT, wf, sw, sh);
    t.queue(&label(SortKey::Cpu, "CPU%"), header_font, col_cpu + 8.0 * s, hy + 11.0 * s, ACCENT, wf, sw, sh);
    t.queue(&label(SortKey::MemPct, "MEM%"), header_font, col_mempct + 8.0 * s, hy + 11.0 * s, ACCENT, wf, sw, sh);
    t.queue(&label(SortKey::Mem, "MEM"), header_font, col_mem + 8.0 * s, hy + 11.0 * s, ACCENT, wf, sw, sh);
    t.queue(&label(SortKey::Pid, "PID"), header_font, col_pid + 8.0 * s, hy + 11.0 * s, ACCENT, wf, sw, sh);

    // Process rows
    let list_y = hy + header_h + 4.0 * s;
    for (i, proc) in procs.iter().enumerate() {
        let y = list_y + i as f32 * row_h;

        let is_selected = selected_pid == Some(proc.pid);
        let hov = in_rect(cx, cy, pad, y, wf - pad * 2.0, row_h);

        if is_selected {
            p.rect_filled(Rect::new(pad, y, wf - pad * 2.0, row_h), 4.0 * s, ROW_SELECTED);
        } else if hov {
            p.rect_filled(Rect::new(pad, y, wf - pad * 2.0, row_h), 4.0 * s, ROW_HOVER);
        }

        // Name
        t.queue(&proc.name, font, col_name + 12.0 * s, y + 9.0 * s, TEXT_PRIMARY, wf, sw, sh);

        // CPU% with color coding (% of total CPU power; sums to 100 across all procs)
        let cpu_str = format!("{:.1}%", proc.cpu);
        let cpu_color = if proc.cpu > 5.0 { HIGH_CPU }
            else if proc.cpu > 1.0 { MED_CPU }
            else { TEXT_MUTED };
        t.queue(&cpu_str, font, col_cpu + 8.0 * s, y + 9.0 * s, cpu_color, wf, sw, sh);

        // MEM%
        let mempct_str = format!("{:.1}%", proc.mem_pct);
        t.queue(&mempct_str, font, col_mempct + 8.0 * s, y + 9.0 * s, TEXT_MUTED, wf, sw, sh);

        // MEM
        let mem_str = if proc.mem_mb > 1024.0 {
            format!("{:.1} GiB", proc.mem_mb / 1024.0)
        } else {
            format!("{:.0} MiB", proc.mem_mb)
        };
        t.queue(&mem_str, font, col_mem + 8.0 * s, y + 9.0 * s, TEXT_MUTED, wf, sw, sh);

        // PID
        let pid_str = format!("{}", proc.pid);
        t.queue(&pid_str, font, col_pid + 8.0 * s, y + 9.0 * s, TEXT_MUTED, wf, sw, sh);

        // Row separator
        if i < procs.len() - 1 {
            p.rect_filled(
                Rect::new(pad + 8.0 * s, y + row_h - 1.0 * s, wf - pad * 2.0 - 16.0 * s, 1.0 * s),
                0.0, BORDER,
            );
        }
    }
}

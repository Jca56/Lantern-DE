use lntrn_render::{Color, Painter, Rect, TextRenderer};

const TEXT_PRIMARY: Color   = Color::rgb(0.88, 0.85, 0.95);
const TEXT_MUTED: Color     = Color::rgb(0.50, 0.45, 0.62);
const ACCENT_CYAN: Color    = Color::rgb(0.25, 0.65, 0.90);
const ACCENT_PINK: Color    = Color::rgb(0.90, 0.35, 0.55);
const ACCENT_GREEN: Color   = Color::rgb(0.30, 0.80, 0.50);
const GRAPH_BG: Color       = Color::rgba(0.04, 0.02, 0.10, 0.35);
const BORDER: Color         = Color::rgba(0.30, 0.20, 0.50, 0.15);

const HISTORY_LEN: usize = 60;

pub struct PerfState {
    pub cpu_history: Vec<f32>,
    pub mem_history: Vec<f32>,
}

impl PerfState {
    pub fn new() -> Self {
        Self {
            cpu_history: vec![0.0; HISTORY_LEN],
            mem_history: vec![0.0; HISTORY_LEN],
        }
    }

    pub fn update(&mut self) {
        let cpu = read_cpu_usage();
        self.cpu_history.push(cpu);
        if self.cpu_history.len() > HISTORY_LEN { self.cpu_history.remove(0); }

        let mem = read_mem_usage();
        self.mem_history.push(mem);
        if self.mem_history.len() > HISTORY_LEN { self.mem_history.remove(0); }
    }
}

fn read_cpu_usage() -> f32 {
    // Simple: read /proc/stat idle vs total
    // For a single sample, just return current load average as approximate
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let avg1: f32 = load.split_whitespace().next()
        .and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let cpus = num_cpus();
    (avg1 / cpus as f32 * 100.0).clamp(0.0, 100.0)
}

fn num_cpus() -> usize {
    std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default()
        .lines().filter(|l| l.starts_with("processor")).count().max(1)
}

fn read_mem_usage() -> f32 {
    let info = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total = 1u64;
    let mut avail = 0u64;
    for line in info.lines() {
        if line.starts_with("MemTotal:") {
            total = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        } else if line.starts_with("MemAvailable:") {
            avail = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    ((total - avail) as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
}

fn draw_graph(
    p: &mut Painter, t: &mut TextRenderer,
    x: f32, y: f32, w: f32, h: f32, s: f32,
    history: &[f32], color: Color, label: &str, current_val: &str,
    wf: f32, sw: u32, sh: u32,
) {
    // Background
    p.rect_filled(Rect::new(x, y, w, h), 10.0 * s, GRAPH_BG);
    p.rect_stroke_sdf(Rect::new(x, y, w, h), 10.0 * s, 1.0 * s, BORDER);

    // Grid lines (horizontal)
    for i in 1..4 {
        let gy = y + h * i as f32 / 4.0;
        p.rect_filled(Rect::new(x + 8.0 * s, gy, w - 16.0 * s, 1.0 * s), 0.0,
            Color::rgba(0.30, 0.20, 0.50, 0.08));
    }

    // Draw line graph
    let n = history.len();
    if n < 2 { return; }
    let graph_pad = 12.0 * s;
    let gx = x + graph_pad;
    let gw = w - graph_pad * 2.0;
    let gy = y + 36.0 * s; // leave room for label
    let gh = h - 46.0 * s;

    for i in 1..n {
        let x0 = gx + (i - 1) as f32 / (n - 1) as f32 * gw;
        let x1 = gx + i as f32 / (n - 1) as f32 * gw;
        let y0 = gy + gh * (1.0 - history[i - 1] / 100.0);
        let y1 = gy + gh * (1.0 - history[i] / 100.0);
        p.line(x0, y0, x1, y1, 2.0 * s, color);
    }

    // Fill under the line (subtle)
    let last_val = history.last().copied().unwrap_or(0.0);
    let fill_h = gh * (last_val / 100.0);
    p.rect_filled(
        Rect::new(gx, gy + gh - fill_h, gw, fill_h),
        0.0, color.with_alpha(0.08),
    );

    // Label + current value
    t.queue(label, 18.0 * s, x + 16.0 * s, y + 10.0 * s, color, wf, sw, sh);
    t.queue(current_val, 18.0 * s, x + w - 80.0 * s, y + 10.0 * s, TEXT_PRIMARY, wf, sw, sh);
}

pub fn draw(
    p: &mut Painter, t: &mut TextRenderer,
    s: f32, top_y: f32, state: &PerfState,
    wf: f32, hf: f32, sw: u32, sh: u32,
) {
    let pad = 32.0 * s;
    let graph_gap = 20.0 * s;
    let graph_w = wf - pad * 2.0;
    let graph_h = 180.0 * s;

    // CPU graph
    let cpu_y = top_y + 12.0 * s;
    if cpu_y + graph_h > hf { return; }
    let cpu_val = state.cpu_history.last().copied().unwrap_or(0.0);
    draw_graph(
        p, t, pad, cpu_y, graph_w, graph_h, s,
        &state.cpu_history, ACCENT_CYAN, "CPU",
        &format!("{:.0}%", cpu_val), wf, sw, sh,
    );

    // Memory graph
    let mem_y = cpu_y + graph_h + graph_gap;
    if mem_y + graph_h > hf { return; }
    let mem_val = state.mem_history.last().copied().unwrap_or(0.0);
    draw_graph(
        p, t, pad, mem_y, graph_w, graph_h, s,
        &state.mem_history, ACCENT_PINK, "Memory",
        &format!("{:.0}%", mem_val), wf, sw, sh,
    );

    // Battery / misc info below graphs
    let info_y = mem_y + graph_h + graph_gap;
    if info_y > hf { return; }
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let load_str = load.split_whitespace().take(3).collect::<Vec<_>>().join("  ");
    t.queue("Load Average", 16.0 * s, pad, info_y, TEXT_MUTED, wf, sw, sh);
    t.queue(&load_str, 18.0 * s, pad + 160.0 * s, info_y, ACCENT_GREEN, wf, sw, sh);
}

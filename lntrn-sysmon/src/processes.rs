use std::fs;
use lntrn_render::{Color, Painter, Rect, TextRenderer};

const TEXT_PRIMARY: Color   = Color::rgb(0.88, 0.85, 0.95);
const TEXT_MUTED: Color     = Color::rgb(0.50, 0.45, 0.62);
const ACCENT: Color         = Color::rgb(0.25, 0.65, 0.90);
const HEADER_BG: Color      = Color::rgba(0.06, 0.03, 0.12, 0.40);
const ROW_HOVER: Color      = Color::rgba(0.10, 0.06, 0.20, 0.30);
const BORDER: Color         = Color::rgba(0.30, 0.20, 0.50, 0.10);
const HIGH_CPU: Color       = Color::rgb(0.90, 0.35, 0.35);
const MED_CPU: Color        = Color::rgb(0.85, 0.72, 0.25);

pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub mem_mb: f32,
}

pub fn read_processes() -> Vec<ProcessEntry> {
    let mut procs = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else { return procs };

    // Read total CPU time for percentage calculation
    let total_mem_kb = {
        let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        info.lines()
            .find(|l| l.starts_with("MemTotal:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.0)
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        let Ok(pid) = pid_str.parse::<u32>() else { continue };

        let comm = fs::read_to_string(format!("/proc/{}/comm", pid))
            .unwrap_or_default().trim().to_string();
        if comm.is_empty() { continue; }

        // Memory from /proc/PID/statm (pages)
        let statm = fs::read_to_string(format!("/proc/{}/statm", pid)).unwrap_or_default();
        let rss_pages: f64 = statm.split_whitespace().nth(1)
            .and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let mem_kb = rss_pages * 4.0; // 4KB pages
        let mem_mb = (mem_kb / 1024.0) as f32;

        // CPU% from /proc/PID/stat (rough — just use RSS-based sort for now)
        // Real CPU% requires two samples with a time delta, so skip for v1
        let mem_pct = (mem_kb / total_mem_kb * 100.0) as f32;

        procs.push(ProcessEntry {
            pid, name: comm, cpu: mem_pct, mem_mb,
        });
    }

    // Sort by memory usage descending
    procs.sort_by(|a, b| b.mem_mb.partial_cmp(&a.mem_mb).unwrap_or(std::cmp::Ordering::Equal));
    procs.truncate(50); // top 50
    procs
}

fn in_rect(cx: f32, cy: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    cx >= x && cx <= x + w && cy >= y && cy <= y + h
}

pub fn draw(
    p: &mut Painter, t: &mut TextRenderer,
    cx: f32, cy: f32, s: f32, top_y: f32,
    procs: &[ProcessEntry], wf: f32, hf: f32, sw: u32, sh: u32,
) {
    let pad = 32.0 * s;
    let row_h = 30.0 * s;
    let header_h = 34.0 * s;
    let font = 17.0 * s;
    let header_font = 16.0 * s;

    // Column positions
    let col_name = pad;
    let col_cpu = wf * 0.50;
    let col_mem = wf * 0.65;
    let col_pid = wf * 0.82;

    // Header row
    let hy = top_y + 10.0 * s;
    p.rect_filled(Rect::new(pad, hy, wf - pad * 2.0, header_h), 8.0 * s, HEADER_BG);
    t.queue("Name", header_font, col_name + 12.0 * s, hy + 8.0 * s, ACCENT, wf, sw, sh);
    t.queue("MEM%", header_font, col_cpu + 8.0 * s, hy + 8.0 * s, ACCENT, wf, sw, sh);
    t.queue("MEM", header_font, col_mem + 8.0 * s, hy + 8.0 * s, ACCENT, wf, sw, sh);
    t.queue("PID", header_font, col_pid + 8.0 * s, hy + 8.0 * s, ACCENT, wf, sw, sh);

    // Process rows
    let list_y = hy + header_h + 4.0 * s;
    for (i, proc) in procs.iter().enumerate() {
        let y = list_y + i as f32 * row_h;
        if y + row_h > hf { break; }

        // Hover highlight
        let hov = in_rect(cx, cy, pad, y, wf - pad * 2.0, row_h);
        if hov {
            p.rect_filled(Rect::new(pad, y, wf - pad * 2.0, row_h), 4.0 * s, ROW_HOVER);
        }

        // Name
        t.queue(&proc.name, font, col_name + 12.0 * s, y + 5.0 * s, TEXT_PRIMARY, wf, sw, sh);

        // CPU% with color coding
        let cpu_str = format!("{:.1}%", proc.cpu);
        let cpu_color = if proc.cpu > 5.0 { HIGH_CPU }
            else if proc.cpu > 2.0 { MED_CPU }
            else { TEXT_MUTED };
        t.queue(&cpu_str, font, col_cpu + 8.0 * s, y + 5.0 * s, cpu_color, wf, sw, sh);

        // MEM
        let mem_str = if proc.mem_mb > 1024.0 {
            format!("{:.1} GiB", proc.mem_mb / 1024.0)
        } else {
            format!("{:.0} MiB", proc.mem_mb)
        };
        t.queue(&mem_str, font, col_mem + 8.0 * s, y + 5.0 * s, TEXT_MUTED, wf, sw, sh);

        // PID
        let pid_str = format!("{}", proc.pid);
        t.queue(&pid_str, font, col_pid + 8.0 * s, y + 5.0 * s, TEXT_MUTED, wf, sw, sh);

        // Row separator
        if i < procs.len() - 1 {
            p.rect_filled(
                Rect::new(pad + 8.0 * s, y + row_h - 1.0 * s, wf - pad * 2.0 - 16.0 * s, 1.0 * s),
                0.0, BORDER,
            );
        }
    }
}

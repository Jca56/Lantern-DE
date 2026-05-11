//! Expanded "system monitor" view: three live graphs (CPU history, RAM
//! used, network RX/TX) and a process list with per-row kill buttons.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::sysmon::proc::{format_kb, format_rate};
use crate::controls::sysmon::{ProcessRow, SysMon};

const SECTION_PAD: f32 = 24.0;
const SECTION_GAP: f32 = 14.0;
const GRAPH_HEIGHT: f32 = 80.0;

const HEADER_FONT: f32 = 18.0;
const VALUE_FONT: f32 = 22.0;
const ROW_FONT: f32 = 22.0;
const PROC_HEADER_FONT: f32 = 17.0;
const PROC_ROW_HEIGHT: f32 = 42.0;

const CPU_COLOR_RGB: (u8, u8, u8) = (0xff, 0x9a, 0x3c);
const MEM_COLOR_RGB: (u8, u8, u8) = (0x55, 0xc6, 0xff);
const RX_COLOR_RGB: (u8, u8, u8) = (0x6e, 0xe0, 0x9c);
const TX_COLOR_RGB: (u8, u8, u8) = (0xff, 0x6c, 0x91);

/// Hit-test result for a click inside the sysmon view.
#[derive(Debug, Clone, Copy)]
pub enum SysMonHit {
    /// Click on a row (or the kill button of an *unselected* row) —
    /// the row should be highlighted.
    SelectProcess(i32),
    /// Click on the kill button of the **already-selected** row —
    /// SIGTERM has been requested.
    KillProcess(i32),
}

pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sysmon: &SysMon,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pad = SECTION_PAD * scale;
    let gap = SECTION_GAP * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let mut y = top_y + pad;
    let graph_h = GRAPH_HEIGHT * scale;

    // CPU section
    draw_metric_block(
        painter,
        text,
        Rect::new(inner_x, y, inner_w, graph_h),
        "CPU",
        &format!("{:.1}%", sysmon.last_cpu_pct),
        sysmon.cpu_history.samples(),
        100.0,
        Color::from_rgb8(CPU_COLOR_RGB.0, CPU_COLOR_RGB.1, CPU_COLOR_RGB.2),
        scale,
        alpha,
        surface_w,
        surface_h,
    );
    y += graph_h + gap;

    // Memory section — use a bar + label rather than a sparkline so the
    // used/total numeric is the focal point. Swap is shown if non-zero.
    draw_memory_block(
        painter,
        text,
        Rect::new(inner_x, y, inner_w, graph_h),
        sysmon,
        scale,
        alpha,
        surface_w,
        surface_h,
    );
    y += graph_h + gap;

    // Network section: dual-line RX/TX over the same axis.
    draw_network_block(
        painter,
        text,
        Rect::new(inner_x, y, inner_w, graph_h),
        sysmon,
        scale,
        alpha,
        surface_w,
        surface_h,
    );
    y += graph_h + gap;

    // Process list header + rows.
    y = draw_process_list(
        painter,
        text,
        sysmon,
        Rect::new(inner_x, y, inner_w, panel.y + panel.h - y - pad),
        scale,
        alpha,
        surface_w,
        surface_h,
    );

    y
}

/// Hit-test a click inside the sysmon view. Returns either a
/// `SelectProcess` (click on a row, or on the kill button of a row that
/// isn't currently selected) or a `KillProcess` (click on the kill
/// button of the already-selected row — the soft-confirm step).
pub fn hit_test_view(
    sysmon: &SysMon,
    panel: Rect,
    top_y: f32,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> Option<SysMonHit> {
    let pad = SECTION_PAD * scale;
    let gap = SECTION_GAP * scale;
    let graph_h = GRAPH_HEIGHT * scale;
    let mut y = top_y + pad + (graph_h + gap) * 3.0;
    let row_h = PROC_ROW_HEIGHT * scale;
    let header_h = (PROC_HEADER_FONT + 8.0) * scale;
    y += header_h;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let kill_w = (PROC_ROW_HEIGHT - 12.0) * scale;
    let kill_x = inner_x + inner_w - kill_w - 8.0 * scale;
    for (i, p) in sysmon.processes.iter().enumerate() {
        let row_y = y + row_h * i as f32;
        if phys_y < row_y || phys_y > row_y + row_h {
            continue;
        }
        if phys_x < inner_x || phys_x > inner_x + inner_w {
            continue;
        }
        let kill_y = row_y + (row_h - kill_w) / 2.0;
        let on_kill = phys_x >= kill_x
            && phys_x <= kill_x + kill_w
            && phys_y >= kill_y
            && phys_y <= kill_y + kill_w;
        if on_kill && sysmon.selected_pid == Some(p.pid) {
            return Some(SysMonHit::KillProcess(p.pid));
        }
        return Some(SysMonHit::SelectProcess(p.pid));
    }
    None
}

/// Send `SIGTERM` to a pid. Logs at info on success, warn on failure.
/// Returns `true` if the kernel accepted the signal.
pub fn kill_process(pid: i32) -> bool {
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc == 0 {
        tracing::info!(%pid, "sysmon: sent SIGTERM");
        true
    } else {
        let err = std::io::Error::last_os_error();
        tracing::warn!(%pid, %err, "sysmon: kill failed");
        false
    }
}

// ── Section drawers ─────────────────────────────────────────────────────────

fn draw_metric_block(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    label: &str,
    value: &str,
    history: &[f32],
    scale_max: f32,
    color: Color,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    panel_card(painter, rect, scale, alpha);
    let pad_in = 12.0 * scale;
    let label_font = HEADER_FONT * scale;
    let value_font = VALUE_FONT * scale;

    // Label (top-left), big value (top-right).
    let label_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.7);
    let value_color = color.with_alpha(alpha);
    let label_w = text.measure_width(label, label_font);
    text.queue(
        label,
        label_font,
        rect.x + pad_in,
        rect.y + pad_in * 0.5,
        label_color,
        label_w,
        surface_w,
        surface_h,
    );
    let val_w = text.measure_width(value, value_font);
    text.queue(
        value,
        value_font,
        rect.x + rect.w - pad_in - val_w,
        rect.y + pad_in * 0.5,
        value_color,
        val_w,
        surface_w,
        surface_h,
    );

    // Sparkline below the headline, occupying the lower ~60% of the card.
    let spark_top = rect.y + label_font + pad_in * 0.8;
    let spark_rect = Rect::new(
        rect.x + pad_in,
        spark_top,
        rect.w - pad_in * 2.0,
        rect.y + rect.h - spark_top - pad_in * 0.6,
    );
    draw_sparkline(painter, spark_rect, history, scale_max, color.with_alpha(alpha), scale);
}

fn draw_memory_block(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    sysmon: &SysMon,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    panel_card(painter, rect, scale, alpha);
    let pad_in = 12.0 * scale;
    let label_font = HEADER_FONT * scale;
    let value_font = VALUE_FONT * scale;
    let row_font = ROW_FONT * scale;
    let label_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.7);
    let mem_color = Color::from_rgb8(MEM_COLOR_RGB.0, MEM_COLOR_RGB.1, MEM_COLOR_RGB.2);

    let used = sysmon.mem.used_kb();
    let total = sysmon.mem.mem_total_kb;
    let value = format!("{} / {}", format_kb(used), format_kb(total));

    let ram_w = text.measure_width("RAM", label_font);
    text.queue(
        "RAM",
        label_font,
        rect.x + pad_in,
        rect.y + pad_in * 0.5,
        label_color,
        ram_w,
        surface_w,
        surface_h,
    );
    let val_w = text.measure_width(&value, value_font);
    text.queue(
        &value,
        value_font,
        rect.x + rect.w - pad_in - val_w,
        rect.y + pad_in * 0.5,
        mem_color.with_alpha(alpha),
        val_w,
        surface_w,
        surface_h,
    );

    // Big bar
    let bar_top = rect.y + label_font + pad_in;
    let bar_h = (rect.y + rect.h - bar_top - pad_in - row_font - 4.0 * scale).max(8.0 * scale);
    let bar_rect = Rect::new(rect.x + pad_in, bar_top, rect.w - pad_in * 2.0, bar_h);
    let track = Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha);
    painter.rect_filled(bar_rect, bar_h * 0.45, track);
    let frac = sysmon.mem.used_fraction();
    if frac > 0.0 {
        let fill_w = (bar_rect.w * frac).max(bar_h);
        let fill_rect = Rect::new(bar_rect.x, bar_rect.y, fill_w, bar_rect.h);
        painter.rect_filled(fill_rect, bar_h * 0.45, mem_color.with_alpha(alpha * 0.9));
    }

    // Footer row: swap, if any.
    let swap_total = sysmon.mem.swap_total_kb;
    let swap_used = sysmon.mem.swap_used_kb();
    let swap_text = if swap_total == 0 {
        "Swap not configured".to_string()
    } else {
        format!("Swap {} / {}", format_kb(swap_used), format_kb(swap_total))
    };
    let footer_y = bar_rect.y + bar_rect.h + 4.0 * scale;
    let swap_w = text.measure_width(&swap_text, row_font);
    text.queue(
        &swap_text,
        row_font,
        rect.x + pad_in,
        footer_y,
        label_color,
        swap_w,
        surface_w,
        surface_h,
    );
}

fn draw_network_block(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    sysmon: &SysMon,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    panel_card(painter, rect, scale, alpha);
    let pad_in = 12.0 * scale;
    let label_font = HEADER_FONT * scale;
    let value_font = VALUE_FONT * scale;
    let label_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.7);
    let rx_color = Color::from_rgb8(RX_COLOR_RGB.0, RX_COLOR_RGB.1, RX_COLOR_RGB.2);
    let tx_color = Color::from_rgb8(TX_COLOR_RGB.0, TX_COLOR_RGB.1, TX_COLOR_RGB.2);

    let net_w = text.measure_width("Network", label_font);
    text.queue(
        "Network",
        label_font,
        rect.x + pad_in,
        rect.y + pad_in * 0.5,
        label_color,
        net_w,
        surface_w,
        surface_h,
    );

    // Twin readouts top-right.
    let rx_str = format!("↓ {}", format_rate(sysmon.last_net_rx_bps));
    let tx_str = format!("↑ {}", format_rate(sysmon.last_net_tx_bps));
    let small_font = ROW_FONT * scale;
    let rx_w = text.measure_width(&rx_str, small_font);
    let tx_w = text.measure_width(&tx_str, small_font);
    let stack_x = rect.x + rect.w - pad_in - rx_w.max(tx_w);
    text.queue(
        &rx_str,
        small_font,
        stack_x,
        rect.y + pad_in * 0.5,
        rx_color.with_alpha(alpha),
        rx_w,
        surface_w,
        surface_h,
    );
    text.queue(
        &tx_str,
        small_font,
        stack_x,
        rect.y + pad_in * 0.5 + small_font + 2.0 * scale,
        tx_color.with_alpha(alpha),
        tx_w,
        surface_w,
        surface_h,
    );
    let _ = value_font;

    // Shared y-axis sparkline area. Auto-scale to max(RX, TX, floor)
    // so a quiet network doesn't look like a flat line at the top.
    let spark_top = rect.y + (small_font + 2.0 * scale) * 2.0 + pad_in * 0.5;
    let spark_rect = Rect::new(
        rect.x + pad_in,
        spark_top,
        rect.w - pad_in * 2.0,
        rect.y + rect.h - spark_top - pad_in * 0.6,
    );
    let scale_floor = 64.0 * 1024.0; // 64 KB/s baseline
    let max_rx = sysmon.net_rx_history.max_with_floor(scale_floor);
    let max_tx = sysmon.net_tx_history.max_with_floor(scale_floor);
    let scale_max = max_rx.max(max_tx);
    draw_sparkline(
        painter,
        spark_rect,
        sysmon.net_rx_history.samples(),
        scale_max,
        rx_color.with_alpha(alpha),
        scale,
    );
    draw_sparkline_line_only(
        painter,
        spark_rect,
        sysmon.net_tx_history.samples(),
        scale_max,
        tx_color.with_alpha(alpha),
        scale,
    );
}

// ── Process list ────────────────────────────────────────────────────────────

fn draw_process_list(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sysmon: &SysMon,
    rect: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let header_font = PROC_HEADER_FONT * scale;
    let row_font = ROW_FONT * scale;
    let row_h = PROC_ROW_HEIGHT * scale;
    let header_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.55);
    let row_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.95);

    let cpu_col_w = 90.0 * scale;
    let mem_col_w = 110.0 * scale;
    let kill_w = (PROC_ROW_HEIGHT - 12.0) * scale;
    let pad_r = 8.0 * scale;
    let kill_x = rect.x + rect.w - kill_w - pad_r;
    let mem_x = kill_x - 16.0 * scale - mem_col_w;
    let cpu_x = mem_x - 16.0 * scale - cpu_col_w;

    // Header row.
    let header_y = rect.y;
    queue_left(text, "PROCESS", header_font, rect.x + 8.0 * scale, header_y, header_color, surface_w, surface_h);
    queue_right(text, "CPU", header_font, cpu_x + cpu_col_w, header_y, header_color, surface_w, surface_h);
    queue_right(text, "MEM", header_font, mem_x + mem_col_w, header_y, header_color, surface_w, surface_h);

    let rows_top = header_y + header_font + 8.0 * scale;
    for (i, p) in sysmon.processes.iter().enumerate() {
        let y = rows_top + row_h * i as f32;
        if y + row_h > rect.y + rect.h {
            break;
        }
        let selected = sysmon.selected_pid == Some(p.pid);
        // Background: strong tint when selected, subtle zebra otherwise.
        if selected {
            let sel_bg = Color::rgba(0.40, 0.65, 1.00, 0.18 * alpha);
            painter.rect_filled(Rect::new(rect.x, y, rect.w, row_h), 6.0 * scale, sel_bg);
            // Accent bar on the left edge so the highlight reads even
            // at low alpha.
            let accent = Color::rgba(0.40, 0.75, 1.00, 0.95 * alpha);
            painter.rect_filled(
                Rect::new(rect.x, y + 4.0 * scale, 3.0 * scale, row_h - 8.0 * scale),
                1.5 * scale,
                accent,
            );
        } else if i % 2 == 0 {
            let band = Color::rgba(1.0, 1.0, 1.0, 0.04 * alpha);
            painter.rect_filled(Rect::new(rect.x, y, rect.w, row_h), 4.0 * scale, band);
        }
        draw_process_row(
            painter, text, p, row_font, row_color, alpha, scale, rect.x + 8.0 * scale, y, row_h,
            cpu_x, cpu_col_w, mem_x, mem_col_w, kill_x, kill_w, selected, surface_w, surface_h,
        );
    }

    rows_top + row_h * sysmon.processes.len() as f32
}

#[allow(clippy::too_many_arguments)]
fn draw_process_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    p: &ProcessRow,
    font: f32,
    row_color: Color,
    alpha: f32,
    scale: f32,
    x: f32,
    y: f32,
    h: f32,
    cpu_x: f32,
    cpu_w: f32,
    mem_x: f32,
    mem_w: f32,
    kill_x: f32,
    kill_w: f32,
    selected: bool,
    surface_w: u32,
    surface_h: u32,
) {
    let baseline = y + (h - font) / 2.0;
    let max_name_w = cpu_x - x - 8.0 * scale;
    let name = truncate_to_width(text, &p.comm, font, max_name_w);
    queue_left(text, &name, font, x, baseline, row_color, surface_w, surface_h);

    let cpu_str = format!("{:.1}%", p.cpu_load * 100.0);
    queue_right(text, &cpu_str, font, cpu_x + cpu_w, baseline, row_color, surface_w, surface_h);

    let mem_str = crate::controls::sysmon::proc::format_bytes(p.rss_bytes);
    queue_right(text, &mem_str, font, mem_x + mem_w, baseline, row_color, surface_w, surface_h);

    // Kill button. "Armed" when this row is selected — brighter red +
    // a white ring outside the body to signal that the next click will
    // actually send SIGTERM. Unarmed = muted slate (visible but clearly
    // inert).
    let btn_y = y + (h - kill_w) / 2.0;
    let btn_rect = Rect::new(kill_x, btn_y, kill_w, kill_w);
    let body = if selected {
        Color::rgba(1.0, 0.30, 0.36, 0.95 * alpha)
    } else {
        Color::rgba(0.55, 0.58, 0.65, 0.55 * alpha)
    };
    painter.rect_filled(btn_rect, kill_w * 0.25, body);
    if selected {
        let ring = Color::rgba(1.0, 1.0, 1.0, 0.65 * alpha);
        painter.rect_stroke(btn_rect, kill_w * 0.25, 1.5 * scale, ring);
    }
    let icon_pad = kill_w * 0.30;
    let line_w = (kill_w * 0.14).max(1.8 * scale);
    let white = Color::rgba(1.0, 1.0, 1.0, alpha);
    painter.line(
        kill_x + icon_pad,
        btn_y + icon_pad,
        kill_x + kill_w - icon_pad,
        btn_y + kill_w - icon_pad,
        line_w,
        white,
    );
    painter.line(
        kill_x + kill_w - icon_pad,
        btn_y + icon_pad,
        kill_x + icon_pad,
        btn_y + kill_w - icon_pad,
        line_w,
        white,
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn panel_card(painter: &mut Painter, rect: Rect, scale: f32, alpha: f32) {
    let bg = Color::rgba(1.0, 1.0, 1.0, 0.05 * alpha);
    painter.rect_filled(rect, 8.0 * scale, bg);
}

fn draw_sparkline(
    painter: &mut Painter,
    rect: Rect,
    samples: &[f32],
    scale_max: f32,
    color: Color,
    scale: f32,
) {
    if samples.len() < 2 {
        return;
    }
    let pts = sparkline_points(rect, samples, scale_max, scale);
    let mut area = pts.clone();
    area.push((rect.x + rect.w, rect.y + rect.h - 2.0 * scale));
    area.push((rect.x, rect.y + rect.h - 2.0 * scale));
    let fill = Color::rgba(color.r, color.g, color.b, color.a * 0.22);
    painter.polygon(&area, fill);
    painter.polyline_round(&pts, 1.8 * scale, color);
}

fn draw_sparkline_line_only(
    painter: &mut Painter,
    rect: Rect,
    samples: &[f32],
    scale_max: f32,
    color: Color,
    scale: f32,
) {
    if samples.len() < 2 {
        return;
    }
    let pts = sparkline_points(rect, samples, scale_max, scale);
    painter.polyline_round(&pts, 1.8 * scale, color);
}

fn sparkline_points(rect: Rect, samples: &[f32], scale_max: f32, scale: f32) -> Vec<(f32, f32)> {
    let n = samples.len();
    let denom = scale_max.max(0.001);
    let pad_y = 2.0 * scale;
    let usable_h = (rect.h - pad_y * 2.0).max(1.0);
    let step = if n > 1 { rect.w / (n - 1) as f32 } else { 0.0 };
    samples
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let frac = (v / denom).clamp(0.0, 1.0);
            (
                rect.x + step * i as f32,
                rect.y + pad_y + (1.0 - frac) * usable_h,
            )
        })
        .collect()
}

fn queue_left(
    text: &mut TextRenderer,
    s: &str,
    font: f32,
    x: f32,
    y: f32,
    color: Color,
    surface_w: u32,
    surface_h: u32,
) {
    let w = text.measure_width(s, font);
    text.queue(s, font, x, y, color, w, surface_w, surface_h);
}

fn queue_right(
    text: &mut TextRenderer,
    s: &str,
    font: f32,
    right_x: f32,
    y: f32,
    color: Color,
    surface_w: u32,
    surface_h: u32,
) {
    let w = text.measure_width(s, font);
    text.queue(s, font, right_x - w, y, color, w, surface_w, surface_h);
}

fn truncate_to_width(text: &mut TextRenderer, s: &str, font: f32, max_w: f32) -> String {
    if text.measure_width(s, font) <= max_w {
        return s.to_string();
    }
    let ellipsis = "…";
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + ellipsis;
        if text.measure_width(&candidate, font) <= max_w {
            return candidate;
        }
    }
    ellipsis.to_string()
}

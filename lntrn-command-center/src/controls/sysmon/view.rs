//! Expanded "system monitor" view: three live graphs (CPU history, RAM
//! used, network RX/TX) and a process list with per-row kill buttons.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::sysmon::proc::{format_kb, format_rate};
use crate::controls::sysmon::process_list::{self, ProcessHit};
use crate::controls::sysmon::SysMon;

const SECTION_PAD: f32 = 24.0;
const SECTION_GAP: f32 = 14.0;
const GRAPH_HEIGHT: f32 = 80.0;

const HEADER_FONT: f32 = 18.0;
const VALUE_FONT: f32 = 22.0;
pub(super) const ROW_FONT: f32 = 22.0;

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
    /// Click on the CPU column header — toggle CPU sort direction.
    SortByCpu,
    /// Click on the MEM column header — toggle MEM sort direction.
    SortByMem,
    /// Click on the × inside the filter strip — clear filter text.
    ClearFilter,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sysmon: &SysMon,
    panel: Rect,
    top_y: f32,
    scale: f32,
    alpha: f32,
    text_size: f32,
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

    // RAM section — same sparkline shape as CPU + Network for visual
    // consistency. Value shows "used / total" so the numeric headline
    // is still informative; swap is omitted (not configured on this box).
    let used = sysmon.mem.used_kb();
    let total = sysmon.mem.mem_total_kb;
    let ram_value = format!("{} / {}", format_kb(used), format_kb(total));
    draw_metric_block(
        painter,
        text,
        Rect::new(inner_x, y, inner_w, graph_h),
        "RAM",
        &ram_value,
        sysmon.mem_history.samples(),
        100.0,
        Color::from_rgb8(MEM_COLOR_RGB.0, MEM_COLOR_RGB.1, MEM_COLOR_RGB.2),
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

    // Process list section — filter strip + sortable headers + rows.
    let proc_rect = Rect::new(inner_x, y, inner_w, panel.y + panel.h - y - pad);
    let lay = process_list::draw(
        painter, text, sysmon, proc_rect, scale, alpha, text_size, surface_w, surface_h,
    );
    y = lay.rows_top + lay.row_h * lay.visible_rows as f32;
    y
}

/// Hit-test a click inside the sysmon view. Resolves clicks against
/// the filter strip, the sortable column headers, and the row /
/// kill-button geometry. Returns `None` for clicks that hit nothing
/// interactive.
pub fn hit_test_view(
    sysmon: &SysMon,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    phys_x: f32,
    phys_y: f32,
) -> Option<SysMonHit> {
    let pad = SECTION_PAD * scale;
    let gap = SECTION_GAP * scale;
    let graph_h = GRAPH_HEIGHT * scale;
    let y_proc_top = top_y + pad + (graph_h + gap) * 3.0;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let proc_rect = Rect::new(
        inner_x, y_proc_top, inner_w, panel.y + panel.h - y_proc_top - pad,
    );
    let lay = process_list::layout(sysmon, proc_rect, scale, text_size);
    let hit = process_list::hit_test(sysmon, &lay, phys_x, phys_y)?;
    Some(match hit {
        ProcessHit::Select(pid) => SysMonHit::SelectProcess(pid),
        ProcessHit::Kill(pid) => SysMonHit::KillProcess(pid),
        ProcessHit::SortByCpu => SysMonHit::SortByCpu,
        ProcessHit::SortByMem => SysMonHit::SortByMem,
        ProcessHit::ClearFilter => SysMonHit::ClearFilter,
    })
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

    // Label (top-left), big value (top-right). Pad max_width past the
    // measured width so the text engine doesn't wrap the trailing
    // glyph when measurement sits right on the wrap boundary.
    let label_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.7);
    let value_color = color.with_alpha(alpha);
    let label_w = text.measure_width(label, label_font);
    text.queue(
        label,
        label_font,
        rect.x + pad_in,
        rect.y + pad_in * 0.5,
        label_color,
        label_w + 8.0 * scale,
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
        val_w + 8.0 * scale,
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

pub(super) fn queue_left(
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

pub(super) fn queue_right(
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


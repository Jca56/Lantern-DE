//! System monitor drawing — panel + pinned bar pills.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::scroll::{ScrollArea, Scrollbar};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use lntrn_ui::gpu::TextInput;

use super::sysmon::{
    SystemMonitor, SortColumn,
    format_bytes, format_bytes_kb, format_rate,
    ZONE_CORES_TOGGLE, ZONE_PROC_BASE, ZONE_PINNED_BASE, ZONE_KILL_BASE,
    ZONE_SORT_NAME, ZONE_SORT_CPU, ZONE_SORT_MEM, ZONE_SORT_PID,
    SPARKLINE_LEN,
};

const SECTION_FONT: f32 = 24.0;
const VALUE_FONT: f32 = 22.0;
const CORE_FONT: f32 = 20.0;
const PROC_FONT: f32 = 20.0;
const BAR_H: f32 = 18.0;
const CORE_BAR_H: f32 = 14.0;
const SECTION_GAP: f32 = 22.0;
const LINE_GAP: f32 = 8.0;
const BAR_RADIUS: f32 = 7.0;
const PROC_LINE_H: f32 = 32.0;
const GAUGE_R: f32 = 75.0;
const GAUGE_THICK: f32 = 12.0;

impl SystemMonitor {
    pub(crate) fn content_height(&self, scale: f32) -> f32 {
        let s = scale;
        let sg = SECTION_GAP * s; let lg = LINE_GAP * s;
        let sf = SECTION_FONT * s; let vf = VALUE_FONT * s; let cf = CORE_FONT * s;
        let bh = BAR_H * s; let cbh = CORE_BAR_H * s;
        let mut h = 16.0 * s;
        h += (GAUGE_R * 2.0) * s + 10.0 * s + sf + cf + 24.0 * s;
        h += sf + 16.0 * s;
        if self.cores_expanded {
            h += ((self.per_core.len() + 1) / 2) as f32 * (cf + cbh + lg + 2.0 * s);
        }
        h += sg;
        for _ in &self.disks { h += vf + lg + bh + lg; }
        h += sg;
        h += sf + lg + cbh + sg;
        h += sf + lg;
        h += 44.0 * s + lg; // filter input
        h += PROC_LINE_H * s; // column headers
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
        scroll.begin(painter, text);

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

        // ── Dual-ring gauges ──
        let gauge_r = GAUGE_R * scale;
        let thick = GAUGE_THICK * scale;
        let gauge_cy = y + gauge_r + 4.0 * scale;
        let quarter_w = w * 0.25;

        let temp_color = match self.cpu_temp {
            0..=59 => Color::from_rgb8(34, 197, 94),
            60..=79 => Color::from_rgb8(234, 179, 8),
            _ => Color::from_rgb8(239, 68, 68),
        };
        let temp_pct = (self.cpu_temp as f32 / 100.0).clamp(0.0, 1.0);
        let temp_sub = if self.cpu_temp > 0 { format!("{}C", self.cpu_temp) } else { String::new() };

        let cpu_cx = x + quarter_w;
        draw_dual_gauge(painter, text, cpu_cx, gauge_cy, gauge_r, thick,
            self.cpu_pct / 100.0, accent, temp_pct, temp_color,
            track, &format!("{:.0}%", self.cpu_pct), accent,
            "CPU", &temp_sub, scale, clip);

        let mem_used = self.mem_total_kb.saturating_sub(self.mem_avail_kb);
        let mem_pct = if self.mem_total_kb > 0 { mem_used as f32 / self.mem_total_kb as f32 } else { 0.0 };
        let swap_pct = if self.swap_total_kb > 0 { self.swap_used_kb as f32 / self.swap_total_kb as f32 } else { 0.0 };
        let ram_cx = x + w - quarter_w;
        let ram_detail = format!("{} / {}", format_bytes_kb(mem_used), format_bytes_kb(self.mem_total_kb));

        draw_dual_gauge(painter, text, ram_cx, gauge_cy, gauge_r, thick,
            mem_pct, ram_c, swap_pct, swap_c,
            track, &format!("{:.0}%", mem_pct * 100.0), ram_c,
            "RAM", "Swap", scale, clip);

        let label_bottom = gauge_cy + gauge_r + 10.0 * scale + sf;
        let rw = text.measure_width(&ram_detail, cf);
        text.queue_clipped(&ram_detail, cf, ram_cx - rw * 0.5, label_bottom + 4.0 * scale, palette.text_secondary, w, clip);

        y += gauge_r * 2.0 + 10.0 * scale + sf + cf + 24.0 * scale;

        // ── Cores toggle ──
        let arrow = if self.cores_expanded { "v" } else { ">" };
        let cores_label = format!("{arrow}  Threads ({})", self.per_core.len());
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
                    let freq_text = if idx < self.cpu_freqs.len() && self.cpu_freqs[idx] > 0 {
                        format!("Core {} {:.0}%  {:.1} GHz", idx, self.per_core[idx], self.cpu_freqs[idx] as f32 / 1_000_000.0)
                    } else {
                        format!("Core {} {:.0}%", idx, self.per_core[idx])
                    };
                    text.queue_clipped(&freq_text, cf, cx, y, palette.text_secondary, cw, clip);
                    draw_bar(painter, cx, y + cf + 2.0 * scale, cw, cbh, br, self.per_core[idx] / 100.0, accent.with_alpha(0.7), track);
                }
                y += cf + cbh + lg + 2.0 * scale;
            }
        }
        y += sg;

        // ── Disks ──
        for disk in &self.disks {
            let pct = if disk.total > 0 { disk.used as f32 / disk.total as f32 } else { 0.0 };
            text.queue_clipped(&format!("{}  {} / {}", disk.name, format_bytes(disk.used), format_bytes(disk.total)),
                vf, x, y, palette.text, w, clip);
            y += vf + lg;
            draw_bar(painter, x, y, w, bh, br, pct, disk_c, track);
            y += bh + lg;
        }
        y += sg;

        // ── Network ──
        text.queue_clipped(&format!("Net  v {}  ^ {}", format_rate(self.rx_rate), format_rate(self.tx_rate)),
            sf, x, y, palette.text, w, clip);
        y += sf + lg;
        let max_rate = 100.0 * 1024.0 * 1024.0;
        let hw = (w - pad) * 0.5;
        draw_bar(painter, x, y, hw, cbh, br, (self.rx_rate as f32 / max_rate).min(1.0), net_c, track);
        draw_bar(painter, x + hw + pad, y, hw, cbh, br, (self.tx_rate as f32 / max_rate).min(1.0), net_c.with_alpha(0.7), track);
        y += cbh + sg;

        // ── Processes ──
        text.queue_clipped("Processes", sf, x, y, palette.text, w, clip);
        y += sf + lg;

        // Filter input
        let input_h = 44.0 * scale;
        let input_rect = Rect::new(x, y, w, input_h);
        TextInput::new(input_rect)
            .text(&self.proc_filter)
            .placeholder("Filter processes...")
            .focused(self.filter_focused)
            .scale(scale)
            .cursor_pos(self.filter_cursor)
            .draw(painter, text, palette, screen_w, screen_h);
        y += input_h + lg;

        // Column headers (clickable for sort)
        let name_w = w * 0.45; let cpu_x = x + name_w; let cpu_w = w * 0.2;
        let mem_x = cpu_x + cpu_w; let mem_w = w * 0.2;
        let pid_x = mem_x + mem_w; let pid_w = w - name_w - cpu_w - mem_w;

        let arrow_up = " ^"; let arrow_dn = " v";
        let cols: [(u32, f32, f32, &str, SortColumn); 4] = [
            (ZONE_SORT_NAME, x, name_w, "Name", SortColumn::Name),
            (ZONE_SORT_CPU, cpu_x, cpu_w, "CPU%", SortColumn::Cpu),
            (ZONE_SORT_MEM, mem_x, mem_w, "Mem", SortColumn::Mem),
            (ZONE_SORT_PID, pid_x, pid_w, "PID", SortColumn::Pid),
        ];
        for (zone_id, col_x, col_w, label, col) in cols {
            let header_rect = Rect::new(col_x, y, col_w, plh);
            let hs = ix.add_zone(zone_id, header_rect);
            let active = self.sort_column == col;
            let color = if active { palette.accent } else if hs.is_hovered() { palette.text } else { palette.text_secondary };
            let arrow = if active { if self.sort_ascending { arrow_up } else { arrow_dn } } else { "" };
            text.queue_clipped(&format!("{label}{arrow}"), pf, col_x, y, color, col_w, clip);
        }
        y += plh;

        // Filter procs for display
        let filter_lower = self.proc_filter.to_lowercase();
        let filtered: Vec<(usize, &_)> = self.procs.iter().enumerate()
            .filter(|(_, p)| filter_lower.is_empty() || p.name.to_lowercase().contains(&filter_lower))
            .collect();

        for (i, p) in filtered {
            let row_rect = Rect::new(x - 4.0 * scale, y, w + 8.0 * scale, plh);
            let zone_id = ZONE_PROC_BASE + i as u32;
            let row_state = ix.add_zone(zone_id, row_rect);
            let hovered = row_state.is_hovered();
            if hovered {
                painter.rect_filled(row_rect, 6.0 * scale, palette.muted.with_alpha(0.15));
            }
            let is_pinned = self.pinned.iter().any(|pin| pin.name == p.name);
            if is_pinned {
                painter.rect_filled(row_rect, 6.0 * scale, palette.accent.with_alpha(0.08));
            }

            let cc = if p.cpu_pct > 50.0 { proc_c } else if p.cpu_pct > 10.0 { palette.warning } else { palette.text_secondary };
            let nm = if p.name.len() > 22 { format!("{}...", &p.name[..20]) } else { p.name.clone() };
            text.queue_clipped(&nm, pf, x, y, palette.text, name_w, clip);
            text.queue_clipped(&format!("{:.1}", p.cpu_pct), pf, cpu_x, y, cc, cpu_w, clip);
            text.queue_clipped(&format_bytes_kb(p.mem_kb), pf, mem_x, y, palette.text_secondary, mem_w, clip);
            text.queue_clipped(&p.pid.to_string(), pf, pid_x, y, palette.muted, pid_w, clip);

            // Kill button on hover
            if hovered {
                let kill_size = pf;
                let kill_x = x + w - kill_size;
                let kill_rect = Rect::new(kill_x - 4.0 * scale, y + 2.0 * scale, kill_size + 8.0 * scale, plh - 4.0 * scale);
                let kill_state = ix.add_zone(ZONE_KILL_BASE + i as u32, kill_rect);
                if kill_state.is_hovered() {
                    painter.rect_filled(kill_rect, 4.0 * scale, proc_c.with_alpha(0.2));
                }
                text.queue_clipped("x", pf, kill_x, y, proc_c, kill_size, clip);
            }

            y += plh;
        }
        let _ = y;

        scroll.end(painter, text);
        if scroll.is_scrollable() {
            let sb = Scrollbar::new(&gr, content_h, self.scroll_offset);
            sb.draw(painter, InteractionState::Idle, palette);
        }
    }

    /// Draw pinned process monitors on the bar. Returns total width consumed.
    pub fn draw_pinned(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        x: f32, bar_y: f32, bar_h: f32, scale: f32,
        screen_w: u32, screen_h: u32,
    ) -> f32 {
        if self.pinned.is_empty() { return 0.0; }

        let pad = 8.0 * scale;
        let font = 18.0 * scale;
        let small = 18.0 * scale;
        let stats_gap = 10.0 * scale;
        let bar_thick = 12.0 * scale;
        let item_gap = 12.0 * scale;
        let mut cx = x;

        let accent = palette.accent;
        let ram_c = Color::from_rgb8(59, 130, 246);
        let track = palette.muted.with_alpha(0.15);

        for (i, pinned) in self.pinned.iter().enumerate() {
            let label = &pinned.name;
            let cpu_text = format!("{:.1}%", pinned.cpu_pct);
            let mem_text = format_bytes_kb(pinned.mem_kb);

            let name_w = text.measure_width(label, font);
            let cpu_w = text.measure_width(&cpu_text, small);
            let mem_w = text.measure_width(&mem_text, small);
            // Width = max of name row vs stats row
            let stats_row_w = cpu_w + stats_gap + mem_w;
            let content_w = name_w.max(stats_row_w);
            let item_w = content_w + pad * 2.0;

            let pill = Rect::new(cx, bar_y + 2.0 * scale, item_w, bar_h - 4.0 * scale);
            let zone_id = ZONE_PINNED_BASE + i as u32;
            let state = ix.add_zone(zone_id, pill);
            let bg_alpha = if state.is_hovered() { 0.15 } else { 0.08 };
            painter.rect_filled(pill, 8.0 * scale, palette.surface_2.with_alpha(bg_alpha));

            // Warning pulse overlay when CPU > 80%
            if pinned.warning_phase > 0.01 {
                let pulse = (pinned.warning_phase * std::f32::consts::PI * 2.0).sin().abs();
                let warn_alpha = 0.06 + 0.12 * pulse;
                painter.rect_filled(pill, 8.0 * scale, Color::from_rgb8(239, 68, 68).with_alpha(warn_alpha));
            }

            let cpu_color = if pinned.cpu_pct > 50.0 {
                Color::from_rgb8(239, 68, 68)
            } else if pinned.cpu_pct > 10.0 {
                palette.warning
            } else {
                accent
            };

            // Row 1: process name (top, centered)
            let name_x = cx + pad + (content_w - name_w) * 0.5;
            text.queue(label, font, name_x, bar_y + 4.0 * scale, palette.text, name_w + 4.0, screen_w, screen_h);

            // Row 2: CPU% and RAM side by side (centered)
            let stats_x = cx + pad + (content_w - stats_row_w) * 0.5;
            let row2_y = bar_y + 4.0 * scale + font + 6.0 * scale;
            text.queue(&cpu_text, small, stats_x, row2_y, cpu_color, cpu_w + 4.0, screen_w, screen_h);
            text.queue(&mem_text, small, stats_x + cpu_w + stats_gap, row2_y, ram_c, mem_w + 4.0, screen_w, screen_h);

            // Sparkline at bottom of pill
            let spark_y = pill.y + pill.h - bar_thick - 2.0 * scale;
            let spark_w = item_w - pad * 2.0;
            let bar_gap = 1.5 * scale;
            let col_w = (spark_w - bar_gap * (SPARKLINE_LEN - 1) as f32) / SPARKLINE_LEN as f32;

            // Track background
            painter.rect_filled(Rect::new(cx + pad, spark_y, spark_w, bar_thick), 2.0 * scale, track);

            // Draw each history column oldest→newest
            for j in 0..SPARKLINE_LEN {
                let sample_idx = (pinned.history_idx + j) % SPARKLINE_LEN;
                let val = pinned.cpu_history[sample_idx];
                if val < 0.1 { continue; }
                let frac = (val / 100.0).clamp(0.0, 1.0);
                let col_h = (bar_thick * frac).max(2.0 * scale);
                let col_x = cx + pad + j as f32 * (col_w + bar_gap);
                let col_y = spark_y + bar_thick - col_h;
                let col_color = if val > 80.0 {
                    Color::from_rgb8(239, 68, 68)
                } else if val > 50.0 {
                    palette.warning
                } else {
                    accent
                };
                painter.rect_filled(Rect::new(col_x, col_y, col_w, col_h), 1.0 * scale, col_color);
            }

            cx += item_w + item_gap;
        }

        cx - x
    }
}

// ── Free drawing helpers ────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_dual_gauge(
    painter: &mut Painter, text: &mut TextRenderer,
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
    let full = pi * 2.0 - 0.001;

    painter.arc(cx, cy, r, start, full, thick, r - thick, track_color);
    if outer_pct > 0.001 {
        painter.arc(cx, cy, r, start, full * outer_pct.clamp(0.0, 1.0), thick, r - thick, outer_color);
    }

    painter.arc(cx, cy, inner_r, start, full, inner_thick, inner_r - inner_thick, track_color);
    if inner_pct > 0.001 {
        painter.arc(cx, cy, inner_r, start, full * inner_pct.clamp(0.0, 1.0), inner_thick, inner_r - inner_thick, inner_color);
    }

    let vf = 24.0 * scale;
    let vw = text.measure_width(center_text, vf);
    text.queue_clipped(center_text, vf, cx - vw * 0.5, cy - vf * 0.8, center_color, vw + 4.0, clip);

    if !sub_label.is_empty() {
        let sf = CORE_FONT * scale;
        let sw = text.measure_width(sub_label, sf);
        text.queue_clipped(sub_label, sf, cx - sw * 0.5, cy + vf * 0.1, inner_color.with_alpha(0.7), sw + 4.0, clip);
    }

    let lf = SECTION_FONT * scale;
    let lw = text.measure_width(label, lf);
    text.queue_clipped(label, lf, cx - lw * 0.5, cy + r + 10.0 * scale, outer_color, lw + 4.0, clip);
}

fn draw_bar(painter: &mut Painter, x: f32, y: f32, w: f32, h: f32, r: f32, pct: f32, fill: Color, track: Color) {
    painter.rect_filled(Rect::new(x, y, w, h), r, track);
    if pct > 0.001 {
        painter.rect_filled(Rect::new(x, y, (w * pct.clamp(0.0, 1.0)).max(h), h), r, fill);
    }
}

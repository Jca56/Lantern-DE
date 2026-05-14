//! Process list section of the expanded sysmon view: filter strip,
//! clickable sortable column headers, and the rows themselves with
//! per-row kill buttons.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::sysmon::{ProcessRow, SysMon};

use super::view::{queue_left, queue_right};

pub const PROC_HEADER_FONT: f32 = 17.0;
pub const PROC_ROW_HEIGHT: f32 = 42.0;
pub const FILTER_STRIP_H: f32 = 34.0;
pub const FILTER_STRIP_GAP: f32 = 8.0;

const ACCENT: Color = Color::rgb(0.97, 0.71, 0.30); // gold

/// Outcomes of clicks inside the process list section. The caller
/// (layershell) drives the corresponding state change.
#[derive(Debug, Clone, Copy)]
pub enum ProcessHit {
    Select(i32),
    Kill(i32),
    SortByCpu,
    SortByMem,
    /// Click on the trailing × inside the filter strip — clear it.
    ClearFilter,
}

/// Layout for one process-list section render. Returned by [`draw`]
/// so the layershell can hit-test using the exact same geometry.
#[derive(Debug, Clone, Copy)]
pub struct ProcessListLayout {
    pub filter_rect: Rect,
    pub clear_btn: Option<Rect>,
    pub cpu_header_rect: Rect,
    pub mem_header_rect: Rect,
    pub rows_top: f32,
    pub row_h: f32,
    pub visible_rows: usize,
    pub kill_x: f32,
    pub kill_w: f32,
    pub inner_x: f32,
    pub inner_w: f32,
}

/// Compute the section's layout without drawing. Used by `draw` and
/// `hit_test_view` so the hit geometry never drifts from what the
/// renderer paints.
pub fn layout(sysmon: &SysMon, rect: Rect, scale: f32, text_size: f32) -> ProcessListLayout {
    let strip_h = FILTER_STRIP_H * scale;
    let strip_gap = FILTER_STRIP_GAP * scale;
    let header_font = PROC_HEADER_FONT * scale;
    // Row height grows with the configured Text Size so larger fonts
    // don't run rows into each other vertically.
    let row_h = (PROC_ROW_HEIGHT.max(text_size + 20.0)) * scale;

    let pad_r = 8.0 * scale;
    let kill_w = (PROC_ROW_HEIGHT - 12.0) * scale;
    // Tight CPU/MEM columns + small inter-column gaps so the process
    // name has as much room as possible to spell itself out.
    let cpu_col_w = 70.0 * scale;
    let mem_col_w = 88.0 * scale;
    let kill_x = rect.x + rect.w - kill_w - pad_r;
    let mem_x = kill_x - 12.0 * scale - mem_col_w;
    let cpu_x = mem_x - 12.0 * scale - cpu_col_w;

    let filter_rect = Rect::new(rect.x, rect.y, rect.w, strip_h);
    let clear_btn = if !sysmon.filter.query().is_empty() {
        let btn_w = strip_h - 8.0 * scale;
        let bx = filter_rect.x + filter_rect.w - btn_w - 6.0 * scale;
        let by = filter_rect.y + (strip_h - btn_w) / 2.0;
        Some(Rect::new(bx, by, btn_w, btn_w))
    } else {
        None
    };

    let header_y = rect.y + strip_h + strip_gap;
    let cpu_header_rect = Rect::new(
        cpu_x, header_y - 2.0 * scale, cpu_col_w, header_font + 4.0 * scale,
    );
    let mem_header_rect = Rect::new(
        mem_x, header_y - 2.0 * scale, mem_col_w, header_font + 4.0 * scale,
    );

    let rows_top = header_y + header_font + 8.0 * scale;
    let rows = filtered_rows(sysmon);
    let avail_h = (rect.y + rect.h - rows_top).max(0.0);
    let max_visible = (avail_h / row_h).floor() as usize;
    let visible = max_visible.min(rows.len());

    ProcessListLayout {
        filter_rect,
        clear_btn,
        cpu_header_rect,
        mem_header_rect,
        rows_top,
        row_h,
        visible_rows: visible,
        kill_x,
        kill_w,
        inner_x: rect.x,
        inner_w: rect.w,
    }
}

/// Return the rows that survive the active filter, in display order.
/// Substring match on the comm (case-insensitive). Empty filter ⇒
/// pass-through.
pub fn filtered_rows<'a>(sysmon: &'a SysMon) -> Vec<&'a ProcessRow> {
    let q = sysmon.filter.query().trim().to_ascii_lowercase();
    if q.is_empty() {
        return sysmon.processes.iter().collect();
    }
    sysmon
        .processes
        .iter()
        .filter(|p| p.comm.to_ascii_lowercase().contains(&q))
        .collect()
}

/// Draw the filter strip + headers + rows. Returns the layout, which
/// the hit-tester reuses so the click geometry never drifts from what
/// was painted.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sysmon: &SysMon,
    rect: Rect,
    scale: f32,
    alpha: f32,
    text_size: f32,
    surface_w: u32,
    surface_h: u32,
) -> ProcessListLayout {
    let lay = layout(sysmon, rect, scale, text_size);
    let header_font = PROC_HEADER_FONT * scale;
    // Row font respects the user's Text Size config — defaults to
    // their settings value, falling back to ROW_FONT if it's missing
    // or absurdly small.
    let row_font = text_size.max(12.0) * scale;
    let header_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.55);
    let row_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.95);
    let cpu_col_w = lay.cpu_header_rect.w;
    let mem_col_w = lay.mem_header_rect.w;
    let cpu_x = lay.cpu_header_rect.x;
    let mem_x = lay.mem_header_rect.x;

    // ── Filter strip ──────────────────────────────────────────────────
    draw_filter_strip(
        painter, text, sysmon, lay.filter_rect, scale, alpha, surface_w, surface_h,
    );

    // ── Headers ───────────────────────────────────────────────────────
    let header_y = lay.cpu_header_rect.y + 2.0 * scale;
    queue_left(
        text, "PROCESS", header_font, rect.x + 8.0 * scale, header_y, header_color,
        surface_w, surface_h,
    );
    let arrow = if sysmon.sort.is_desc() { "▼" } else { "▲" };
    let cpu_label = if sysmon.sort.is_cpu() {
        format!("CPU {}", arrow)
    } else {
        "CPU".to_string()
    };
    let mem_label = if sysmon.sort.is_mem() {
        format!("MEM {}", arrow)
    } else {
        "MEM".to_string()
    };
    let cpu_color = if sysmon.sort.is_cpu() { ACCENT.with_alpha(alpha) } else { header_color };
    let mem_color = if sysmon.sort.is_mem() { ACCENT.with_alpha(alpha) } else { header_color };
    queue_right(
        text, &cpu_label, header_font, cpu_x + cpu_col_w, header_y, cpu_color,
        surface_w, surface_h,
    );
    queue_right(
        text, &mem_label, header_font, mem_x + mem_col_w, header_y, mem_color,
        surface_w, surface_h,
    );

    // ── Rows ──────────────────────────────────────────────────────────
    let rows = filtered_rows(sysmon);
    let kill_x = lay.kill_x;
    let kill_w = lay.kill_w;
    let visible = lay.visible_rows;
    let rows_top = lay.rows_top;
    let row_h = lay.row_h;

    for (i, p) in rows.iter().take(visible).enumerate() {
        let y = rows_top + row_h * i as f32;
        let selected = sysmon.selected_pid == Some(p.pid);
        if selected {
            let sel_bg = Color::rgba(0.40, 0.65, 1.00, 0.18 * alpha);
            painter.rect_filled(Rect::new(rect.x, y, rect.w, row_h), 6.0 * scale, sel_bg);
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
        draw_row(
            painter, text, p, row_font, row_color, alpha, scale,
            rect.x + 8.0 * scale, y, row_h,
            cpu_x, cpu_col_w, mem_x, mem_col_w, kill_x, kill_w, selected,
            surface_w, surface_h,
        );
    }

    // When the filter zeroes out the result set, show a tiny hint so
    // it doesn't look like the panel broke.
    if rows.is_empty() && !sysmon.filter.query().is_empty() {
        let hint = format!("No processes match \"{}\"", sysmon.filter.query());
        let muted = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.45);
        let w = text.measure_width(&hint, row_font);
        text.queue(
            &hint, row_font,
            rect.x + (rect.w - w) / 2.0,
            rows_top + 16.0 * scale,
            muted, w, surface_w, surface_h,
        );
    }

    lay
}

/// Resolve a click against the laid-out section. Returns `None` if the
/// click missed everything interactive.
pub fn hit_test(
    sysmon: &SysMon,
    layout: &ProcessListLayout,
    phys_x: f32,
    phys_y: f32,
) -> Option<ProcessHit> {
    // Filter strip clear-× takes precedence over the strip itself.
    if let Some(clear) = layout.clear_btn {
        if inside(clear, phys_x, phys_y) {
            return Some(ProcessHit::ClearFilter);
        }
    }
    // Header columns
    if inside(layout.cpu_header_rect, phys_x, phys_y) {
        return Some(ProcessHit::SortByCpu);
    }
    if inside(layout.mem_header_rect, phys_x, phys_y) {
        return Some(ProcessHit::SortByMem);
    }
    // Rows + kill buttons
    let rows = filtered_rows(sysmon);
    for (i, p) in rows.iter().take(layout.visible_rows).enumerate() {
        let row_y = layout.rows_top + layout.row_h * i as f32;
        if phys_y < row_y || phys_y > row_y + layout.row_h {
            continue;
        }
        if phys_x < layout.inner_x || phys_x > layout.inner_x + layout.inner_w {
            continue;
        }
        let kill_y = row_y + (layout.row_h - layout.kill_w) / 2.0;
        let on_kill = phys_x >= layout.kill_x
            && phys_x <= layout.kill_x + layout.kill_w
            && phys_y >= kill_y
            && phys_y <= kill_y + layout.kill_w;
        if on_kill && sysmon.selected_pid == Some(p.pid) {
            return Some(ProcessHit::Kill(p.pid));
        }
        return Some(ProcessHit::Select(p.pid));
    }
    None
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

#[allow(clippy::too_many_arguments)]
fn draw_filter_strip(
    painter: &mut Painter,
    text: &mut TextRenderer,
    sysmon: &SysMon,
    rect: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let bg = Color::rgba(1.0, 1.0, 1.0, 0.06 * alpha);
    painter.rect_filled(rect, rect.h * 0.30, bg);
    let pad = 12.0 * scale;
    let icon_size = (rect.h * 0.55).max(12.0);
    let icon_cx = rect.x + pad + icon_size * 0.5;
    let icon_cy = rect.y + rect.h * 0.5;
    let icon_color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.55);

    // Tiny magnifier glyph: circle + diagonal handle, painted vectorially.
    painter.circle_stroke(
        icon_cx,
        icon_cy - icon_size * 0.05,
        icon_size * 0.30,
        1.6 * scale,
        icon_color,
    );
    painter.line(
        icon_cx + icon_size * 0.20,
        icon_cy + icon_size * 0.18,
        icon_cx + icon_size * 0.40,
        icon_cy + icon_size * 0.40,
        1.8 * scale,
        icon_color,
    );

    let text_x = icon_cx + icon_size * 0.65;
    let font = (rect.h * 0.55).max(14.0);
    let q = sysmon.filter.query();
    if q.is_empty() {
        let placeholder = "Filter processes…";
        let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.40);
        let w = text.measure_width(placeholder, font);
        text.queue(
            placeholder, font, text_x, rect.y + (rect.h - font) / 2.0,
            color, w, surface_w, surface_h,
        );
    } else {
        let color = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.95);
        let w = text.measure_width(q, font);
        text.queue(
            q, font, text_x, rect.y + (rect.h - font) / 2.0,
            color, w, surface_w, surface_h,
        );

        // Trailing × clear button.
        let btn_w = rect.h - 8.0 * scale;
        let bx = rect.x + rect.w - btn_w - 6.0 * scale;
        let by = rect.y + (rect.h - btn_w) / 2.0;
        let body = Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha);
        painter.rect_filled(Rect::new(bx, by, btn_w, btn_w), btn_w * 0.30, body);
        let pad_in = btn_w * 0.30;
        let line = Color::rgba(1.0, 1.0, 1.0, 0.85 * alpha);
        let lw = (btn_w * 0.10).max(1.4 * scale);
        painter.line(
            bx + pad_in, by + pad_in,
            bx + btn_w - pad_in, by + btn_w - pad_in,
            lw, line,
        );
        painter.line(
            bx + btn_w - pad_in, by + pad_in,
            bx + pad_in, by + btn_w - pad_in,
            lw, line,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
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
    // No truncation — push a clip rect spanning the full name cell so
    // long names just visually clip at the CPU column instead of being
    // shortened with an ellipsis (the empty space to the right was
    // a usability gripe). The clip is popped after the queue so the
    // numeric columns aren't affected.
    let name_clip_w = (cpu_x - x - 4.0 * scale).max(0.0);
    let name_clip = Rect::new(x, y, name_clip_w, h);
    text.push_clip([name_clip.x, name_clip.y, name_clip.w, name_clip.h]);
    let full_w = text.measure_width(&p.comm, font);
    text.queue(&p.comm, font, x, baseline, row_color, full_w + 8.0 * scale, surface_w, surface_h);
    text.pop_clip();

    let cpu_str = format!("{:.1}%", p.cpu_load * 100.0);
    queue_right(text, &cpu_str, font, cpu_x + cpu_w, baseline, row_color, surface_w, surface_h);

    let mem_str = super::proc::format_bytes(p.rss_bytes);
    queue_right(text, &mem_str, font, mem_x + mem_w, baseline, row_color, surface_w, surface_h);

    // Kill button only appears on the highlighted (selected) row so
    // the list reads clean while idle.
    if selected {
        let btn_y = y + (h - kill_w) / 2.0;
        let btn_rect = Rect::new(kill_x, btn_y, kill_w, kill_w);
        let body = Color::rgba(1.0, 0.30, 0.36, 0.95 * alpha);
        painter.rect_filled(btn_rect, kill_w * 0.25, body);
        let ring = Color::rgba(1.0, 1.0, 1.0, 0.65 * alpha);
        painter.rect_stroke(btn_rect, kill_w * 0.25, 1.5 * scale, ring);
        let icon_pad = kill_w * 0.30;
        let line_w = (kill_w * 0.14).max(1.8 * scale);
        let white = Color::rgba(1.0, 1.0, 1.0, alpha);
        painter.line(
            kill_x + icon_pad, btn_y + icon_pad,
            kill_x + kill_w - icon_pad, btn_y + kill_w - icon_pad,
            line_w, white,
        );
        painter.line(
            kill_x + kill_w - icon_pad, btn_y + icon_pad,
            kill_x + icon_pad, btn_y + kill_w - icon_pad,
            line_w, white,
        );
    }
}

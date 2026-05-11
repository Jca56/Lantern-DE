//! Inline tile for the system monitor: two side-by-side mini
//! sparklines (CPU + RAM) with the current % overlaid as a small label.
//! Designed to be glanceable at one second of staring.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::sysmon::SysMon;
use crate::controls::tile::TileLayout;
use crate::render::IconRequest;

/// Logical width reserved for the sysmon tile in the controls row.
pub const TILE_WIDTH: f32 = 150.0;

/// Logical width of one sparkline column (the other half mirrors).
const SPARK_W: f32 = 66.0;
/// Vertical fraction of the row that a sparkline occupies.
const SPARK_H_FRAC: f32 = 0.62;
const SPARK_GAP: f32 = 6.0;

const LABEL_FONT: f32 = 14.0;
const LABEL_GAP: f32 = 2.0;

const CPU_COLOR_RGB: (u8, u8, u8) = (0xff, 0x9a, 0x3c); // warm orange
const MEM_COLOR_RGB: (u8, u8, u8) = (0x55, 0xc6, 0xff); // cool cyan

pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    _icons: &mut Vec<IconRequest>,
    sysmon: &SysMon,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let spark_w = SPARK_W * scale;
    let spark_h = layout.h * SPARK_H_FRAC;
    let gap = SPARK_GAP * scale;
    let label_font = LABEL_FONT * scale;
    let label_gap = LABEL_GAP * scale;

    // Center the two sparklines + gap horizontally inside the tile.
    let group_w = spark_w * 2.0 + gap;
    let start_x = layout.x + (layout.w - group_w) / 2.0;
    // Anchor the sparklines to the bottom of the row so the labels can
    // sit *above* them without overflowing the tile bounds.
    let spark_top = layout.y + layout.h - spark_h;

    // CPU spark (left)
    let cpu_color = Color::from_rgb8(CPU_COLOR_RGB.0, CPU_COLOR_RGB.1, CPU_COLOR_RGB.2)
        .with_alpha(alpha);
    draw_sparkline(
        painter,
        Rect::new(start_x, spark_top, spark_w, spark_h),
        sysmon.cpu_history.samples(),
        100.0,
        cpu_color,
        alpha,
        scale,
    );

    // RAM spark (right)
    let mem_color = Color::from_rgb8(MEM_COLOR_RGB.0, MEM_COLOR_RGB.1, MEM_COLOR_RGB.2)
        .with_alpha(alpha);
    let mem_x = start_x + spark_w + gap;
    draw_sparkline(
        painter,
        Rect::new(mem_x, spark_top, spark_w, spark_h),
        sysmon.mem_history.samples(),
        100.0,
        mem_color,
        alpha,
        scale,
    );

    // Labels above each sparkline. Tight, glanceable. We round to int %
    // because half-percent precision is noise at this size.
    let cpu_label = format!("CPU {}%", sysmon.last_cpu_pct.round() as i32);
    let mem_label = format!("RAM {}%", (sysmon.mem.used_fraction() * 100.0).round() as i32);
    let label_y = spark_top - label_font - label_gap;
    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.85);

    let cpu_w = text.measure_width(&cpu_label, label_font);
    text.queue(
        &cpu_label,
        label_font,
        start_x + (spark_w - cpu_w) / 2.0,
        label_y.max(layout.y),
        white,
        cpu_w,
        surface_w,
        surface_h,
    );

    let mem_w = text.measure_width(&mem_label, label_font);
    text.queue(
        &mem_label,
        label_font,
        mem_x + (spark_w - mem_w) / 2.0,
        label_y.max(layout.y),
        white,
        mem_w,
        surface_w,
        surface_h,
    );
}

/// Draw one sparkline inside `rect`. Samples are mapped: the newest
/// sample is at the right edge, oldest at the left, vertical position
/// derived from `value / scale_max`.
fn draw_sparkline(
    painter: &mut Painter,
    rect: Rect,
    samples: &[f32],
    scale_max: f32,
    color: Color,
    alpha: f32,
    scale: f32,
) {
    // Background: a faint pill so the sparkline reads even when there's
    // no signal yet (zero-history case looks intentional, not broken).
    let bg = Color::rgba(1.0, 1.0, 1.0, 0.06 * alpha);
    painter.rect_filled(rect, 4.0 * scale, bg);

    if samples.len() < 2 {
        return;
    }

    // Map each sample to a pixel coordinate. Samples might be a partial
    // ring (just opened, only a few points) — pin them to the right edge
    // so the line "grows in" from the left as more arrive.
    let n = samples.len();
    let denom = scale_max.max(0.001);
    let pad_y = 2.0 * scale;
    let usable_h = (rect.h - pad_y * 2.0).max(1.0);
    // Step measured against the full ring capacity so the curve doesn't
    // squish horizontally as it fills up. We only know the cap from the
    // call site, but mapping over `n - 1` is fine — points sit closer
    // initially, then space out to fill once the buffer is full.
    let step = if n > 1 { rect.w / (n - 1) as f32 } else { 0.0 };
    let map_point = |i: usize, v: f32| -> (f32, f32) {
        let frac = (v / denom).clamp(0.0, 1.0);
        let x = rect.x + step * i as f32;
        let y = rect.y + pad_y + (1.0 - frac) * usable_h;
        (x, y)
    };

    let pts: Vec<(f32, f32)> = samples
        .iter()
        .enumerate()
        .map(|(i, v)| map_point(i, *v))
        .collect();

    // Filled area under the curve, ~25% alpha of the line color.
    let mut area = pts.clone();
    area.push((rect.x + rect.w, rect.y + rect.h - pad_y));
    area.push((rect.x, rect.y + rect.h - pad_y));
    let fill_color = Color::rgba(color.r, color.g, color.b, color.a * 0.25);
    painter.polygon(&area, fill_color);

    // The line on top.
    painter.polyline_round(&pts, 1.5 * scale, color);
}

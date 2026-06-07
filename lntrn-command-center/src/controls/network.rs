//! Network throughput tile: two mini sparklines (download + upload) with
//! the current per-second rate overlaid as a small label. Reads the
//! RX/TX history the SysMon worker already collects — no extra polling.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::sysmon::SysMon;
use crate::controls::tile::TileLayout;
use crate::render::IconRequest;

/// Logical width reserved for the network tile in the controls row.
pub const TILE_WIDTH: f32 = 150.0;

/// Logical width of one sparkline column (download + upload).
const SPARK_W: f32 = 64.0;
const SPARK_H_FRAC: f32 = 0.62;
const SPARK_GAP: f32 = 8.0;

const LABEL_FONT: f32 = 15.0;
const LABEL_GAP: f32 = 2.0;

/// Download = green, upload = violet (distinct from sysmon's orange/cyan).
const RX_COLOR_RGB: (u8, u8, u8) = (0x4c, 0xd1, 0x64);
const TX_COLOR_RGB: (u8, u8, u8) = (0xc9, 0x6b, 0xff);

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

    // Share a single autoscale max across both directions so the two
    // sparklines are directly comparable in height.
    let scale_max = sysmon
        .net_rx_history
        .max_with_floor(1.0)
        .max(sysmon.net_tx_history.max_with_floor(1.0));

    let group_w = spark_w * 2.0 + gap;
    let start_x = layout.x + (layout.w - group_w) / 2.0;
    let spark_top = layout.y + layout.h - spark_h;

    let rx_color = Color::from_rgb8(RX_COLOR_RGB.0, RX_COLOR_RGB.1, RX_COLOR_RGB.2).with_alpha(alpha);
    let tx_color = Color::from_rgb8(TX_COLOR_RGB.0, TX_COLOR_RGB.1, TX_COLOR_RGB.2).with_alpha(alpha);

    draw_sparkline(
        painter,
        Rect::new(start_x, spark_top, spark_w, spark_h),
        sysmon.net_rx_history.samples(),
        scale_max,
        rx_color,
        alpha,
        scale,
    );
    let tx_x = start_x + spark_w + gap;
    draw_sparkline(
        painter,
        Rect::new(tx_x, spark_top, spark_w, spark_h),
        sysmon.net_tx_history.samples(),
        scale_max,
        tx_color,
        alpha,
        scale,
    );

    // Labels above each sparkline: ↓ download, ↑ upload.
    let rx_label = format!("↓ {}", compact_rate(sysmon.last_net_rx_bps));
    let tx_label = format!("↑ {}", compact_rate(sysmon.last_net_tx_bps));
    let label_y = (spark_top - label_font - label_gap).max(layout.y);
    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.85);

    let rx_w = text.measure_width(&rx_label, label_font);
    text.queue(
        &rx_label,
        label_font,
        start_x,
        label_y,
        white,
        rx_w + 8.0 * scale,
        surface_w,
        surface_h,
    );
    let tx_w = text.measure_width(&tx_label, label_font);
    text.queue(
        &tx_label,
        label_font,
        tx_x,
        label_y,
        white,
        tx_w + 8.0 * scale,
        surface_w,
        surface_h,
    );
}

/// Compact per-second rate that fits under a 64px sparkline, e.g.
/// "1.2M", "340K", "0". No "/s" suffix — the ↓/↑ arrows carry the
/// "rate" meaning and space is tight.
fn compact_rate(bps: f32) -> String {
    const KB: f32 = 1024.0;
    const MB: f32 = 1024.0 * 1024.0;
    const GB: f32 = 1024.0 * 1024.0 * 1024.0;
    if bps >= GB {
        format!("{:.1}G", bps / GB)
    } else if bps >= MB {
        format!("{:.1}M", bps / MB)
    } else if bps >= KB {
        format!("{:.0}K", bps / KB)
    } else {
        "0".to_string()
    }
}

/// Mini autoscaled sparkline. Newest sample at the right edge.
fn draw_sparkline(
    painter: &mut Painter,
    rect: Rect,
    samples: &[f32],
    scale_max: f32,
    color: Color,
    alpha: f32,
    scale: f32,
) {
    let bg = Color::rgba(1.0, 1.0, 1.0, 0.06 * alpha);
    painter.rect_filled(rect, 4.0 * scale, bg);

    if samples.len() < 2 {
        return;
    }

    let n = samples.len();
    let denom = scale_max.max(0.001);
    let pad_y = 2.0 * scale;
    let usable_h = (rect.h - pad_y * 2.0).max(1.0);
    let step = rect.w / (n - 1) as f32;
    let map_point = |i: usize, v: f32| -> (f32, f32) {
        let frac = (v / denom).clamp(0.0, 1.0);
        let x = rect.x + step * i as f32;
        let y = rect.y + pad_y + (1.0 - frac) * usable_h;
        (x, y)
    };

    let pts: Vec<(f32, f32)> = samples.iter().enumerate().map(|(i, v)| map_point(i, *v)).collect();

    let mut area = pts.clone();
    area.push((rect.x + rect.w, rect.y + rect.h - pad_y));
    area.push((rect.x, rect.y + rect.h - pad_y));
    let fill_color = Color::rgba(color.r, color.g, color.b, color.a * 0.25);
    painter.polygon(&area, fill_color);

    painter.polyline_round(&pts, 1.5 * scale, color);
}

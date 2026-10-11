//! Battery readouts for Bluetooth devices: the gauge on a connected
//! device's row, and the figure the toolbar tile shows beside its rune.
//!
//! BlueZ publishes `org.bluez.Battery1` for a device only while it's
//! connected, and only if the device reports a level at all (headsets
//! and controllers mostly do; phones and computers mostly don't), so
//! every readout here is optional. Colours follow the laptop battery
//! tile's zones so a charge level means the same thing wherever it shows.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::{Bluetooth, Device};
use crate::controls::battery::{fill_rgb, PILL_TRACK_RGB, YELLOW_BELOW};

/// Row gauge: the pill's height relative to the status badge's font, and
/// its width relative to that height.
const PILL_H_EM: f32 = 1.05;
const PILL_ASPECT: f32 = 2.8;
/// Logical px between the percentage and the pill.
const PILL_LABEL_GAP: f32 = 8.0;
/// Least of the pill a nearly flat battery still fills, as a share of the
/// pill's height, so the last few percent show as a sliver, not nothing.
const PILL_MIN_FILL: f32 = 0.35;

impl Device {
    /// Charge to show for this device: the level it last reported, while
    /// it's connected. One left over from an earlier connection is stale.
    pub(super) fn live_battery(&self) -> Option<u8> {
        self.battery_percent
            .filter(|_| self.connected)
            .map(|pct| pct.min(100))
    }
}

impl Bluetooth {
    /// Lowest charge among the connected devices that report one: the
    /// device that needs charging first. `None` when none of them do.
    pub fn lowest_connected_battery(&self) -> Option<u8> {
        self.devices.iter().filter_map(Device::live_battery).min()
    }
}

/// Colour for a charge figure standing on its own, with no gauge beside
/// it: white while there's plenty, then the gauge's yellow and red as it
/// runs down.
pub(super) fn figure_color(percent: u8, alpha: f32) -> Color {
    let level = u32::from(percent);
    if level >= YELLOW_BELOW {
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.92 * alpha)
    } else {
        let (r, g, b) = fill_rgb(level);
        Color::from_rgb8(r, g, b).with_alpha(alpha)
    }
}

/// Draw a device row's battery gauge, "58% [pill]", with its right edge
/// at `right`, centred in the row `row_y .. row_y + row_h`. Returns the x
/// of its left edge, so the caller can keep the device name clear of it.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_row_gauge(
    painter: &mut Painter,
    text: &mut TextRenderer,
    percent: u8,
    right: f32,
    row_y: f32,
    row_h: f32,
    font: f32,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) -> f32 {
    let pill_h = font * PILL_H_EM;
    let pill_w = pill_h * PILL_ASPECT;
    let pill = Rect::new(
        right - pill_w,
        row_y + (row_h - pill_h) / 2.0,
        pill_w,
        pill_h,
    );
    draw_pill(painter, pill, percent, alpha);

    let label = format!("{percent}%");
    let label_w = text.measure_width(&label, font);
    let label_x = pill.x - PILL_LABEL_GAP * scale - label_w;
    // Slack past the measured width: queue() quantizes max_width down,
    // and an exact fit wraps the last glyph onto a clipped second line.
    text.queue(
        &label,
        font,
        label_x,
        row_y + (row_h - font) / 2.0,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.92 * alpha),
        label_w + 8.0 * scale,
        surface_w,
        surface_h,
    );
    label_x
}

/// The gauge itself: a grey track with the charge laid over it in the
/// zone's colour.
fn draw_pill(painter: &mut Painter, pill: Rect, percent: u8, alpha: f32) {
    let radius = pill.h * 0.5;
    let (tr, tg, tb) = PILL_TRACK_RGB;
    painter.rect_filled(pill, radius, Color::from_rgb8(tr, tg, tb).with_alpha(alpha));
    if percent == 0 {
        return;
    }

    let level = u32::from(percent);
    let (r, g, b) = fill_rgb(level);
    let fill = Color::from_rgb8(r, g, b).with_alpha(alpha);
    if level >= 100 {
        painter.rect_filled(pill, radius, fill);
        return;
    }

    // The fill is the whole pill cut off at the charge level rather than
    // a shorter pill of its own: a rounded fill can't get narrower than
    // the pill is tall, which at this size would draw every level under
    // a third the same.
    let fill_w = (pill.w * level as f32 / 100.0).max(pill.h * PILL_MIN_FILL);
    // Only the clip's right edge is meant to cut. The painter truncates a
    // clip to whole pixels, so the other three sides sit a pixel outside
    // the pill (or the track shows through along the fill's bottom) and
    // the right edge lands on a pixel boundary itself.
    let left = pill.x.floor() - 1.0;
    let top = pill.y.floor() - 1.0;
    let right = (pill.x + fill_w).round();
    painter.push_clip(Rect::new(left, top, right - left, pill.h.ceil() + 3.0));
    painter.rect_filled(pill, radius, fill);
    painter.pop_clip();
}
